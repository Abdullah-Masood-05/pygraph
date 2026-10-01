//! Wire-format decoder.
//!
//! Decoding happens in two phases:
//!
//! 1. [`decode`] (pure Rust, may run without the GIL) validates the whole
//!    payload once and builds a compact, allocation-free-per-record index
//!    ([`Rec`], 16 bytes each). Strings are validated in place and kept as
//!    `&str` slices into the input; container children are *not* copied —
//!    they are re-read lazily from the input slice during phase 2. Every
//!    child/record/string/type reference is bounds-checked here, so phase 2
//!    never sees an invalid index. The number of incoming references of each
//!    record is counted (saturating at 2) so phase 2 only memoizes records
//!    that can actually be reached more than once.
//!
//! 2. [`reconstruct`] (needs the GIL) materializes Python objects with an
//!    explicit frame stack (no native recursion, depth limited by the
//!    interpreter's recursion limit). Scalar children are materialized inline.
//!    Mutable containers (list/dict/set/dataclass/bytearray) that are shared
//!    are created and memoized *before* their children, so arbitrary DAGs and
//!    cycles work; immutable containers (tuple/frozenset) that are shared are
//!    marked in-progress and a cycle through them raises `ValueError`.
//!
//! The format is self-contained: this module does not depend on the
//! serializer's in-memory graph representation.

use std::sync::Mutex;

use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::*;
use rustc_hash::FxHashMap;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooShort(&'static str),
    InvalidMagic,
    InvalidUtf8(String),
    Truncated(&'static str),
    CountExceedsData {
        item: &'static str,
        count: u32,
        max_possible: usize,
    },
    UnknownTag(u8),
    InvalidReference(u32),
    InvalidStringIndex(u32),
    InvalidTypeId(u16),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort(msg) => write!(f, "Invalid data: too short for {}", msg),
            DecodeError::InvalidMagic => write!(f, "Invalid magic bytes"),
            DecodeError::InvalidUtf8(err) => write!(f, "Invalid UTF-8 in string table: {}", err),
            DecodeError::Truncated(msg) => write!(f, "Invalid data: {} truncated", msg),
            DecodeError::CountExceedsData {
                item,
                count,
                max_possible,
            } => {
                write!(
                    f,
                    "Invalid {} count {} exceeds maximum possible elements ({}) for remaining data",
                    item, count, max_possible
                )
            }
            DecodeError::UnknownTag(b) => write!(f, "Unknown tag: 0x{:02X}", b),
            DecodeError::InvalidReference(id) => write!(f, "Invalid reference: {}", id),
            DecodeError::InvalidStringIndex(idx) => write!(f, "Invalid string index: {}", idx),
            DecodeError::InvalidTypeId(id) => write!(f, "Invalid type_id: {}", id),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for PyErr {
    fn from(err: DecodeError) -> PyErr {
        pyo3::exceptions::PyValueError::new_err(err.to_string())
    }
}

/// A type-table entry, borrowing its strings from the input buffer.
#[derive(Debug, Clone)]
pub struct TypeDef<'a> {
    pub name: &'a str,
    pub fields: Vec<&'a str>,
    pub schema_version: u32,
}

/// Compact decoded record (16 bytes, `Copy`, no heap data).
///
/// Field meaning by tag:
/// - `Int`: `b` = i64 bits. `Float`: `b` = f64 bits.
/// - `String`: `a` = string-table index.
/// - `Bytes`/`ByteArray`/`StrRaw`/`BigInt`: `a` = byte length, `b` = payload offset.
/// - `Complex`: `b` = payload offset (16 bytes).
/// - `List`/`Tuple`/`Set`/`FrozenSet`: `a` = child count, `b` = offset of first u32 child id.
/// - `Dict`: `a` = pair count, `b` = offset of first (key, value) u32 pair.
/// - `Dataclass`: `aux` = type id, `a` = field count, `b` = offset of first u32 field id.
/// - `Reference`: `a` = target record id.
#[derive(Debug, Clone, Copy)]
struct Rec {
    tag: Tag,
    aux: u16,
    a: u32,
    b: u64,
}

/// Result of phase 1: a validated index over the input buffer.
pub struct DecodedGraph<'a> {
    data: &'a [u8],
    /// String table entries (UTF-8 not yet validated, see [`decode`]).
    pub strings: Vec<&'a [u8]>,
    pub types: Vec<TypeDef<'a>>,
    recs: Vec<Rec>,
    /// Incoming-reference count per record, saturating at 2 (root counts 1).
    rec_refs: Vec<u8>,
    /// Number of `String` records per string-table index, saturating at 2.
    str_refs: Vec<u8>,
}

impl DecodedGraph<'_> {
    #[cfg(test)]
    fn record_count(&self) -> usize {
        self.recs.len()
    }
}

#[inline]
fn bump(counter: &mut u8) {
    if *counter < 2 {
        *counter += 1;
    }
}

/// Reads a little-endian u32 at an offset that phase 1 already validated.
#[inline(always)]
fn u32_at(data: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&data[off..off + 4]);
    u32::from_le_bytes(b)
}

#[inline(always)]
fn f64_at(data: &[u8], off: usize) -> f64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[off..off + 8]);
    f64::from_le_bytes(b)
}

