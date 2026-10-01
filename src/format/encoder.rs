//! Byte-level writer for the v1 wire format.
//!
//! The walker (`graph::traversal`) assigns record ids in breadth-first order and
//! emits each record straight into `RecordWriter::rec` in id order, so no
//! intermediate `Record` tree is ever built. The string table and type table are
//! accumulated in their own buffers and everything is stitched together once in
//! [`Encoded::into_pybytes`], copying directly into the final `bytes` object.
//! Writer buffers are recycled per thread to avoid regrowing them on every call.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::hash::Hasher;

use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use rustc_hash::{FxHashMap, FxHasher};

use super::*;

/// Sentinel for "no entry" in u32 index chains / lookup tables.
pub const NO_ID: u32 = u32::MAX;

/// Payloads at least this large are not copied into the intermediate buffers;
/// they are referenced in place (see [`Ext`]) and copied once into the output.
pub const EXT_THRESHOLD: usize = 4096;

/// Release the GIL for the final copy only when the output is this large.
const GIL_RELEASE_THRESHOLD: usize = 8 << 20;

/// A borrowed byte range that is spliced into a buffer at `pos` during assembly.
#[derive(Clone, Copy)]
struct Ext {
    pos: usize,
    ptr: *const u8,
    len: usize,
}

struct StrEntry {
    /// Offset of the bytes in `strings`; `usize::MAX` for external entries,
    /// which never match an intern lookup.
    off: usize,
    len: u32,
    /// Collision chain: next string index with the same hash.
    next: u32,
    /// Record id of the String record for this entry (walker-managed).
    rec: u32,
}

struct TypeEntry {
    name: String,
    fields: Vec<String>,
    name_idx: u32,
    schema_version: u32,
    field_idxs: Vec<u32>,
}

#[derive(Default)]
pub struct RecordWriter {
    // String table: `strings` holds the serialized entries (u32 len + bytes).
    strings: Vec<u8>,
    /// u64 so that exceeding u32::MAX is reported at assembly instead of wrapping.
    str_count: u64,
    /// Per string-table index bookkeeping.
    str_entries: Vec<StrEntry>,
    /// hash of contents -> most recently interned string index with that hash.
    str_heads: FxHashMap<u64, u32>,
    str_ext: Vec<Ext>,

    // Record section.
    rec: Vec<u8>,
    rec_ext: Vec<Ext>,
    rec_count: u64,

    types: Vec<TypeEntry>,
}

#[inline]
fn hash_bytes(s: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(s);
    h.finish()
}

/// Max bytes of buffer capacity a thread keeps between calls (per pool).
pub const RETAIN_MAX: usize = 4 << 20;

thread_local! {
    /// Cleared writer buffers from the previous call on this thread. Reusing them
    /// avoids regrowing (and, on Windows, re-faulting large VirtualAlloc'd
    /// blocks) on every call. Re-entrant calls simply find the pool empty.
    static WRITER_POOL: RefCell<Option<RecordWriter>> = const { RefCell::new(None) };
}

impl RecordWriter {
    pub fn new() -> Self {
        if let Some(w) = WRITER_POOL.with(|p| p.borrow_mut().take()) {
            return w;
        }
        Self {
            strings: Vec::with_capacity(256),
            rec: Vec::with_capacity(256),
            ..Default::default()
        }
    }

    fn retained_bytes(&self) -> usize {
        self.strings.capacity()
            + self.rec.capacity()
            + self.str_entries.capacity() * std::mem::size_of::<StrEntry>()
            + self.str_heads.capacity() * (std::mem::size_of::<(u64, u32)>() + 1)
            + (self.str_ext.capacity() + self.rec_ext.capacity()) * std::mem::size_of::<Ext>()
    }

    /// Clear and return the buffers to this thread's pool (if not too large).
    fn recycle(mut self) {
        if self.retained_bytes() > RETAIN_MAX {
            return;
        }
        self.strings.clear();
        self.str_count = 0;
        self.str_entries.clear();
        self.str_heads.clear();
        self.str_ext.clear();
        self.rec.clear();
        self.rec_ext.clear();
        self.rec_count = 0;
        self.types.clear();
        // Ignore failure during thread teardown.
        let _ = WRITER_POOL.try_with(|p| *p.borrow_mut() = Some(self));
    }

    // ----- ids -------------------------------------------------------------

