//! Object-graph walker for serialization.
//!
//! Records are assigned ids in breadth-first order and written straight into the
//! output buffer in that same order (see `format::encoder::RecordWriter`), so the
//! root is always record 0 and no intermediate record tree is allocated.
//!
//! * Containers refer to children directly by record id. Shared / cyclic objects
//!   simply reuse the id of the first occurrence (no `Reference` records).
//! * Immutable scalars are deduplicated by value: one record each for
//!   None/True/False, and one per distinct int, float (bit pattern) and string.
//! * Everything else is memoised by identity and pinned for the duration of the
//!   call so a memo address can never be reused by a different object.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::ptr::addr_of_mut;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;

use pyo3::exceptions::{PyRecursionError, PyTypeError, PyValueError};
use pyo3::ffi;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::*;

use crate::format::encoder::{Encoded, RecordWriter, EXT_THRESHOLD, NO_ID, RETAIN_MAX};
use crate::format::Tag;

/// Legacy in-memory record representation (still used by the decoder).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Record {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(u32),
    Bytes(Vec<u8>),
    List(Vec<u32>),
    Tuple(Vec<u32>),
    Dict(Vec<(u32, u32)>),
    Set(Vec<u32>),
    FrozenSet(Vec<u32>),
    Dataclass { type_id: u16, fields: Vec<u32> },
    Reference(u32),
}

/// Legacy record-tree container. No longer used by the serializer.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ObjectGraph {
    pub strings: Vec<String>,
    pub records: Vec<Option<Record>>,
    pub string_index: FxHashMap<String, u32>,
}

#[allow(dead_code)]
impl ObjectGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.string_index.get(s) {
            return idx;
        }
        let idx = self.strings.len() as u32;
        self.string_index.insert(s.to_string(), idx);
        self.strings.push(s.to_string());
        idx
    }

    pub fn push_placeholder(&mut self) -> u32 {
        let id = self.records.len() as u32;
        self.records.push(None);
        id
    }

    pub fn set_record(&mut self, id: u32, record: Record) {
        self.records[id as usize] = Some(record);
    }
}

// ---------------------------------------------------------------------------
// Type classification cache (process-wide)
// ---------------------------------------------------------------------------

/// Exactly-matched stdlib types with a dedicated record tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Std {
    DateTime,
    Date,
    Time,
    TimeDelta,
    Timezone,
    ZoneInfo,
    Decimal,
    Uuid,
    Fraction,
    PurePosixPath,
    PureWindowsPath,
    PosixPath,
    WindowsPath,
    OrderedDict,
    DefaultDict,
    Counter,
    Deque,
}

/// (module, attribute, kind). A type can only be one of these if its module
/// is already in `sys.modules`, so detection never imports anything.
const STD_TYPES: [(&str, &str, Std); 17] = [
    ("datetime", "datetime", Std::DateTime),
    ("datetime", "date", Std::Date),
    ("datetime", "time", Std::Time),
    ("datetime", "timedelta", Std::TimeDelta),
    ("datetime", "timezone", Std::Timezone),
    ("zoneinfo", "ZoneInfo", Std::ZoneInfo),
    ("decimal", "Decimal", Std::Decimal),
    ("uuid", "UUID", Std::Uuid),
    ("fractions", "Fraction", Std::Fraction),
    ("pathlib", "PurePosixPath", Std::PurePosixPath),
    ("pathlib", "PureWindowsPath", Std::PureWindowsPath),
    ("pathlib", "PosixPath", Std::PosixPath),
    ("pathlib", "WindowsPath", Std::WindowsPath),
    ("collections", "OrderedDict", Std::OrderedDict),
    ("collections", "defaultdict", Std::DefaultDict),
    ("collections", "Counter", Std::Counter),
    ("collections", "deque", Std::Deque),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassKind {
    Dataclass,
    NamedTuple,
    Enum,
    Std(Std),
}

impl ClassKind {
    /// Type-table kind discriminator (writer-local, never serialized).
    fn table_kind(self) -> u8 {
        match self {
            ClassKind::Dataclass => 0,
            ClassKind::NamedTuple => 1,
            _ => 2,
        }
    }
}

struct ClassInfo {
    /// Strong reference: keeps the type (and thus its cache key address) alive.
    _ty: Py<PyType>,
    kind: ClassKind,
    name: String,
    field_names: Vec<String>,
    /// Interned field names for fast `getattr` (dataclasses only).
    field_objs: Vec<Py<PyString>>,
    schema_version: u32,
}

/// type pointer -> metadata. Bounded; cleared wholesale when full.
static CLASS_CACHE: Mutex<Option<FxHashMap<usize, Arc<ClassInfo>>>> = Mutex::new(None);
const CLASS_CACHE_MAX: usize = 1024;

fn cache_get(key: usize) -> Option<Arc<ClassInfo>> {
    let guard = CLASS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref()?.get(&key).cloned()
}

fn cache_put(key: usize, info: Arc<ClassInfo>) {
    let evicted = {
        let mut guard = CLASS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(FxHashMap::default);
        let evicted = if map.len() >= CLASS_CACHE_MAX {
            Some(std::mem::take(map))
        } else {
            None
        };
        map.insert(key, info);
        evicted
    };
    // Dropped outside the lock: releasing types may run arbitrary Python code.
    drop(evicted);
}

/// `sys.modules.get(name)` (never imports).
fn loaded_module<'py>(py: Python<'py>, name: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let modules = py.import(intern!(py, "sys"))?.getattr(intern!(py, "modules"))?;
    let modules = modules.downcast::<PyDict>()?;
    modules.get_item(name)
}