/// Validates `count` u32 child ids starting at `*offset`, bumping the
/// reference count of each target, and advances `*offset` past them.
#[inline]
fn scan_children(
    data: &[u8],
    offset: &mut usize,
    count: usize,
    rec_refs: &mut [u8],
    what: &'static str,
) -> Result<(), DecodeError> {
    let bytes = count
        .checked_mul(4)
        .ok_or(DecodeError::Truncated(what))?;
    let slice = read_bytes(data, offset, bytes).ok_or(DecodeError::Truncated(what))?;
    for chunk in slice.chunks_exact(4) {
        let id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        match rec_refs.get_mut(id as usize) {
            Some(c) => bump(c),
            None => return Err(DecodeError::InvalidReference(id)),
        }
    }
    Ok(())
}

/// Reads a u32 length and checks that many bytes remain; returns (len, payload offset).
#[inline]
fn length_prefixed(
    data: &[u8],
    offset: &mut usize,
    len_what: &'static str,
    data_what: &'static str,
) -> Result<(u32, usize), DecodeError> {
    let len = read_u32(data, offset).ok_or(DecodeError::Truncated(len_what))?;
    if len as usize > data.len().saturating_sub(*offset) {
        return Err(DecodeError::Truncated(data_what));
    }
    let start = *offset;
    *offset += len as usize;
    Ok((len, start))
}

pub fn decode(data: &[u8]) -> Result<DecodedGraph<'_>, DecodeError> {
    let mut offset = 0;

    let magic = read_bytes(data, &mut offset, 4).ok_or(DecodeError::TooShort("header"))?;
    if magic != MAGIC_WRITE && magic != MAGIC_LEGACY {
        return Err(DecodeError::InvalidMagic);
    }

    let _version = read_u16(data, &mut offset).ok_or(DecodeError::TooShort("version"))?;
    let _schema = read_u32(data, &mut offset).ok_or(DecodeError::TooShort("schema version"))?;
    let _flags = read_u8(data, &mut offset).ok_or(DecodeError::TooShort("flags"))?;

    // ---- string table -------------------------------------------------
    let string_count =
        read_u32(data, &mut offset).ok_or(DecodeError::Truncated("string count"))?;
    let max_strings = data.len().saturating_sub(offset) / 4;
    if (string_count as usize) > max_strings {
        return Err(DecodeError::CountExceedsData {
            item: "string",
            count: string_count,
            max_possible: max_strings,
        });
    }
    // Bounded by remaining input (each entry needs >= 4 bytes).
    // Entries are kept as raw byte slices: string *values* are UTF-8 decoded
    // (strictly) by CPython when materialized, so validating them here too
    // would decode every string twice. Type/field names are validated below.
    let mut strings: Vec<&[u8]> = Vec::with_capacity(string_count as usize);
    for _ in 0..string_count {
        let (len, start) =
            length_prefixed(data, &mut offset, "string length", "string data")?;
        strings.push(&data[start..start + len as usize]);
    }
    let get_str = |idx: u32| -> Result<&str, DecodeError> {
        let bytes = strings
            .get(idx as usize)
            .copied()
            .ok_or(DecodeError::InvalidStringIndex(idx))?;
        std::str::from_utf8(bytes).map_err(|e| DecodeError::InvalidUtf8(e.to_string()))
    };

    // ---- type table ---------------------------------------------------
    let type_count = read_u32(data, &mut offset).ok_or(DecodeError::Truncated("type count"))?;
    let max_types = data.len().saturating_sub(offset) / 12;
    if (type_count as usize) > max_types {
        return Err(DecodeError::CountExceedsData {
            item: "type",
            count: type_count,
            max_possible: max_types,
        });
    }
    let mut types: Vec<TypeDef> = Vec::with_capacity(type_count as usize);
    for _ in 0..type_count {
        let _type_id = read_u16(data, &mut offset).ok_or(DecodeError::Truncated("type_id"))?;
        let name_idx =
            read_u32(data, &mut offset).ok_or(DecodeError::Truncated("type name_idx"))?;
        let schema_version =
            read_u32(data, &mut offset).ok_or(DecodeError::Truncated("schema_version"))?;
        let field_count =
            read_u16(data, &mut offset).ok_or(DecodeError::Truncated("field_count"))?;
        let max_fields = data.len().saturating_sub(offset) / 4;
        if (field_count as usize) > max_fields {
            return Err(DecodeError::CountExceedsData {
                item: "field",
                count: field_count as u32,
                max_possible: max_fields,
            });
        }
        let mut fields = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            let fidx =
                read_u32(data, &mut offset).ok_or(DecodeError::Truncated("field index"))?;
            fields.push(get_str(fidx)?);
        }
        types.push(TypeDef {
            name: get_str(name_idx)?,
            fields,
            schema_version,
        });
    }

    // ---- records ------------------------------------------------------
    let obj_count = read_u32(data, &mut offset).ok_or(DecodeError::Truncated("object count"))?;
    let max_objects = data.len().saturating_sub(offset);
    if (obj_count as usize) > max_objects {
        return Err(DecodeError::CountExceedsData {
            item: "object",
            count: obj_count,
            max_possible: max_objects,
        });
    }
    let n = obj_count as usize;
    // `n` <= remaining input bytes (each record is >= 1 byte), so these
    // allocations are proportional to the actual input size.
    let mut rec_refs = vec![0u8; n];
    let mut str_refs = vec![0u8; strings.len()];
    let mut recs: Vec<Rec> = Vec::with_capacity(n.min(1 << 20));
    if n > 0 {
        rec_refs[0] = 1; // the root is referenced by the caller
    }

    for _ in 0..n {
        let tag_byte = read_u8(data, &mut offset).ok_or(DecodeError::Truncated("tag"))?;
        let tag = Tag::from_u8(tag_byte).ok_or(DecodeError::UnknownTag(tag_byte))?;
        let mut rec = Rec {
            tag,
            aux: 0,
            a: 0,
            b: 0,
        };
        match tag {
            Tag::None | Tag::True | Tag::False => {}
            Tag::Int => {
                let v = read_i64_zigzag(data, &mut offset).ok_or(DecodeError::Truncated("int"))?;
                rec.b = v as u64;
            }
            Tag::Float => {
                let bytes =
                    read_bytes(data, &mut offset, 8).ok_or(DecodeError::Truncated("float"))?;
                rec.b = u64::from_le_bytes(bytes.try_into().unwrap());
            }
            Tag::String => {
                let idx =
                    read_u32(data, &mut offset).ok_or(DecodeError::Truncated("string index"))?;
                let c = str_refs
                    .get_mut(idx as usize)
                    .ok_or(DecodeError::InvalidStringIndex(idx))?;
                bump(c);
                rec.a = idx;
            }
            Tag::Bytes | Tag::ByteArray => {
                let (len, start) =
                    length_prefixed(data, &mut offset, "bytes length", "bytes data")?;
                rec.a = len;
                rec.b = start as u64;
            }
            Tag::StrRaw => {
                let (len, start) =
                    length_prefixed(data, &mut offset, "raw string length", "raw string data")?;
                rec.a = len;
                rec.b = start as u64;
            }
            Tag::BigInt => {
                let (len, start) =
                    length_prefixed(data, &mut offset, "bigint length", "bigint data")?;
                rec.a = len;
                rec.b = start as u64;
            }
            Tag::Complex => {
                let start = offset;
                read_bytes(data, &mut offset, 16).ok_or(DecodeError::Truncated("complex"))?;
                rec.b = start as u64;
            }
            Tag::List | Tag::Tuple | Tag::Set | Tag::FrozenSet => {
                let (count_what, item, ref_what) = match tag {
                    Tag::List | Tag::Tuple => {
                        ("list/tuple count", "list/tuple item", "list/tuple ref")
                    }
                    _ => ("set count", "set item", "set ref"),
                };
                let count = read_u32(data, &mut offset).ok_or(DecodeError::Truncated(count_what))?;
                let max_refs = data.len().saturating_sub(offset) / 4;
                if count as usize > max_refs {
                    return Err(DecodeError::CountExceedsData {
                        item,
                        count,
                        max_possible: max_refs,
                    });
                }
                rec.a = count;
                rec.b = offset as u64;
                scan_children(data, &mut offset, count as usize, &mut rec_refs, ref_what)?;
            }
            Tag::Dict => {
                let count =
                    read_u32(data, &mut offset).ok_or(DecodeError::Truncated("dict count"))?;
                let max_pairs = data.len().saturating_sub(offset) / 8;
                if count as usize > max_pairs {
                    return Err(DecodeError::CountExceedsData {
                        item: "dict pair",
                        count,
                        max_possible: max_pairs,
                    });
                }
                rec.a = count;
                rec.b = offset as u64;
                scan_children(
                    data,
                    &mut offset,
                    count as usize * 2,
                    &mut rec_refs,
                    "dict value",
                )?;
            }
            Tag::Dataclass => {
                let type_id = read_u16(data, &mut offset)
                    .ok_or(DecodeError::Truncated("dataclass type_id"))?;
                let field_count = read_u16(data, &mut offset)
                    .ok_or(DecodeError::Truncated("dataclass field_count"))?;
                let max_fields = data.len().saturating_sub(offset) / 4;
                if field_count as usize > max_fields {
                    return Err(DecodeError::CountExceedsData {
                        item: "dataclass field",
                        count: field_count as u32,
                        max_possible: max_fields,
                    });
                }
                if type_id as usize >= types.len() {
                    return Err(DecodeError::InvalidTypeId(type_id));
                }
                rec.aux = type_id;
                rec.a = field_count as u32;
                rec.b = offset as u64;
                scan_children(
                    data,
                    &mut offset,
                    field_count as usize,
                    &mut rec_refs,
                    "dataclass field",
                )?;
            }
            Tag::Reference => {
                let target =
                    read_u32(data, &mut offset).ok_or(DecodeError::Truncated("reference"))?;
                let c = rec_refs
                    .get_mut(target as usize)
                    .ok_or(DecodeError::InvalidReference(target))?;
                bump(c);
                rec.a = target;
            }
        }
        recs.push(rec);
    }

    Ok(DecodedGraph {
        data,
        strings,
        types,
        recs,
        rec_refs,
        str_refs,
    })
}