    /// Reserve the next record id. Records MUST be written in id order.
    #[inline]
    pub fn alloc_id(&mut self) -> u32 {
        let id = self.rec_count as u32;
        self.rec_count += 1;
        id
    }

    // ----- string table ----------------------------------------------------

    /// Caller guarantees `s.len() <= u32::MAX` (only short strings are hashed).
    #[inline]
    fn push_str_entry(&mut self, s: &[u8], next: u32) -> u32 {
        let idx = self.str_count as u32;
        self.str_count += 1;
        self.strings.reserve(4 + s.len());
        write_u32(&mut self.strings, s.len() as u32);
        let off = self.strings.len();
        self.strings.extend_from_slice(s);
        self.str_entries.push(StrEntry {
            off,
            len: s.len() as u32,
            next,
            rec: NO_ID,
        });
        idx
    }

    /// Intern a (valid UTF-8) string by value. Equal contents share one index.
    #[inline]
    pub fn intern_str(&mut self, s: &[u8]) -> u32 {
        let h = hash_bytes(s);
        match self.str_heads.entry(h) {
            Entry::Vacant(e) => {
                e.insert(self.str_count as u32);
                self.push_str_entry(s, NO_ID)
            }
            Entry::Occupied(mut e) => {
                let head = *e.get();
                let mut cur = head;
                while cur != NO_ID {
                    let ent = &self.str_entries[cur as usize];
                    if ent.off != usize::MAX
                        && ent.len as usize == s.len()
                        && &self.strings[ent.off..ent.off + s.len()] == s
                    {
                        return cur;
                    }
                    cur = ent.next;
                }
                *e.get_mut() = self.str_count as u32;
                self.push_str_entry(s, head)
            }
        }
    }

    /// Record id associated with string-table entry `idx` (`NO_ID` if none yet).
    #[inline]
    pub fn str_rec(&mut self, idx: u32) -> &mut u32 {
        &mut self.str_entries[idx as usize].rec
    }

    /// Append a string entry without hashing and without copying its bytes now.
    ///
    /// # Safety
    /// `ptr..ptr+len` must be valid, immutable UTF-8 for as long as this writer
    /// is alive (the caller pins the owning Python `str`).
    #[inline]
    pub unsafe fn push_str_ext(&mut self, ptr: *const u8, len: usize) -> u32 {
        let idx = self.str_count as u32;
        self.str_count += 1;
        write_u32(&mut self.strings, len as u32);
        self.str_ext.push(Ext {
            pos: self.strings.len(),
            ptr,
            len,
        });
        self.str_entries.push(StrEntry {
            off: usize::MAX,
            len: 0,
            next: NO_ID,
            rec: NO_ID,
        });
        idx
    }

    // ----- type table ------------------------------------------------------

    /// Register a dataclass type and return its type id, or `None` if the
    /// u16 id space is exhausted.
    ///
    /// Entries are shared only between classes with the same name AND the same
    /// field list (mirroring `TypeRegistry::register`), so distinct classes that
    /// happen to share a `__name__` each get their own entry.
    pub fn register_type(
        &mut self,
        name: &str,
        fields: &[String],
        schema_version: u32,
    ) -> Option<u16> {
        if let Some(pos) = self
            .types
            .iter()
            .position(|t| t.name == name && t.fields.as_slice() == fields)
        {
            return Some(pos as u16);
        }
        let id = u16::try_from(self.types.len()).ok()?;
        let name_idx = self.intern_str(name.as_bytes());
        let field_idxs = fields
            .iter()
            .map(|f| self.intern_str(f.as_bytes()))
            .collect();
        self.types.push(TypeEntry {
            name: name.to_string(),
            fields: fields.to_vec(),
            name_idx,
            schema_version,
            field_idxs,
        });
        Some(id)
    }

    // ----- record payload primitives ----------------------------------------

    #[inline]
    pub fn reserve(&mut self, n: usize) {
        self.rec.reserve(n);
    }

    #[inline]
    pub fn tag(&mut self, tag: Tag) {
        self.rec.push(tag as u8);
    }

    #[inline]
    pub fn u32(&mut self, v: u32) {
        write_u32(&mut self.rec, v);
    }

    #[inline]
    pub fn u16(&mut self, v: u16) {
        write_u16(&mut self.rec, v);
    }

    #[inline]
    pub fn raw(&mut self, b: &[u8]) {
        self.rec.extend_from_slice(b);
    }

    /// Current record-buffer position (for back-patching counts).
    #[inline]
    pub fn pos(&self) -> usize {
        self.rec.len()
    }