fn std_kind(py: Python<'_>, ty: &Bound<'_, PyType>) -> PyResult<Option<Std>> {
    let mut last: Option<(&str, Option<Bound<'_, PyAny>>)> = None;
    for (module, attr, kind) in STD_TYPES {
        if last.as_ref().map(|(m, _)| *m) != Some(module) {
            last = Some((module, loaded_module(py, module)?));
        }
        if let Some((_, Some(m))) = &last {
            if let Ok(t) = m.getattr(attr) {
                if t.is(ty) {
                    return Ok(Some(kind));
                }
            }
        }
    }
    Ok(None)
}

fn schema_version_of(py: Python<'_>, ty: &Bound<'_, PyType>) -> u32 {
    ty.getattr(intern!(py, "__pysafe_pickle_version__"))
        .or_else(|_| ty.getattr(intern!(py, "__pygraph_version__")))
        .and_then(|v| v.extract())
        .unwrap_or(0)
}

fn is_enum_class(py: Python<'_>, ty: &Bound<'_, PyType>) -> PyResult<bool> {
    match loaded_module(py, "enum")? {
        Some(m) => ty.is_instance(&m.getattr(intern!(py, "EnumMeta"))?),
        None => Ok(false),
    }
}

/// Field names of a namedtuple class (a strict tuple subclass whose `_fields`
/// is a tuple of str), or `None`.
fn namedtuple_fields(py: Python<'_>, ty: &Bound<'_, PyType>) -> PyResult<Option<Vec<String>>> {
    // SAFETY: both are valid type objects; we hold the GIL.
    let is_tuple_sub = unsafe {
        ffi::PyType_IsSubtype(ty.as_type_ptr(), addr_of_mut!(ffi::PyTuple_Type)) != 0
    };
    if !is_tuple_sub {
        return Ok(None);
    }
    let Ok(fields) = ty.getattr(intern!(py, "_fields")) else {
        return Ok(None);
    };
    let Ok(fields) = fields.downcast::<PyTuple>() else {
        return Ok(None);
    };
    let mut out = Vec::with_capacity(fields.len());
    for f in fields.iter() {
        match f.extract::<String>() {
            Ok(s) => out.push(s),
            Err(_) => return Ok(None),
        }
    }
    Ok(Some(out))
}

/// Returns metadata for `ty`, or `None` if instances are not serializable.
fn class_info(py: Python<'_>, ty: &Bound<'_, PyType>) -> PyResult<Option<Arc<ClassInfo>>> {
    let key = ty.as_ptr() as usize;
    if let Some(info) = cache_get(key) {
        return Ok(Some(info));
    }
    let mut field_names = Vec::new();
    let mut field_objs = Vec::new();
    let mut schema_version = 0;
    let kind = if let Some(s) = std_kind(py, ty)? {
        ClassKind::Std(s)
    } else if is_enum_class(py, ty)? {
        ClassKind::Enum
    } else if let Some(fields) = namedtuple_fields(py, ty)? {
        field_names = fields;
        ClassKind::NamedTuple
    } else if ty.hasattr(intern!(py, "__dataclass_fields__"))? {
        // `dataclasses.fields()` excludes ClassVar / InitVar pseudo-fields.
        let fields = py
            .import(intern!(py, "dataclasses"))?
            .getattr(intern!(py, "fields"))?
            .call1((ty,))?;
        for f in fields.try_iter()? {
            let name: String = f?.getattr(intern!(py, "name"))?.extract()?;
            field_objs.push(PyString::intern(py, &name).unbind());
            field_names.push(name);
        }
        schema_version = schema_version_of(py, ty);
        ClassKind::Dataclass
    } else {
        return Ok(None);
    };
    let info = Arc::new(ClassInfo {
        _ty: ty.clone().unbind(),
        kind,
        name: ty.name()?.to_string(),
        field_names,
        field_objs,
        schema_version,
    });
    cache_put(key, info.clone());
    Ok(Some(info))
}

#[cold]
fn unsupported(what: impl std::fmt::Display) -> PyErr {
    PyTypeError::new_err(format!("Unsupported type: {}", what))
}

/// `str` -> UTF-8 bytes, passing lone surrogates through.
fn utf8_pass(s: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    let py = s.py();
    let b = s.call_method1(
        intern!(py, "encode"),
        (intern!(py, "utf-8"), intern!(py, "surrogatepass")),
    )?;
    Ok(b.downcast_into::<PyBytes>()?.as_bytes().to_vec())
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

/// Strings at least this long are memoised by identity and copied lazily
/// instead of being hashed into the string table.
const LARGE_STR: usize = EXT_THRESHOLD;

/// Lossy, direct-mapped value -> record id cache for ints/floats.
///
/// Deduplicates repeated numbers (common in real data) at the cost of one
/// array probe, without the growth/rehash cost of a full hash map on inputs where
/// every number is distinct. Misses just emit another (identical) record.
#[derive(Default)]
struct ScalarCache {
    slots: Vec<(u64, u32)>,
}

const SCALAR_CACHE_BITS: u32 = 6;

impl ScalarCache {
    #[inline]
    fn slot(&mut self, key: u64) -> &mut (u64, u32) {
        if self.slots.is_empty() {
            self.slots = vec![(0, NO_ID); 1 << SCALAR_CACHE_BITS];
        }
        let i = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - SCALAR_CACHE_BITS)) as usize;
        &mut self.slots[i]
    }
}