// ======================================================================
// Phase 2: object reconstruction
// ======================================================================

/// Process-wide cache of per-type Python state, keyed by (name, field names).
/// `make_class` already returns the same class for the same key, so caching
/// it here only skips the Python-level call, the field-name interning and the
/// version lookup on every `loads`.
struct CachedType {
    cls: Py<PyAny>,
    names: Vec<Py<PyString>>,
    version: u32,
}

static TYPE_CACHE: Mutex<Option<FxHashMap<Vec<u8>, CachedType>>> = Mutex::new(None);
/// Bound on distinct cached types (untrusted inputs can name arbitrary types).
const TYPE_CACHE_MAX: usize = 4096;

static MAKE_CLASS: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
static OBJECT_NEW: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
static INT_FROM_BYTES: GILOnceCell<Py<PyAny>> = GILOnceCell::new();

fn make_class_fn(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    MAKE_CLASS
        .get_or_try_init(py, || {
            let m = py
                .import("pysafe_pickle._reconstruct")
                .or_else(|_| py.import("pygraph._reconstruct"))?;
            Ok::<_, PyErr>(m.getattr("make_class")?.unbind())
        })
        .map(|f| f.bind(py))
}

fn object_new_fn(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    OBJECT_NEW
        .get_or_try_init(py, || {
            Ok::<_, PyErr>(py.get_type::<PyAny>().getattr("__new__")?.unbind())
        })
        .map(|f| f.bind(py))
}