    #[inline]
    pub fn patch_u32(&mut self, pos: usize, v: u32) {
        self.rec[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn int(&mut self, v: i64) {
        self.rec.reserve(11);
        self.rec.push(Tag::Int as u8);
        write_i64_zigzag(&mut self.rec, v);
    }

    #[inline]
    pub fn float(&mut self, v: f64) {
        self.rec.reserve(9);
        self.rec.push(Tag::Float as u8);
        self.rec.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn string(&mut self, idx: u32) {
        self.rec.reserve(5);
        self.rec.push(Tag::String as u8);
        self.u32(idx);
    }

    /// Tag + u32 length + bytes, copying the payload.
    #[inline]
    pub fn blob(&mut self, tag: Tag, b: &[u8]) {
        self.rec.reserve(5 + b.len());
        self.rec.push(tag as u8);
        self.u32(b.len() as u32);
        self.rec.extend_from_slice(b);
    }

    /// Tag + u32 length, with the payload spliced in at assembly time.
    ///
    /// # Safety
    /// `ptr..ptr+len` must stay valid and unmodified for the writer's lifetime
    /// (the caller pins an immutable Python object owning the buffer).
    #[inline]
    pub unsafe fn blob_ext(&mut self, tag: Tag, ptr: *const u8, len: usize) {
        self.rec.reserve(5);
        self.rec.push(tag as u8);
        self.u32(len as u32);
        self.rec_ext.push(Ext {
            pos: self.rec.len(),
            ptr,
            len,
        });
    }

    // ----- assembly ----------------------------------------------------------

    fn type_table_len(&self) -> usize {
        4 + self
            .types
            .iter()
            .map(|t| 2 + 4 + 4 + 2 + 4 * t.field_idxs.len())
            .sum::<usize>()
    }

    pub fn total_len(&self) -> usize {
        let ext = |v: &[Ext]| v.iter().map(|e| e.len).sum::<usize>();
        11 + 4
            + self.strings.len()
            + ext(&self.str_ext)
            + self.type_table_len()
            + 4
            + self.rec.len()
            + ext(&self.rec_ext)
    }

    /// Write the complete payload into `out` (which must be `total_len()` long).
    fn write_into(&self, out: &mut [u8]) {
        let mut w = SliceWriter { out, pos: 0 };
        let mut header = Vec::with_capacity(11);
        write_header(&mut header, 0);
        w.put(&header);
        w.put(&(self.str_count as u32).to_le_bytes());
        w.put_with_ext(&self.strings, &self.str_ext);
        w.put(&(self.types.len() as u32).to_le_bytes());
        for (id, t) in self.types.iter().enumerate() {
            w.put(&(id as u16).to_le_bytes());
            w.put(&t.name_idx.to_le_bytes());
            w.put(&t.schema_version.to_le_bytes());
            w.put(&(t.field_idxs.len() as u16).to_le_bytes());
            for f in &t.field_idxs {
                w.put(&f.to_le_bytes());
            }
        }
        w.put(&(self.rec_count as u32).to_le_bytes());
        w.put_with_ext(&self.rec, &self.rec_ext);
        debug_assert_eq!(w.pos, w.out.len());
    }

    /// Assemble the final payload directly into a new Python `bytes` object.
    /// Callers must keep the owners of all [`Ext`] buffers alive (see [`Encoded`]).
    fn to_pybytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        if self.rec_count > u32::MAX as u64 || self.str_count > u32::MAX as u64 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "object graph exceeds the format's limits (2**32-1 records or strings)",
            ));
        }
        let total = self.total_len();
        // SAFETY: PyBytes_FromStringAndSize(NULL, n) returns a new, unshared bytes
        // object with an uninitialised n-byte buffer (or NULL + exception). We fill
        // every byte via `write_into` before the object is exposed to Python. This
        // avoids the zero-fill `PyBytes::new_with` would do.
        unsafe {
            let raw = ffi::PyBytes_FromStringAndSize(std::ptr::null(), total as ffi::Py_ssize_t);
            let bytes = Bound::from_owned_ptr_or_err(py, raw)?.downcast_into_unchecked::<PyBytes>();
            let buf = ffi::PyBytes_AsString(raw) as *mut u8;
            let out = std::slice::from_raw_parts_mut(buf, total);
            if total >= GIL_RELEASE_THRESHOLD {
                let job = SendWrapper((self, out));
                py.allow_threads(move || {
                    // SAFETY (Send): see `SendWrapper`; `self` is only read.
                    // `into_inner()` makes the closure capture the whole wrapper
                    // (edition-2021 closures would otherwise capture fields).
                    let (writer, out) = job.into_inner();
                    writer.write_into(out);
                });
            } else {
                self.write_into(out);
            }
            Ok(bytes)
        }
    }
}