/// A record whose id has been assigned but whose bytes are not written yet.
enum Item {
    None,
    True,
    False,
    Int(i64),
    Float(f64),
    Str(u32),
    /// Index into `Walker::pinned`.
    Obj { pin: u32, depth: u32 },
}

thread_local! {
    /// Walker scratch (queue, identity memo) reused across calls on this thread.
    static WALK_POOL: RefCell<Option<(VecDeque<Item>, FxHashMap<usize, u32>)>> =
        const { RefCell::new(None) };
}

pub struct Walker<'py> {
    py: Python<'py>,
    out: RecordWriter,
    queue: VecDeque<Item>,
    /// Identity memo: object address -> record id.
    memo: FxHashMap<usize, u32>,
    /// Strong refs to every identity-memoised object (keeps addresses unique and
    /// borrowed `bytes`/`str` buffers alive until the output is assembled).
    pinned: Vec<Bound<'py, PyAny>>,
    int_cache: ScalarCache,
    /// Keyed by bit pattern, so 0.0/-0.0 and NaN payloads stay distinct.
    float_cache: ScalarCache,
    none_id: u32,
    true_id: u32,
    false_id: u32,
    /// Per-call type pointer -> (type id, metadata). The type id is only
    /// meaningful for dataclass / namedtuple / enum kinds.
    types: FxHashMap<usize, (u16, Arc<ClassInfo>)>,
    max_depth: u32,
}

#[cold]
fn recursion_error() -> PyErr {
    PyRecursionError::new_err("maximum recursion depth exceeded in serialization")
}

#[cold]
fn too_large(what: &str) -> PyErr {
    PyValueError::new_err(format!("{} exceeds the format's limits", what))
}

#[inline]
fn len_u32(n: usize, what: &str) -> PyResult<u32> {
    u32::try_from(n).map_err(|_| too_large(what))
}

impl<'py> Walker<'py> {
    pub fn new(py: Python<'py>) -> Self {
        // SAFETY: we hold the GIL; Py_GetRecursionLimit is part of the stable ABI.
        let limit = unsafe { ffi::Py_GetRecursionLimit() };
        let (queue, memo) = WALK_POOL
            .with(|p| p.borrow_mut().take())
            .unwrap_or_default();
        Self {
            py,
            out: RecordWriter::new(),
            queue,
            memo,
            pinned: Vec::new(),
            int_cache: ScalarCache::default(),
            float_cache: ScalarCache::default(),
            none_id: NO_ID,
            true_id: NO_ID,
            false_id: NO_ID,
            types: FxHashMap::default(),
            max_depth: limit.max(0) as u32,
        }
    }