fn int_from_bytes_fn(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    INT_FROM_BYTES
        .get_or_try_init(py, || {
            Ok::<_, PyErr>(py.get_type::<PyInt>().getattr("from_bytes")?.unbind())
        })
        .map(|f| f.bind(py))
}

fn type_cache_key(td: &TypeDef<'_>) -> Vec<u8> {
    let mut key = Vec::with_capacity(
        4 + td.name.len() + td.fields.iter().map(|f| 4 + f.len()).sum::<usize>(),
    );
    key.extend_from_slice(&(td.name.len() as u32).to_le_bytes());
    key.extend_from_slice(td.name.as_bytes());
    for f in &td.fields {
        key.extend_from_slice(&(f.len() as u32).to_le_bytes());
        key.extend_from_slice(f.as_bytes());
    }
    key
}

/// Per-`loads` resolved dataclass type.
struct RtType<'py> {
    cls: Bound<'py, PyAny>,
    names: Vec<Bound<'py, PyString>>,
    migrate: bool,
    current_version: u32,
}

fn resolve_type<'py>(py: Python<'py>, td: &TypeDef<'_>) -> PyResult<RtType<'py>> {
    let key = type_cache_key(td);
    let hit = {
        // No Python code runs while the lock is held (only refcount bumps).
        let guard = TYPE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().and_then(|m| m.get(&key)).map(|c| {
            (
                c.cls.bind(py).clone(),
                c.names.iter().map(|n| n.bind(py).clone()).collect::<Vec<_>>(),
                c.version,
            )
        })
    };
    let (cls, names, version) = match hit {
        Some(h) => h,
        None => {
            let names: Vec<Bound<'py, PyString>> =
                td.fields.iter().map(|f| PyString::intern(py, f)).collect();
            let cls = make_class_fn(py)?.call1((td.name, PyList::new(py, &names)?))?;
            let version: u32 = cls
                .getattr("__pysafe_pickle_version__")
                .or_else(|_| cls.getattr("__pygraph_version__"))
                .and_then(|v| v.extract())
                .unwrap_or(0);
            let entry = CachedType {
                cls: cls.clone().unbind(),
                names: names.iter().map(|n| n.clone().unbind()).collect(),
                version,
            };
            let evicted = {
                let mut guard = TYPE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
                let map = guard.get_or_insert_with(FxHashMap::default);
                let evicted = if map.len() >= TYPE_CACHE_MAX {
                    Some(std::mem::take(map))
                } else {
                    None
                };
                map.insert(key, entry);
                evicted
            };
            // Drop evicted Python references outside the lock.
            drop(evicted);
            (cls, names, version)
        }
    };
    let serialized = td.schema_version;
    Ok(RtType {
        cls,
        names,
        migrate: serialized > 0 && version > 0 && serialized != version,
        current_version: version,
    })
}

enum Kind<'py> {
    /// Shared list: created up-front (memoized) and appended to.
    ListEarly(Bound<'py, PyList>),
    /// Unshared list: children collected on the value stack, built at the end.
    ListLate,
    Tuple,
    Dict(Bound<'py, PyDict>, Option<Bound<'py, PyAny>>),
    Set(Bound<'py, PySet>),
    FrozenSet,
    Dataclass {
        obj: Bound<'py, PyAny>,
        ty: usize,
        migrate: bool,
    },
}

struct Frame<'py> {
    id: u32,
    kind: Kind<'py>,
    /// Byte offset of the first child id.
    off: usize,
    /// Index of the next child id to read.
    next: usize,
    /// Number of child ids (2 * pairs for dicts).
    total: usize,
    /// Value-stack height when this frame started.
    base: usize,
    depth: usize,
}

enum Step<'py> {
    Value(Bound<'py, PyAny>),
    Push(Frame<'py>),
}

const SHARED: u8 = 2;

/// Leaf records have no children and can be materialized directly.
#[inline(always)]
fn is_leaf(tag: Tag) -> bool {
    !matches!(
        tag,
        Tag::List
            | Tag::Tuple
            | Tag::Dict
            | Tag::Set
            | Tag::FrozenSet
            | Tag::Dataclass
            | Tag::Reference
    )
}

struct Builder<'a, 'py> {
    py: Python<'py>,
    g: &'a DecodedGraph<'a>,
    memo: Vec<Option<Bound<'py, PyAny>>>,
    in_progress: Vec<bool>,
    str_cache: Vec<Option<Bound<'py, PyAny>>>,
    types: Vec<Option<RtType<'py>>>,
    values: Vec<Bound<'py, PyAny>>,
    max_depth: usize,
}

/// `bytes.decode("utf-8", errors)` (strict when `errors` is `None`). Invalid
/// input raises `UnicodeDecodeError` (a `ValueError` subclass).
#[inline]
fn decode_utf8<'py>(
    py: Python<'py>,
    bytes: &[u8],
    errors: Option<&std::ffi::CStr>,
) -> PyResult<Bound<'py, PyAny>> {
    // SAFETY: `bytes` is a valid slice for the duration of the call and its
    // length fits in Py_ssize_t (it came from a u32 length); `errors` is NULL
    // or a NUL-terminated C string; the GIL is held. The result is a new
    // reference or NULL with an exception set, which `from_owned_ptr_or_err`
    // takes ownership of / converts to PyErr.
    unsafe {
        let ptr = pyo3::ffi::PyUnicode_DecodeUTF8(
            bytes.as_ptr() as *const std::os::raw::c_char,
            bytes.len() as pyo3::ffi::Py_ssize_t,
            errors.map_or(std::ptr::null(), |e| e.as_ptr()),
        );
        Bound::from_owned_ptr_or_err(py, ptr)
    }
}