/// A finished serialization: the record writer plus strong references to every
/// Python object whose (immutable) buffer the writer borrows via [`Ext`].
pub struct Encoded<'py> {
    writer: RecordWriter,
    pins: Vec<Bound<'py, PyAny>>,
}

impl<'py> Encoded<'py> {
    pub fn new(writer: RecordWriter, pins: Vec<Bound<'py, PyAny>>) -> Self {
        Self { writer, pins }
    }

    /// Assemble the payload into a new Python `bytes` object.
    pub fn into_pybytes(self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let Encoded { writer, pins } = self;
        let result = writer.to_pybytes(py);
        writer.recycle();
        // The writer's borrowed buffers are owned by `pins`; release them last.
        drop(pins);
        result
    }
}

/// Wrapper used to move the writer (which holds raw pointers into pinned,
/// immutable Python buffers) and the output slice into `allow_threads`.
struct SendWrapper<T>(T);
// SAFETY: the raw pointers only reference immutable `bytes`/`str` buffers kept
// alive by strong references owned by the (blocked) calling frame, and the output
// buffer belongs to a bytes object no other code can see yet. The closure only
// reads the former and writes the latter, so no data race is possible.
unsafe impl<T> Send for SendWrapper<T> {}

impl<T> SendWrapper<T> {
    fn into_inner(self) -> T {
        self.0
    }
}

struct SliceWriter<'a> {
    out: &'a mut [u8],
    pos: usize,
}

impl SliceWriter<'_> {
    #[inline]
    fn put(&mut self, b: &[u8]) {
        self.out[self.pos..self.pos + b.len()].copy_from_slice(b);
        self.pos += b.len();
    }

    fn put_with_ext(&mut self, buf: &[u8], exts: &[Ext]) {
        let mut cur = 0;
        for e in exts {
            self.put(&buf[cur..e.pos]);
            // SAFETY: see `push_str_ext` / `blob_ext` contracts.
            let ext = unsafe { std::slice::from_raw_parts(e.ptr, e.len) };
            self.put(ext);
            cur = e.pos;
        }
        self.put(&buf[cur..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_str_dedup_and_ext_assembly() {
        let mut w = RecordWriter::new();
        let a = w.intern_str(b"hello");
        let b = w.intern_str(b"world");
        assert_eq!(w.intern_str(b"hello"), a);
        assert_ne!(a, b);
        let big = vec![b'x'; EXT_THRESHOLD];
        let c = unsafe { w.push_str_ext(big.as_ptr(), big.len()) };
        assert_eq!(c, 2);

        let root = w.alloc_id();
        assert_eq!(root, 0);
        w.tag(Tag::List);
        w.u32(1);
        let child = w.alloc_id();
        w.u32(child);
        w.string(c);

        let mut out = vec![0u8; w.total_len()];
        w.write_into(&mut out);
        assert_eq!(&out[..4], MAGIC_WRITE);
        let mut off = 11;
        assert_eq!(read_u32(&out, &mut off), Some(3));
        let l = read_u32(&out, &mut off).unwrap() as usize;
        assert_eq!(read_bytes(&out, &mut off, l), Some(&b"hello"[..]));
        let l = read_u32(&out, &mut off).unwrap() as usize;
        assert_eq!(read_bytes(&out, &mut off, l), Some(&b"world"[..]));
        let l = read_u32(&out, &mut off).unwrap() as usize;
        assert_eq!(read_bytes(&out, &mut off, l), Some(&big[..]));
        assert_eq!(read_u32(&out, &mut off), Some(0)); // types
        assert_eq!(read_u32(&out, &mut off), Some(2)); // records
        assert_eq!(read_u8(&out, &mut off), Some(Tag::List as u8));
        assert_eq!(read_u32(&out, &mut off), Some(1));
        assert_eq!(read_u32(&out, &mut off), Some(1));
        assert_eq!(read_u8(&out, &mut off), Some(Tag::String as u8));
        assert_eq!(read_u32(&out, &mut off), Some(2));
        assert_eq!(off, out.len());
    }
}