    /// Serialize the graph rooted at `root` (which becomes record 0).
    pub fn walk(mut self, root: &Bound<'py, PyAny>) -> PyResult<Encoded<'py>> {
        let root_id = self.assign(root.as_ptr(), 0)?;
        debug_assert_eq!(root_id, 0);
        while let Some(item) = self.queue.pop_front() {
            self.write_item(item)?;
        }
        let Walker {
            out,
            pinned,
            queue,
            mut memo,
            ..
        } = self;
        // Recycle scratch buffers for the next call on this thread (queue is empty).
        if (queue.capacity() + memo.capacity()) * 16 <= RETAIN_MAX {
            memo.clear();
            let _ = WALK_POOL.try_with(|p| *p.borrow_mut() = Some((queue, memo)));
        }
        Ok(Encoded::new(out, pinned))
    }

    // ----- id assignment (never runs Python code) -----------------------------

    /// Return the record id for `ptr` (a live, borrowed object), allocating a new
    /// id and queueing the record if this value/object has not been seen.
    ///
    /// This function must not execute arbitrary Python code: callers iterate
    /// containers with borrowed references and rely on them not being mutated.
    #[inline]
    fn assign(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> PyResult<u32> {
        if depth >= self.max_depth {
            return Err(recursion_error());
        }
        // SAFETY: `ptr` is a valid object pointer and we hold the GIL. The type
        // statics are only compared by address.
        unsafe {
            let tp = ffi::Py_TYPE(ptr);
            if tp == addr_of_mut!(ffi::PyUnicode_Type) {
                return self.assign_str(ptr, depth);
            }
            if tp == addr_of_mut!(ffi::PyLong_Type) {
                let mut overflow = 0;
                let v = ffi::PyLong_AsLongLongAndOverflow(ptr, &mut overflow);
                if overflow == 0 {
                    if v == -1 && !ffi::PyErr_Occurred().is_null() {
                        return Err(PyErr::fetch(self.py));
                    }
                    return Ok(self.assign_int(v));
                }
                // Out of i64 range: written as BigInt when dequeued.
                return Ok(self.assign_obj(ptr, depth));
            }
            if tp == addr_of_mut!(ffi::PyFloat_Type) {
                let v = ffi::PyFloat_AsDouble(ptr);
                return Ok(self.assign_float(v));
            }
            if ptr == ffi::Py_None() {
                if self.none_id == NO_ID {
                    self.none_id = self.push(Item::None);
                }
                return Ok(self.none_id);
            }
            if tp == addr_of_mut!(ffi::PyBool_Type) {
                return Ok(if ptr == ffi::Py_True() {
                    if self.true_id == NO_ID {
                        self.true_id = self.push(Item::True);
                    }
                    self.true_id
                } else {
                    if self.false_id == NO_ID {
                        self.false_id = self.push(Item::False);
                    }
                    self.false_id
                });
            }
        }
        Ok(self.assign_obj(ptr, depth))
    }

    #[inline]
    fn push(&mut self, item: Item) -> u32 {
        self.queue.push_back(item);
        self.out.alloc_id()
    }

    #[inline]
    fn assign_int(&mut self, v: i64) -> u32 {
        let slot = self.int_cache.slot(v as u64);
        if slot.1 != NO_ID && slot.0 == v as u64 {
            return slot.1;
        }
        self.queue.push_back(Item::Int(v));
        let id = self.out.alloc_id();
        *slot = (v as u64, id);
        id
    }

    #[inline]
    fn assign_float(&mut self, v: f64) -> u32 {
        let bits = v.to_bits();
        let slot = self.float_cache.slot(bits);
        if slot.1 != NO_ID && slot.0 == bits {
            return slot.1;
        }
        self.queue.push_back(Item::Float(v));
        let id = self.out.alloc_id();
        *slot = (bits, id);
        id
    }

    #[inline]
    unsafe fn assign_str(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> PyResult<u32> {
        let mut size: ffi::Py_ssize_t = 0;
        let data = ffi::PyUnicode_AsUTF8AndSize(ptr, &mut size);
        if data.is_null() {
            // Not strict UTF-8 (lone surrogates): written as StrRaw when dequeued.
            drop(PyErr::take(self.py));
            return Ok(self.assign_obj(ptr, depth));
        }
        let len = size as usize;
        if len >= LARGE_STR {
            // Identity-memoised, not hashed; bytes spliced in at assembly.
            if let Some(&id) = self.memo.get(&(ptr as usize)) {
                return Ok(id);
            }
            len_u32(len, "string")?;
            // SAFETY: the UTF-8 buffer is owned by the str object, immutable and
            // lives as long as the object, which we pin right here.
            self.pinned.push(Bound::from_borrowed_ptr(self.py, ptr));
            let idx = self.out.push_str_ext(data as *const u8, len);
            let id = self.push(Item::Str(idx));
            self.memo.insert(ptr as usize, id);
            return Ok(id);
        }
        // SAFETY: valid for `len` bytes while `ptr` is alive (we only use it here).
        let bytes = std::slice::from_raw_parts(data as *const u8, len);
        let idx = self.out.intern_str(bytes);
        let id = *self.out.str_rec(idx);
        if id != NO_ID {
            return Ok(id);
        }
        let id = self.push(Item::Str(idx));
        *self.out.str_rec(idx) = id;
        Ok(id)
    }

    #[inline]
    fn assign_obj(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> u32 {
        match self.memo.entry(ptr as usize) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let pin = self.pinned.len() as u32;
                // SAFETY: `ptr` is a valid borrowed reference; this takes a new
                // strong reference, pinning the object for the rest of the call.
                self.pinned
                    .push(unsafe { Bound::from_borrowed_ptr(self.py, ptr) });
                self.queue.push_back(Item::Obj { pin, depth });
                *e.insert(self.out.alloc_id())
            }
        }
    }

    // ----- record emission ------------------------------------------------------

    fn write_item(&mut self, item: Item) -> PyResult<()> {
        match item {
            Item::None => self.out.tag(Tag::None),
            Item::True => self.out.tag(Tag::True),
            Item::False => self.out.tag(Tag::False),
            Item::Int(v) => self.out.int(v),
            Item::Float(v) => self.out.float(v),
            Item::Str(idx) => self.out.string(idx),
            Item::Obj { pin, depth } => {
                let ptr = self.pinned[pin as usize].as_ptr();
                // SAFETY: `ptr` is pinned (strong ref in `self.pinned`) and we hold the GIL.
                unsafe { self.write_obj(ptr, depth)? }
            }
        }
        Ok(())
    }

    /// Write a sequence record whose children come from `get(i)` (borrowed refs).
    #[inline]
    unsafe fn write_seq(
        &mut self,
        tag: Tag,
        ptr: *mut ffi::PyObject,
        n: ffi::Py_ssize_t,
        get: unsafe extern "C" fn(*mut ffi::PyObject, ffi::Py_ssize_t) -> *mut ffi::PyObject,
        depth: u32,
    ) -> PyResult<()> {
        if n < 0 {
            return Err(PyErr::fetch(self.py));
        }
        let count = len_u32(n as usize, "container")?;
        self.out.reserve(5 + 4 * n as usize);
        self.out.tag(tag);
        self.out.u32(count);
        for i in 0..n {
            // Borrowed reference; the container cannot change while we loop
            // because `assign` never runs Python code.
            let child = get(ptr, i);
            if child.is_null() {
                return Err(PyErr::fetch(self.py));
            }
            let id = self.assign(child, depth + 1)?;
            self.out.u32(id);
        }
        Ok(())
    }

    unsafe fn write_obj(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> PyResult<()> {
        let py = self.py;
        let tp = ffi::Py_TYPE(ptr);

        if tp == addr_of_mut!(ffi::PyList_Type) {
            return self.write_seq(Tag::List, ptr, ffi::PyList_Size(ptr), ffi::PyList_GetItem, depth);
        }
        if tp == addr_of_mut!(ffi::PyDict_Type) {
            let n = ffi::PyDict_Size(ptr);
            if n < 0 {
                return Err(PyErr::fetch(py));
            }
            let count = len_u32(n as usize, "dict")?;
            self.out.reserve(5 + 8 * n as usize);
            self.out.tag(Tag::Dict);
            self.out.u32(count);
            let mut pos: ffi::Py_ssize_t = 0;
            let mut k: *mut ffi::PyObject = std::ptr::null_mut();
            let mut v: *mut ffi::PyObject = std::ptr::null_mut();
            // Borrowed refs; safe because `assign` never runs Python code, so the
            // dict cannot be mutated during iteration.
            while ffi::PyDict_Next(ptr, &mut pos, &mut k, &mut v) != 0 {
                let kid = self.assign(k, depth + 1)?;
                let vid = self.assign(v, depth + 1)?;
                self.out.u32(kid);
                self.out.u32(vid);
            }
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyTuple_Type) {
            return self.write_seq(Tag::Tuple, ptr, ffi::PyTuple_Size(ptr), ffi::PyTuple_GetItem, depth);
        }
        if tp == addr_of_mut!(ffi::PySet_Type) || tp == addr_of_mut!(ffi::PyFrozenSet_Type) {
            let tag = if tp == addr_of_mut!(ffi::PySet_Type) {
                Tag::Set
            } else {
                Tag::FrozenSet
            };
            let obj = Bound::from_borrowed_ptr(py, ptr);
            self.out.tag(tag);
            let count_pos = self.out.pos();
            self.out.u32(0);
            let mut count: u32 = 0;
            for item in obj.try_iter()? {
                let item = item?;
                let id = self.assign(item.as_ptr(), depth + 1)?;
                self.out.u32(id);
                count = count.checked_add(1).ok_or_else(|| too_large("set length"))?;
            }
            self.out.patch_u32(count_pos, count);
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyBytes_Type) {
            let n = ffi::PyBytes_Size(ptr) as usize;
            len_u32(n, "bytes")?;
            let data = ffi::PyBytes_AsString(ptr) as *const u8;
            if n >= EXT_THRESHOLD {
                // SAFETY: bytes are immutable and the object is pinned.
                self.out.blob_ext(Tag::Bytes, data, n);
            } else {
                self.out.blob(Tag::Bytes, std::slice::from_raw_parts(data, n));
            }
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyByteArray_Type) {
            // Mutable: always copied now.
            let n = ffi::PyByteArray_Size(ptr) as usize;
            len_u32(n, "bytearray")?;
            let data = ffi::PyByteArray_AsString(ptr) as *const u8;
            let slice = if n == 0 { &[][..] } else { std::slice::from_raw_parts(data, n) };
            self.out.blob(Tag::ByteArray, slice);
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyLong_Type) {
            return self.write_bigint(ptr);
        }
        if tp == addr_of_mut!(ffi::PyUnicode_Type) {
            // Only strings that failed strict UTF-8 encoding get here.
            let obj = Bound::from_borrowed_ptr(py, ptr);
            let encoded = obj.call_method1(
                intern!(py, "encode"),
                (intern!(py, "utf-8"), intern!(py, "surrogatepass")),
            )?;
            let b = encoded.downcast_into::<PyBytes>()?;
            len_u32(b.as_bytes().len(), "string")?;
            self.out.blob(Tag::StrRaw, b.as_bytes());
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyComplex_Type) {
            let re = ffi::PyComplex_RealAsDouble(ptr);
            let im = ffi::PyComplex_ImagAsDouble(ptr);
            self.out.reserve(17);
            self.out.tag(Tag::Complex);
            self.out.raw(&re.to_le_bytes());
            self.out.raw(&im.to_le_bytes());
            return Ok(());
        }
        if tp == addr_of_mut!(ffi::PyRange_Type) || tp == addr_of_mut!(ffi::PySlice_Type) {
            let obj = Bound::from_borrowed_ptr(py, ptr);
            let tag = if tp == addr_of_mut!(ffi::PyRange_Type) { Tag::Range } else { Tag::Slice };
            let parts = [
                obj.getattr(intern!(py, "start"))?,
                obj.getattr(intern!(py, "stop"))?,
                obj.getattr(intern!(py, "step"))?,
            ];
            return self.write_children(tag, &parts, depth);
        }
        if ptr == ffi::Py_Ellipsis() {
            self.out.tag(Tag::Ellipsis);
            return Ok(());
        }
        if ptr == ffi::Py_NotImplemented() {
            self.out.tag(Tag::NotImplemented);
            return Ok(());
        }
        self.write_class(ptr, depth)
    }

    /// Writes `tag` followed by the record ids of `children` (fixed count, no
    /// length prefix). The children are owned, so assigning them is safe.
    fn write_children(&mut self, tag: Tag, children: &[Bound<'py, PyAny>], depth: u32) -> PyResult<()> {
        let mut ids = [0u32; 3];
        for (slot, c) in ids.iter_mut().zip(children) {
            *slot = self.assign(c.as_ptr(), depth + 1)?;
        }
        self.out.tag(tag);
        for id in &ids[..children.len()] {
            self.out.u32(*id);
        }
        Ok(())
    }

    /// Record id for a datetime/time `tzinfo` (`NO_ID` for naive values).
    fn tz_child(&mut self, tz: &Bound<'py, PyAny>, depth: u32) -> PyResult<u32> {
        if tz.is_none() {
            return Ok(NO_ID);
        }
        let ty = tz.get_type();
        match class_info(self.py, &ty)?.map(|i| i.kind) {
            Some(ClassKind::Std(Std::Timezone)) | Some(ClassKind::Std(Std::ZoneInfo)) => {
                self.assign(tz.as_ptr(), depth + 1)
            }
            _ => Err(unsupported(format_args!("tzinfo {}", ty.name()?))),
        }
    }

    /// Pair count + (key, value) ids of a dict (or a dict subclass whose
    /// storage order is its iteration order) via `PyDict_Next`.
    unsafe fn write_dict_pairs(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> PyResult<()> {
        let n = ffi::PyDict_Size(ptr);
        if n < 0 {
            return Err(PyErr::fetch(self.py));
        }
        let count = len_u32(n as usize, "dict")?;
        self.out.reserve(4 + 8 * n as usize);
        self.out.u32(count);
        let mut pos: ffi::Py_ssize_t = 0;
        let mut k: *mut ffi::PyObject = std::ptr::null_mut();
        let mut v: *mut ffi::PyObject = std::ptr::null_mut();
        // Borrowed refs; `assign` never runs Python code (see the Dict case).
        while ffi::PyDict_Next(ptr, &mut pos, &mut k, &mut v) != 0 {
            let kid = self.assign(k, depth + 1)?;
            let vid = self.assign(v, depth + 1)?;
            self.out.u32(kid);
            self.out.u32(vid);
        }
        Ok(())
    }

    fn write_timedelta(&mut self, td: &Bound<'py, PyAny>) -> PyResult<()> {
        let py = self.py;
        let days: i32 = td.getattr(intern!(py, "days"))?.extract()?;
        let secs: u32 = td.getattr(intern!(py, "seconds"))?.extract()?;
        let us: u32 = td.getattr(intern!(py, "microseconds"))?.extract()?;
        self.out.raw(&days.to_le_bytes());
        self.out.raw(&secs.to_le_bytes());
        self.out.raw(&us.to_le_bytes());
        Ok(())
    }

    unsafe fn write_std(&mut self, obj: &Bound<'py, PyAny>, kind: Std, depth: u32) -> PyResult<()> {
        let py = self.py;
        let u8_attr = |name: &Bound<'py, PyString>| -> PyResult<u8> { obj.getattr(name)?.extract() };
        match kind {
            Std::DateTime | Std::Date | Std::Time => {
                let mut buf: Vec<u8> = Vec::with_capacity(16);
                if kind != Std::Time {
                    let year: u16 = obj.getattr(intern!(py, "year"))?.extract()?;
                    buf.extend_from_slice(&year.to_le_bytes());
                    buf.push(u8_attr(intern!(py, "month"))?);
                    buf.push(u8_attr(intern!(py, "day"))?);
                }
                if kind != Std::Date {
                    buf.push(u8_attr(intern!(py, "hour"))?);
                    buf.push(u8_attr(intern!(py, "minute"))?);
                    buf.push(u8_attr(intern!(py, "second"))?);
                    let us: u32 = obj.getattr(intern!(py, "microsecond"))?.extract()?;
                    buf.extend_from_slice(&us.to_le_bytes());
                    buf.push(u8_attr(intern!(py, "fold"))?);
                    let tz = obj.getattr(intern!(py, "tzinfo"))?;
                    let tz_id = self.tz_child(&tz, depth)?;
                    buf.extend_from_slice(&tz_id.to_le_bytes());
                }
                self.out.tag(match kind {
                    Std::DateTime => Tag::DateTime,
                    Std::Date => Tag::Date,
                    _ => Tag::Time,
                });
                self.out.raw(&buf);
            }
            Std::TimeDelta => {
                self.out.tag(Tag::TimeDelta);
                self.write_timedelta(obj)?;
            }
            Std::Timezone => {
                let offset = obj.call_method1(intern!(py, "utcoffset"), (py.None(),))?;
                let args = obj.call_method0(intern!(py, "__getinitargs__"))?;
                let args = args.downcast::<PyTuple>()?;
                let name = if args.len() >= 2 {
                    let name = utf8_pass(&args.get_item(1)?)?;
                    len_u32(name.len(), "timezone name")?;
                    Some(name)
                } else {
                    None
                };
                self.out.tag(Tag::Timezone);
                self.write_timedelta(&offset)?;
                match name {
                    Some(name) => {
                        self.out.u8(1);
                        self.out.len_bytes(&name);
                    }
                    None => self.out.u8(0),
                }
            }
            Std::ZoneInfo => {
                let key = obj.getattr(intern!(py, "key"))?;
                if key.is_none() {
                    return Err(unsupported("ZoneInfo without a key"));
                }
                let key = utf8_pass(&key)?;
                len_u32(key.len(), "ZoneInfo key")?;
                self.out.blob(Tag::ZoneInfo, &key);
            }
            Std::Decimal => {
                let s = utf8_pass(obj.str()?.as_any())?;
                len_u32(s.len(), "Decimal")?;
                self.out.blob(Tag::Decimal, &s);
            }
            Std::Uuid => {
                let b = obj.getattr(intern!(py, "bytes"))?;
                let b = b.downcast::<PyBytes>()?.as_bytes();
                if b.len() != 16 {
                    return Err(PyValueError::new_err("UUID.bytes is not 16 bytes long"));
                }
                self.out.tag(Tag::Uuid);
                self.out.raw(b);
            }
            Std::Fraction => {
                let parts = [
                    obj.getattr(intern!(py, "numerator"))?,
                    obj.getattr(intern!(py, "denominator"))?,
                ];
                self.write_children(Tag::Fraction, &parts, depth)?;
            }
            Std::PurePosixPath | Std::PureWindowsPath | Std::PosixPath | Std::WindowsPath => {
                let flavor = match kind {
                    Std::PurePosixPath => 0u8,
                    Std::PureWindowsPath => 1,
                    Std::PosixPath => 2,
                    _ => 3,
                };
                let s = utf8_pass(obj.str()?.as_any())?;
                len_u32(s.len(), "path")?;
                self.out.tag(Tag::Path);
                self.out.u8(flavor);
                self.out.len_bytes(&s);
            }
            Std::OrderedDict => {
                // Iterate via items(): after move_to_end() the dict storage
                // order (PyDict_Next) differs from the OrderedDict's order.
                self.out.tag(Tag::OrderedDict);
                let count_pos = self.out.pos();
                self.out.u32(0);
                let mut count: u32 = 0;
                for item in obj.call_method0(intern!(py, "items"))?.try_iter()? {
                    let item = item?;
                    let pair = item.downcast::<PyTuple>()?;
                    let k = pair.get_item(0)?;
                    let v = pair.get_item(1)?;
                    let kid = self.assign(k.as_ptr(), depth + 1)?;
                    let vid = self.assign(v.as_ptr(), depth + 1)?;
                    self.out.u32(kid);
                    self.out.u32(vid);
                    count = count.checked_add(1).ok_or_else(|| too_large("OrderedDict length"))?;
                }
                self.out.patch_u32(count_pos, count);
            }
            Std::DefaultDict => {
                let factory = obj.getattr(intern!(py, "default_factory"))?;
                let code = if factory.is_none() {
                    0u8
                } else {
                    let fp = factory.as_ptr() as *mut ffi::PyTypeObject;
                    // Same order as `format::DEFAULT_FACTORIES[1..]`.
                    let builtins: [*mut ffi::PyTypeObject; 11] = [
                        addr_of_mut!(ffi::PyList_Type),
                        addr_of_mut!(ffi::PyDict_Type),
                        addr_of_mut!(ffi::PySet_Type),
                        addr_of_mut!(ffi::PyLong_Type),
                        addr_of_mut!(ffi::PyFloat_Type),
                        addr_of_mut!(ffi::PyUnicode_Type),
                        addr_of_mut!(ffi::PyTuple_Type),
                        addr_of_mut!(ffi::PyBytes_Type),
                        addr_of_mut!(ffi::PyFrozenSet_Type),
                        addr_of_mut!(ffi::PyBool_Type),
                        addr_of_mut!(ffi::PyComplex_Type),
                    ];
                    match builtins.iter().position(|&t| t == fp) {
                        Some(i) => i as u8 + 1,
                        None => {
                            let name = factory
                                .getattr(intern!(py, "__qualname__"))
                                .and_then(|n| n.str())
                                .or_else(|_| factory.repr())?;
                            return Err(unsupported(format_args!(
                                "defaultdict with factory {}",
                                name.to_string_lossy()
                            )));
                        }
                    }
                };
                self.out.tag(Tag::DefaultDict);
                self.out.u8(code);
                self.write_dict_pairs(obj.as_ptr(), depth)?;
            }
            Std::Counter => {
                self.out.tag(Tag::Counter);
                self.write_dict_pairs(obj.as_ptr(), depth)?;
            }
            Std::Deque => {
                let maxlen = obj.getattr(intern!(py, "maxlen"))?;
                let enc: u64 = if maxlen.is_none() {
                    0
                } else {
                    maxlen
                        .extract::<u64>()?
                        .checked_add(1)
                        .ok_or_else(|| too_large("deque maxlen"))?
                };
                self.out.tag(Tag::Deque);
                self.out.u64(enc);
                let count_pos = self.out.pos();
                self.out.u32(0);
                let mut count: u32 = 0;
                for item in obj.try_iter()? {
                    let item = item?;
                    let id = self.assign(item.as_ptr(), depth + 1)?;
                    self.out.u32(id);
                    count = count.checked_add(1).ok_or_else(|| too_large("deque length"))?;
                }
                self.out.patch_u32(count_pos, count);
            }
        }
        Ok(())
    }

    unsafe fn write_bigint(&mut self, ptr: *mut ffi::PyObject) -> PyResult<()> {
        let py = self.py;
        let obj = Bound::from_borrowed_ptr(py, ptr);
        let bits: usize = obj.call_method0(intern!(py, "bit_length"))?.extract()?;
        let n = (bits + 8) / 8;
        len_u32(n, "int")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item(intern!(py, "signed"), true)?;
        let b = obj.call_method(intern!(py, "to_bytes"), (n, intern!(py, "little")), Some(&kwargs))?;
        let b = b.downcast_into::<PyBytes>()?;
        self.out.blob(Tag::BigInt, b.as_bytes());
        Ok(())
    }

    /// Dataclasses, namedtuples, enum members and the exact stdlib types.
    unsafe fn write_class(&mut self, ptr: *mut ffi::PyObject, depth: u32) -> PyResult<()> {
        let py = self.py;
        let tp_key = ffi::Py_TYPE(ptr) as usize;
        let (type_id, info) = match self.types.get(&tp_key) {
            Some((id, info)) => (*id, info.clone()),
            None => {
                let obj = Bound::from_borrowed_ptr(py, ptr);
                let ty = obj.get_type();
                let Some(info) = class_info(py, &ty)? else {
                    return Err(unsupported(ty.name()?));
                };
                let id = if matches!(info.kind, ClassKind::Std(_)) {
                    0
                } else {
                    self.out
                        .register_type(
                            info.kind.table_kind(),
                            &info.name,
                            &info.field_names,
                            info.schema_version,
                        )
                        .ok_or_else(|| too_large("number of dataclass types (65535)"))?
                };
                self.types.insert(tp_key, (id, info.clone()));
                (id, info)
            }
        };
        match info.kind {
            ClassKind::Dataclass => {}
            ClassKind::Std(kind) => {
                let obj = Bound::from_borrowed_ptr(py, ptr);
                return self.write_std(&obj, kind, depth);
            }
            ClassKind::NamedTuple => {
                let n = ffi::PyTuple_Size(ptr);
                if n < 0 {
                    return Err(PyErr::fetch(py));
                }
                let count = len_u32(n as usize, "namedtuple")?;
                self.out.reserve(7 + 4 * n as usize);
                self.out.tag(Tag::NamedTuple);
                self.out.u16(type_id);
                self.out.u32(count);
                for i in 0..n {
                    // Borrowed; tuples are immutable and `assign` runs no Python code.
                    let child = ffi::PyTuple_GetItem(ptr, i);
                    if child.is_null() {
                        return Err(PyErr::fetch(py));
                    }
                    let id = self.assign(child, depth + 1)?;
                    self.out.u32(id);
                }
                return Ok(());
            }
            ClassKind::Enum => {
                // By name when the name looks the member up again (plain
                // members); otherwise (e.g. composite Flag values) by value.
                let obj = Bound::from_borrowed_ptr(py, ptr);
                let name = obj.getattr(intern!(py, "_name_"))?;
                let by_name = name.is_instance_of::<PyString>()
                    && obj
                        .get_type()
                        .getattr(intern!(py, "__members__"))?
                        .get_item(&name)
                        .map(|m| m.is(&obj))
                        .unwrap_or(false);
                let child = if by_name {
                    name
                } else {
                    obj.getattr(intern!(py, "_value_"))?
                };
                let id = self.assign(child.as_ptr(), depth + 1)?;
                self.out.tag(Tag::Enum);
                self.out.u16(type_id);
                self.out.u8(if by_name { 0 } else { 1 });
                self.out.u32(id);
                return Ok(());
            }
        }
        let nfields = info.field_objs.len();
        let field_count = u16::try_from(nfields).map_err(|_| too_large("dataclass field count (65535)"))?;
        self.out.reserve(5 + 4 * nfields);
        self.out.tag(Tag::Dataclass);
        self.out.u16(type_id);
        self.out.u16(field_count);
        for name in &info.field_objs {
            // May run Python code (properties/__getattr__); we hold an owned
            // reference to the result, and `ptr` itself is pinned.
            let val = ffi::PyObject_GetAttr(ptr, name.as_ptr());
            let val = Bound::from_owned_ptr_or_err(py, val)?;
            let id = self.assign(val.as_ptr(), depth + 1)?;
            self.out.u32(id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_string() {
        let mut graph = ObjectGraph::new();
        let idx1 = graph.intern_string("hello");
        let idx2 = graph.intern_string("hello");
        let idx3 = graph.intern_string("world");
        assert_eq!(idx1, idx2);
        assert_eq!(idx1, 0);
        assert_eq!(idx3, 1);
    }

    #[test]
    fn test_push_and_set() {
        let mut graph = ObjectGraph::new();
        let id = graph.push_placeholder();
        assert!(graph.records[id as usize].is_none());
        graph.set_record(id, Record::None);
        assert!(graph.records[id as usize].is_some());
    }
}