fn cyclic_immutable(id: usize) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(format!(
        "Cyclic reference involving immutable container at ref_id {} is not supported",
        id
    ))
}

impl<'a, 'py> Builder<'a, 'py> {
    #[inline]
    fn string(&mut self, idx: u32) -> PyResult<Bound<'py, PyAny>> {
        let i = idx as usize;
        let s = self.g.strings[i];
        if self.g.str_refs[i] >= SHARED {
            if let Some(cached) = &self.str_cache[i] {
                return Ok(cached.clone());
            }
            let obj = decode_utf8(self.py, s, None)?;
            self.str_cache[i] = Some(obj.clone());
            Ok(obj)
        } else {
            decode_utf8(self.py, s, None)
        }
    }

    fn rt_type(&mut self, ty: usize) -> PyResult<(Bound<'py, PyAny>, bool)> {
        if self.types[ty].is_none() {
            self.types[ty] = Some(resolve_type(self.py, &self.g.types[ty])?);
        }
        let rt = self.types[ty].as_ref().unwrap();
        Ok((rt.cls.clone(), rt.migrate))
    }

    fn big_int(&self, bytes: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let n = bytes.len();
        if n == 0 {
            return Ok(0i64.into_pyobject(py)?.into_any());
        }
        let fill = if bytes[n - 1] & 0x80 != 0 { 0xFF } else { 0x00 };
        if n <= 8 {
            let mut b = [fill; 8];
            b[..n].copy_from_slice(bytes);
            return Ok(i64::from_le_bytes(b).into_pyobject(py)?.into_any());
        }
        if n <= 16 {
            let mut b = [fill; 16];
            b[..n].copy_from_slice(bytes);
            return Ok(i128::from_le_bytes(b).into_pyobject(py)?.into_any());
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("signed", true)?;
        int_from_bytes_fn(py)?.call((PyBytes::new(py, bytes), "little"), Some(&kwargs))
    }

    /// Materializes a leaf record (any tag for which [`is_leaf`] is true).
    #[inline(always)]
    fn scalar(&mut self, r: Rec) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let data = self.g.data;
        let off = r.b as usize;
        Ok(match r.tag {
            Tag::None => py.None().into_bound(py),
            Tag::True => PyBool::new(py, true).to_owned().into_any(),
            Tag::False => PyBool::new(py, false).to_owned().into_any(),
            Tag::Int => (r.b as i64).into_pyobject(py)?.into_any(),
            Tag::Float => PyFloat::new(py, f64::from_bits(r.b)).into_any(),
            Tag::String => self.string(r.a)?,
            Tag::Bytes => PyBytes::new(py, &data[off..off + r.a as usize]).into_any(),
            _ => self.rare_scalar(r)?,
        })
    }

    #[inline(never)]
    fn rare_scalar(&mut self, r: Rec) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let data = self.g.data;
        let off = r.b as usize;
        Ok(match r.tag {
            Tag::ByteArray => PyByteArray::new(py, &data[off..off + r.a as usize]).into_any(),
            Tag::StrRaw => decode_utf8(py, &data[off..off + r.a as usize], Some(c"surrogatepass"))?,
            Tag::BigInt => self.big_int(&data[off..off + r.a as usize])?,
            Tag::Complex => {
                PyComplex::from_doubles(py, f64_at(data, off), f64_at(data, off + 8)).into_any()
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "internal error: non-leaf record materialized as scalar",
                ))
            }
        })
    }

    /// Evaluates record `id` at `depth`: returns a finished value, or a frame
    /// that must be filled with its children.
    fn resolve(&mut self, mut id: usize, mut depth: usize) -> PyResult<Step<'py>> {
        let py = self.py;
        let g = self.g;
        loop {
            if depth >= self.max_depth {
                return Err(pyo3::exceptions::PyRecursionError::new_err(
                    "maximum recursion depth exceeded during reconstruction",
                ));
            }
            let shared = g.rec_refs[id] >= SHARED;
            if shared {
                if self.in_progress[id] {
                    return Err(cyclic_immutable(id));
                }
                if let Some(obj) = &self.memo[id] {
                    return Ok(Step::Value(obj.clone()));
                }
            }
            let r = g.recs[id];
            let off = r.b as usize;
            let value: Bound<'py, PyAny> = match r.tag {
                Tag::Reference => {
                    id = r.a as usize;
                    depth += 1;
                    continue;
                }
                Tag::None
                | Tag::True
                | Tag::False
                | Tag::Int
                | Tag::Float
                | Tag::String
                | Tag::Bytes
                | Tag::ByteArray
                | Tag::StrRaw
                | Tag::BigInt
                | Tag::Complex => self.scalar(r)?,
                Tag::List | Tag::Tuple | Tag::Set | Tag::FrozenSet | Tag::Dict | Tag::Dataclass => {
                    let total = if r.tag == Tag::Dict {
                        r.a as usize * 2
                    } else {
                        r.a as usize
                    };
                    let kind = match r.tag {
                        Tag::List => {
                            if shared {
                                let l = PyList::empty(py);
                                self.memo[id] = Some(l.clone().into_any());
                                Kind::ListEarly(l)
                            } else {
                                Kind::ListLate
                            }
                        }
                        Tag::Tuple | Tag::FrozenSet => {
                            if shared {
                                self.in_progress[id] = true;
                            }
                            if r.tag == Tag::Tuple {
                                Kind::Tuple
                            } else {
                                Kind::FrozenSet
                            }
                        }
                        Tag::Dict => {
                            let d = PyDict::new(py);
                            if shared {
                                self.memo[id] = Some(d.clone().into_any());
                            }
                            Kind::Dict(d, None)
                        }
                        Tag::Set => {
                            let s = PySet::empty(py)?;
                            if shared {
                                self.memo[id] = Some(s.clone().into_any());
                            }
                            Kind::Set(s)
                        }
                        _ => {
                            // Dataclass: create the instance first (pickle
                            // semantics, `__init__` is not called) so that
                            // self-references resolve to it.
                            let ty = r.aux as usize;
                            let (cls, migrate) = self.rt_type(ty)?;
                            let obj = object_new_fn(py)?.call1((cls,))?;
                            if shared {
                                self.memo[id] = Some(obj.clone());
                            }
                            Kind::Dataclass { obj, ty, migrate }
                        }
                    };
                    return Ok(Step::Push(Frame {
                        id: id as u32,
                        kind,
                        off,
                        next: 0,
                        total,
                        base: self.values.len(),
                        depth,
                    }));
                }
            };
            if shared {
                self.memo[id] = Some(value.clone());
            }
            return Ok(Step::Value(value));
        }
    }

    /// Hands a finished child value to its parent frame.
    #[inline]
    fn deliver(&mut self, frame: &mut Frame<'py>, v: Bound<'py, PyAny>) -> PyResult<()> {
        let child_idx = frame.next - 1;
        match &mut frame.kind {
            Kind::ListEarly(l) => l.append(v),
            Kind::ListLate | Kind::Tuple | Kind::FrozenSet => {
                self.values.push(v);
                Ok(())
            }
            Kind::Dict(d, pending) => match pending.take() {
                None => {
                    *pending = Some(v);
                    Ok(())
                }
                Some(k) => d.set_item(k, v),
            },
            Kind::Set(s) => s.add(v),
            Kind::Dataclass { obj, ty, migrate } => {
                if *migrate {
                    self.values.push(v);
                    return Ok(());
                }
                let names = &self.types[*ty].as_ref().unwrap().names;
                if let Some(name) = names.get(child_idx) {
                    set_attr(obj, name, &v)?;
                }
                Ok(())
            }
        }
    }

    /// Completes a frame whose children have all been delivered.
    fn finish(&mut self, frame: Frame<'py>) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let id = frame.id as usize;
        match frame.kind {
            Kind::ListEarly(l) => Ok(l.into_any()),
            Kind::ListLate => Ok(PyList::new(py, self.values.drain(frame.base..))?.into_any()),
            Kind::Tuple | Kind::FrozenSet => {
                let obj = if matches!(frame.kind, Kind::Tuple) {
                    PyTuple::new(py, self.values.drain(frame.base..))?.into_any()
                } else {
                    PyFrozenSet::new(py, self.values.drain(frame.base..))?.into_any()
                };
                if self.g.rec_refs[id] >= SHARED {
                    self.in_progress[id] = false;
                    self.memo[id] = Some(obj.clone());
                }
                Ok(obj)
            }
            Kind::Dict(d, _) => Ok(d.into_any()),
            Kind::Set(s) => Ok(s.into_any()),
            Kind::Dataclass { obj, ty, migrate } => {
                if migrate {
                    self.finish_migration(&obj, ty, frame.base)?;
                } else {
                    // Fields missing from the record default to None
                    // (matches the old `cls(**kwargs)` behaviour).
                    let names = &self.types[ty].as_ref().unwrap().names;
                    if frame.total < names.len() {
                        let none = py.None().into_bound(py);
                        for name in &names[frame.total..] {
                            set_attr(&obj, name, &none)?;
                        }
                    }
                }
                Ok(obj)
            }
        }
    }

    /// Schema-migration path: runs `apply_migrations` on the serialized state
    /// and sets the migrated fields (and any extra keys) on `obj`.
    fn finish_migration(&mut self, obj: &Bound<'py, PyAny>, ty: usize, base: usize) -> PyResult<()> {
        let py = self.py;
        let td = &self.g.types[ty];
        let rt = self.types[ty].as_ref().unwrap();
        let serialized_version = td.schema_version;
        let values: Vec<Bound<'py, PyAny>> = self.values.drain(base..).collect();

        let state = PyDict::new(py);
        for (name, val) in td.fields.iter().zip(values.iter()) {
            state.set_item(*name, val)?;
        }
        state.set_item("__pysafe_pickle_version__", serialized_version)?;
        state.set_item("__pygraph_version__", serialized_version)?;

        let migrations = py
            .import("pysafe_pickle.migrations")
            .or_else(|_| py.import("pygraph.migrations"))?;
        let migrated = migrations.getattr("apply_migrations")?.call1((
            td.name,
            &state,
            serialized_version,
            rt.current_version,
        ))?;
        let migrated: &Bound<'py, PyDict> = migrated.downcast().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err("Migration function must return a dict")
        })?;

        let none = py.None().into_bound(py);
        for name in &rt.names {
            let v = migrated.get_item(name)?.unwrap_or_else(|| none.clone());
            set_attr(obj, name, &v)?;
        }

        let fields_obj = obj.getattr("__dataclass_fields__")?;
        let current_fields: &Bound<'py, PyDict> = fields_obj.downcast().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err("__dataclass_fields__ is not a dict")
        })?;
        let extra = PyDict::new(py);
        for (k, v) in migrated.iter() {
            let key: String = k.extract()?;
            if key != "__pysafe_pickle_version__"
                && key != "__pygraph_version__"
                && !current_fields.contains(&k)?
            {
                extra.set_item(&k, v)?;
            }
        }
        if !extra.is_empty() {
            let _ = obj.setattr("__pysafe_pickle_extra__", &extra);
            let _ = obj.setattr("__pygraph_extra__", extra);
        }
        Ok(())
    }
}

/// `object.__setattr__(obj, name, value)`.
#[inline]
fn set_attr(obj: &Bound<'_, PyAny>, name: &Bound<'_, PyString>, value: &Bound<'_, PyAny>) -> PyResult<()> {
    // SAFETY: all three pointers are valid, owned by live `Bound`s, and the
    // GIL is held. PyObject_GenericSetAttr does not steal references.
    let rc = unsafe {
        pyo3::ffi::PyObject_GenericSetAttr(obj.as_ptr(), name.as_ptr(), value.as_ptr())
    };
    if rc < 0 {
        Err(PyErr::fetch(obj.py()))
    } else {
        Ok(())
    }
}

/// Materializes the root record (record 0) of a decoded graph.
pub fn reconstruct<'py>(py: Python<'py>, g: &DecodedGraph<'_>) -> PyResult<Bound<'py, PyAny>> {
    let n = g.recs.len();
    if n == 0 {
        return Err(DecodeError::InvalidReference(0).into());
    }
    // SAFETY: plain C call with the GIL held; returns the current limit.
    let limit = unsafe { pyo3::ffi::Py_GetRecursionLimit() };
    // `memo`/`in_progress` are only ever indexed for shared records and
    // `str_cache` only for shared strings, so skip allocating them (a
    // noticeable cost for small payloads) when nothing is shared.
    let any_shared = g.rec_refs.iter().any(|&c| c >= SHARED);
    let any_shared_str = g.str_refs.iter().any(|&c| c >= SHARED);
    let none_vec = |len: usize| -> Vec<Option<_>> { std::iter::repeat_with(|| None).take(len).collect() };
    let mut b = Builder {
        py,
        g,
        memo: none_vec(if any_shared { n } else { 0 }),
        in_progress: if any_shared { vec![false; n] } else { Vec::new() },
        str_cache: if any_shared_str {
            std::iter::repeat_with(|| None).take(g.strings.len()).collect()
        } else {
            Vec::new()
        },
        types: std::iter::repeat_with(|| None).take(g.types.len()).collect(),
        values: Vec::with_capacity(n.min(64)),
        max_depth: if limit > 0 { limit as usize } else { 1000 },
    };

    let mut frames: Vec<Frame<'py>> = match b.resolve(0, 0)? {
        Step::Value(v) => return Ok(v),
        Step::Push(f) => vec![f],
    };

    loop {
        let top = frames.last_mut().expect("frame stack is non-empty");
        let depth = top.depth + 1;
        let mut child_frame = None;
        // Sequence frames collecting onto the value stack: tight loop for
        // leaf children.
        if matches!(top.kind, Kind::ListLate | Kind::Tuple | Kind::FrozenSet) && depth < b.max_depth {
            let ids = &g.data[top.off..top.off + 4 * top.total];
            while top.next < top.total {
                let o = 4 * top.next;
                let cid = u32::from_le_bytes([ids[o], ids[o + 1], ids[o + 2], ids[o + 3]]) as usize;
                if g.rec_refs[cid] >= SHARED {
                    break;
                }
                let r = g.recs[cid];
                if !is_leaf(r.tag) {
                    break;
                }
                let v = b.scalar(r)?;
                b.values.push(v);
                top.next += 1;
            }
        }
        while top.next < top.total {
            let cid = u32_at(g.data, top.off + 4 * top.next) as usize;
            top.next += 1;
            // Fast paths: an already-built shared record, or an unshared leaf
            // (the common cases) skip the general resolver.
            if depth < b.max_depth {
                if g.rec_refs[cid] >= SHARED {
                    if let Some(obj) = &b.memo[cid] {
                        let v = obj.clone();
                        b.deliver(top, v)?;
                        continue;
                    }
                } else {
                    let r = g.recs[cid];
                    if is_leaf(r.tag) {
                        let v = b.scalar(r)?;
                        b.deliver(top, v)?;
                        continue;
                    }
                }
            }
            match b.resolve(cid, depth)? {
                Step::Value(v) => b.deliver(top, v)?,
                Step::Push(f) => {
                    child_frame = Some(f);
                    break;
                }
            }
        }
        if let Some(f) = child_frame {
            frames.push(f);
            continue;
        }
        let f = frames.pop().expect("frame stack is non-empty");
        let v = b.finish(f)?;
        match frames.last_mut() {
            Some(parent) => b.deliver(parent, v)?,
            None => return Ok(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(strings: &[&str], records: &[Vec<u8>]) -> Vec<u8> {
        let mut buf = Vec::new();
        write_header(&mut buf, 0);
        write_u32(&mut buf, strings.len() as u32);
        for s in strings {
            write_u32(&mut buf, s.len() as u32);
            buf.extend_from_slice(s.as_bytes());
        }
        write_u32(&mut buf, 0); // no types
        write_u32(&mut buf, records.len() as u32);
        for r in records {
            buf.extend_from_slice(r);
        }
        buf
    }

    fn list(tag: Tag, ids: &[u32]) -> Vec<u8> {
        let mut r = vec![tag as u8];
        r.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        for id in ids {
            r.extend_from_slice(&id.to_le_bytes());
        }
        r
    }

    #[test]
    fn parses_new_tags() {
        let mut big = vec![Tag::BigInt as u8];
        big.extend_from_slice(&3u32.to_le_bytes());
        big.extend_from_slice(&[1, 2, 3]);
        let mut ba = vec![Tag::ByteArray as u8];
        ba.extend_from_slice(&2u32.to_le_bytes());
        ba.extend_from_slice(b"hi");
        let mut cx = vec![Tag::Complex as u8];
        cx.extend_from_slice(&1.5f64.to_le_bytes());
        cx.extend_from_slice(&(-2.0f64).to_le_bytes());
        let mut raw = vec![Tag::StrRaw as u8];
        raw.extend_from_slice(&3u32.to_le_bytes());
        raw.extend_from_slice(&[0xED, 0xA0, 0x80]);
        let data = header(&[], &[list(Tag::List, &[1, 2, 3, 4]), big, ba, cx, raw]);
        let g = decode(&data).unwrap();
        assert_eq!(g.record_count(), 5);
        assert_eq!(g.recs[1].tag, Tag::BigInt);
        assert_eq!(g.recs[1].a, 3);
        assert_eq!(g.recs[3].tag, Tag::Complex);
        assert_eq!(f64_at(&data, g.recs[3].b as usize), 1.5);
        assert_eq!(g.recs[4].a, 3);
    }

    #[test]
    fn truncated_bigint_rejected() {
        let mut big = vec![Tag::BigInt as u8];
        big.extend_from_slice(&100u32.to_le_bytes());
        big.extend_from_slice(&[1, 2, 3]);
        let data = header(&[], &[big]);
        assert!(matches!(decode(&data), Err(DecodeError::Truncated(_))));
    }

    #[test]
    fn truncated_complex_rejected() {
        let mut cx = vec![Tag::Complex as u8];
        cx.extend_from_slice(&[0u8; 15]);
        let data = header(&[], &[cx]);
        assert!(matches!(decode(&data), Err(DecodeError::Truncated(_))));
    }

    #[test]
    fn invalid_child_reference_rejected() {
        let data = header(&[], &[list(Tag::List, &[7])]);
        assert_eq!(decode(&data).err(), Some(DecodeError::InvalidReference(7)));
    }

    #[test]
    fn invalid_string_index_rejected() {
        let mut s = vec![Tag::String as u8];
        s.extend_from_slice(&3u32.to_le_bytes());
        let data = header(&["a"], &[s]);
        assert_eq!(decode(&data).err(), Some(DecodeError::InvalidStringIndex(3)));
    }

    #[test]
    fn unknown_tag_rejected() {
        let data = header(&[], &[vec![0x7F]]);
        assert_eq!(decode(&data).err(), Some(DecodeError::UnknownTag(0x7F)));
    }

    #[test]
    fn count_bombs_rejected() {
        let mut r = vec![Tag::Dict as u8];
        r.extend_from_slice(&u32::MAX.to_le_bytes());
        let data = header(&[], &[r]);
        let err = decode(&data).err().unwrap();
        assert!(err.to_string().contains("exceeds maximum possible elements"));
    }

    #[test]
    fn reference_counts_for_dag_and_cycles() {
        // 0: [1, 1, 2]   1: None   2: [0]  (cycle back to root)
        let data = header(
            &[],
            &[
                list(Tag::List, &[1, 1, 2]),
                vec![Tag::None as u8],
                list(Tag::List, &[0]),
            ],
        );
        let g = decode(&data).unwrap();
        assert_eq!(g.rec_refs, vec![2, 2, 1]);
    }

    #[test]
    #[ignore]
    fn bench_pass1() {
        for f in ["wide", "strs", "ints"] {
            let data = std::fs::read(format!(".venv-de/{}.bin", f)).unwrap();
            let mut best = std::time::Duration::MAX;
            for _ in 0..2000 {
                let t = std::time::Instant::now();
                let g = decode(&data).unwrap();
                std::hint::black_box(&g);
                drop(g);
                best = best.min(t.elapsed());
            }
            println!("{} {:?}", f, best);
            let n = decode(&data).unwrap().recs.len();
            let mut best = std::time::Duration::MAX;
            for _ in 0..2000 {
                let t = std::time::Instant::now();
                let mut v: Vec<Rec> = Vec::with_capacity(n);
                for i in 0..n {
                    v.push(Rec { tag: Tag::None, aux: 0, a: i as u32, b: 0 });
                }
                let r = vec![0u8; n];
                std::hint::black_box((&v, &r));
                drop(v);
                best = best.min(t.elapsed());
            }
            println!("  alloc+fill {:?}", best);
        }
    }

    #[test]
    fn invalid_utf8_type_name_rejected() {
        let mut buf = Vec::new();
        write_header(&mut buf, 0);
        write_u32(&mut buf, 1); // one string: b"\xFF"
        write_u32(&mut buf, 1);
        buf.push(0xFF);
        write_u32(&mut buf, 1); // one type named by string 0
        write_u16(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u16(&mut buf, 0);
        write_u32(&mut buf, 0);
        assert!(matches!(decode(&buf), Err(DecodeError::InvalidUtf8(_))));
    }

    #[test]
    fn invalid_type_name_index_rejected() {
        let mut buf = Vec::new();
        write_header(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 1);
        write_u16(&mut buf, 0);
        write_u32(&mut buf, 5); // name_idx out of range
        write_u32(&mut buf, 0);
        write_u16(&mut buf, 0);
        write_u32(&mut buf, 0);
        assert_eq!(decode(&buf).err(), Some(DecodeError::InvalidStringIndex(5)));
    }
}
