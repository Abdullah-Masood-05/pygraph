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

use std::cell::Cell;
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
    InvalidField(&'static str),
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
            DecodeError::InvalidField(what) => write!(f, "Invalid data: bad {}", what),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for PyErr {
    #[cold]
    #[inline(never)]
    fn from(err: DecodeError) -> PyErr {
        crate::errors::decode_error(err.to_string())
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
    /// Offset just past the last record (bytes of `data` this payload uses).
    pub end: usize,
}

/// Phase-1 index buffers, kept per thread between calls: for small payloads
/// the heap allocations are a visible share of the cost of a `loads`.
struct Scratch {
    strings: Vec<&'static [u8]>,
    recs: Vec<Rec>,
    rec_refs: Vec<u8>,
    str_refs: Vec<u8>,
}

impl Scratch {
    const EMPTY: Scratch = Scratch {
        strings: Vec::new(),
        recs: Vec::new(),
        rec_refs: Vec::new(),
        str_refs: Vec::new(),
    };
}

/// Buffers with a larger capacity (in elements) are freed instead of kept, so
/// a thread retains well under 1 MB; for payloads that large the allocation
/// cost is negligible anyway.
const SCRATCH_KEEP_MAX: usize = 1 << 14;

thread_local! {
    static SCRATCH: Cell<Scratch> = const { Cell::new(Scratch::EMPTY) };
    static SCRATCH2: Cell<Scratch2> = const { Cell::new(Scratch2::EMPTY) };
}

fn take_scratch() -> Scratch {
    // `try_with`: the thread-local may already be gone during thread exit.
    SCRATCH.try_with(|s| s.replace(Scratch::EMPTY)).unwrap_or(Scratch::EMPTY)
}

/// Empties `v` and returns its allocation as a `Vec<U>` (which must have the
/// same layout as `T`; in practice `U` is `T` with a different lifetime).
/// Returns an unallocated vector if the capacity is above
/// [`SCRATCH_KEEP_MAX`].
fn recycle<T, U>(mut v: Vec<T>) -> Vec<U> {
    if v.capacity() > SCRATCH_KEEP_MAX {
        return Vec::new();
    }
    v.clear();
    // `v` is empty, so the closure never runs; for same-layout element
    // types `collect` reuses the allocation (in-place iteration).
    v.into_iter().map(|_| unreachable!()).collect()
}

impl Drop for DecodedGraph<'_> {
    fn drop(&mut self) {
        let s = Scratch {
            strings: recycle(std::mem::take(&mut self.strings)),
            recs: recycle(std::mem::take(&mut self.recs)),
            rec_refs: recycle(std::mem::take(&mut self.rec_refs)),
            str_refs: recycle(std::mem::take(&mut self.str_refs)),
        };
        let _ = SCRATCH.try_with(|c| c.set(s));
    }
}

impl DecodedGraph<'_> {
    #[cfg(test)]
    fn record_count(&self) -> usize {
        self.recs.len()
    }
}

#[inline]
fn bump(counter: &mut u8) {
    // Saturating at 2, branch-free.
    *counter += (*counter < 2) as u8;
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

/// Skips `n` fixed payload bytes; returns their offset.
#[inline]
fn fixed(data: &[u8], offset: &mut usize, n: usize, what: &'static str) -> Result<usize, DecodeError> {
    let start = *offset;
    read_bytes(data, offset, n).ok_or(DecodeError::Truncated(what))?;
    Ok(start)
}

/// Reads an optional tzinfo record id (`u32::MAX` = none).
#[inline]
fn tz_ref(data: &[u8], offset: &mut usize, rec_refs: &mut [u8]) -> Result<u32, DecodeError> {
    let id = read_u32(data, offset).ok_or(DecodeError::Truncated("tzinfo ref"))?;
    if id != u32::MAX {
        bump(rec_refs.get_mut(id as usize).ok_or(DecodeError::InvalidReference(id))?);
    }
    Ok(id)
}

/// Reads a u32 count of `width`-byte entries (bounded by the remaining input).
#[inline]
fn bounded_count(
    data: &[u8],
    offset: &mut usize,
    width: usize,
    item: &'static str,
) -> Result<u32, DecodeError> {
    let count = read_u32(data, offset).ok_or(DecodeError::Truncated(item))?;
    let max = data.len().saturating_sub(*offset) / width;
    if count as usize > max {
        return Err(DecodeError::CountExceedsData {
            item,
            count,
            max_possible: max,
        });
    }
    Ok(count)
}

/// Phase-1 parsing of the stdlib / enum / namedtuple records (tags >= 0x13).
/// Field meaning in [`Rec`]:
/// - `DateTime`/`Time`: `b` = payload offset, `a` = tzinfo id (u32::MAX = none).
/// - `Date`/`TimeDelta`/`Uuid`: `b` = payload offset.
/// - `Timezone`: `b` = offset of the 12-byte timedelta, `aux` = has_name,
///   `a` = name length (name bytes at `b + 17`).
/// - `ZoneInfo`/`Decimal`: `a` = length, `b` = offset. `Path`: same, `aux` = flavor.
/// - `Fraction`/`Range`/`Slice`: `a` = child count (2/3/3), `b` = first child id offset.
/// - `OrderedDict`/`Counter`/`DefaultDict`: as `Dict`; `aux` = factory code.
/// - `Deque`: `b` = offset of the u64 maxlen (children at `b + 12`), `a` = count.
/// - `Enum`: `aux` = type id, `a` = mode, `b` = offset of the child id.
/// - `NamedTuple`: `aux` = type id, `a` = count, `b` = first child id offset.
///
/// Takes and returns the offset by value (and returns the record) so that the
/// hot loop in [`decode`] keeps its own state in registers.
#[inline(never)]
fn parse_ext_record(
    data: &[u8],
    offset: usize,
    tag: Tag,
    rec_refs: &mut [u8],
    type_count: usize,
) -> Result<(Rec, usize), DecodeError> {
    let mut off = offset;
    let mut rec = Rec {
        tag,
        aux: 0,
        a: 0,
        b: 0,
    };
    parse_ext_fields(data, &mut off, &mut rec, rec_refs, type_count)?;
    Ok((rec, off))
}

#[inline(always)]
fn parse_ext_fields(
    data: &[u8],
    offset: &mut usize,
    rec: &mut Rec,
    rec_refs: &mut [u8],
    type_count: usize,
) -> Result<(), DecodeError> {
    match rec.tag {
        Tag::DateTime => {
            rec.b = fixed(data, offset, 12, "datetime")? as u64;
            rec.a = tz_ref(data, offset, rec_refs)?;
        }
        Tag::Time => {
            rec.b = fixed(data, offset, 8, "time")? as u64;
            rec.a = tz_ref(data, offset, rec_refs)?;
        }
        Tag::Date => rec.b = fixed(data, offset, 4, "date")? as u64,
        Tag::TimeDelta => rec.b = fixed(data, offset, 12, "timedelta")? as u64,
        Tag::Uuid => rec.b = fixed(data, offset, 16, "uuid")? as u64,
        Tag::Timezone => {
            rec.b = fixed(data, offset, 12, "timezone")? as u64;
            let has_name = read_u8(data, offset).ok_or(DecodeError::Truncated("timezone"))?;
            if has_name > 1 {
                return Err(DecodeError::InvalidField("timezone name flag"));
            }
            rec.aux = has_name as u16;
            if has_name == 1 {
                let (len, _) = length_prefixed(data, offset, "timezone name length", "timezone name")?;
                rec.a = len;
            }
        }
        Tag::ZoneInfo | Tag::Decimal => {
            let (len, start) = length_prefixed(data, offset, "length", "payload")?;
            rec.a = len;
            rec.b = start as u64;
        }
        Tag::Path => {
            let flavor = read_u8(data, offset).ok_or(DecodeError::Truncated("path flavor"))?;
            if flavor > 3 {
                return Err(DecodeError::InvalidField("path flavor"));
            }
            rec.aux = flavor as u16;
            let (len, start) = length_prefixed(data, offset, "path length", "path data")?;
            rec.a = len;
            rec.b = start as u64;
        }
        Tag::Fraction | Tag::Range | Tag::Slice => {
            let n = if rec.tag == Tag::Fraction { 2 } else { 3 };
            rec.a = n as u32;
            rec.b = *offset as u64;
            scan_children(data, offset, n, rec_refs, "child ref")?;
        }
        Tag::Ellipsis | Tag::NotImplemented => {}
        Tag::OrderedDict | Tag::Counter | Tag::DefaultDict => {
            if rec.tag == Tag::DefaultDict {
                let code = read_u8(data, offset).ok_or(DecodeError::Truncated("defaultdict factory"))?;
                if code as usize >= DEFAULT_FACTORIES.len() {
                    return Err(DecodeError::InvalidField("defaultdict factory"));
                }
                rec.aux = code as u16;
            }
            let count = bounded_count(data, offset, 8, "dict pair")?;
            rec.a = count;
            rec.b = *offset as u64;
            scan_children(data, offset, count as usize * 2, rec_refs, "dict value")?;
        }
        Tag::Deque => {
            rec.b = fixed(data, offset, 8, "deque maxlen")? as u64;
            let count = bounded_count(data, offset, 4, "deque item")?;
            rec.a = count;
            scan_children(data, offset, count as usize, rec_refs, "deque item")?;
        }
        Tag::Enum | Tag::NamedTuple => {
            let type_id = read_u16(data, offset).ok_or(DecodeError::Truncated("type_id"))?;
            if type_id as usize >= type_count {
                return Err(DecodeError::InvalidTypeId(type_id));
            }
            rec.aux = type_id;
            let count = if rec.tag == Tag::Enum {
                let mode = read_u8(data, offset).ok_or(DecodeError::Truncated("enum mode"))?;
                if mode > 1 {
                    return Err(DecodeError::InvalidField("enum mode"));
                }
                rec.a = mode as u32;
                1
            } else {
                let count = bounded_count(data, offset, 4, "namedtuple item")?;
                rec.a = count;
                count
            };
            rec.b = *offset as u64;
            scan_children(data, offset, count as usize, rec_refs, "child ref")?;
        }
        _ => return Err(DecodeError::UnknownTag(rec.tag as u8)),
    }
    Ok(())
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
    let scratch = take_scratch();
    let mut strings: Vec<&[u8]> = scratch.strings;
    strings.clear();
    strings.reserve(string_count as usize);
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
    let mut rec_refs = scratch.rec_refs;
    rec_refs.clear();
    rec_refs.resize(n, 0);
    let mut str_refs = scratch.str_refs;
    str_refs.clear();
    str_refs.resize(strings.len(), 0);
    let mut recs: Vec<Rec> = scratch.recs;
    recs.clear();
    recs.reserve(n.min(1 << 20));
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
            _ => {
                let (r, o) = parse_ext_record(data, offset, tag, &mut rec_refs, types.len())?;
                rec = r;
                offset = o;
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
        end: offset,
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

static RECONSTRUCT: GILOnceCell<Py<PyModule>> = GILOnceCell::new();

/// Attribute of `pysafe_pickle._reconstruct` (helpers for classes=, namedtuples, enums).
fn recon<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    RECONSTRUCT
        .get_or_try_init(py, || {
            let m = py
                .import("pysafe_pickle._reconstruct")
                .or_else(|_| py.import("pygraph._reconstruct"))?;
            Ok::<_, PyErr>(m.unbind())
        })?
        .bind(py)
        .getattr(name)
}

/// Stdlib constructors used for the 0x13+ tags, imported on first use.
#[derive(Clone, Copy)]
enum StdCtor {
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

const STD_CTORS: [(&str, &str); 17] = [
    ("datetime", "datetime"),
    ("datetime", "date"),
    ("datetime", "time"),
    ("datetime", "timedelta"),
    ("datetime", "timezone"),
    ("zoneinfo", "ZoneInfo"),
    ("decimal", "Decimal"),
    ("uuid", "UUID"),
    ("fractions", "Fraction"),
    ("pathlib", "PurePosixPath"),
    ("pathlib", "PureWindowsPath"),
    ("pathlib", "PosixPath"),
    ("pathlib", "WindowsPath"),
    ("collections", "OrderedDict"),
    ("collections", "defaultdict"),
    ("collections", "Counter"),
    ("collections", "deque"),
];

#[allow(clippy::declare_interior_mutable_const)]
const CTOR_INIT: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
static STD_CELLS: [GILOnceCell<Py<PyAny>>; 17] = [CTOR_INIT; 17];

fn std_ctor(py: Python<'_>, which: StdCtor) -> PyResult<&Bound<'_, PyAny>> {
    let i = which as usize;
    STD_CELLS[i]
        .get_or_try_init(py, || {
            let (m, a) = STD_CTORS[i];
            Ok::<_, PyErr>(py.import(m)?.getattr(a)?.unbind())
        })
        .map(|f| f.bind(py))
}

#[cold]
fn invalid(msg: impl Into<String>) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(msg.into())
}

/// Rejects ZoneInfo keys that could address files outside the tz database.
fn check_zone_key(key: &str) -> PyResult<()> {
    let bad = key.is_empty()
        || key.starts_with('/')
        || key.contains(['\\', '\0', ':'])
        || key.split('/').any(|c| c.is_empty() || c == "." || c == "..");
    if bad {
        return Err(invalid(format!("Invalid ZoneInfo key: {:?}", key)));
    }
    Ok(())
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

/// Per-`loads` resolved type (dataclass, namedtuple or enum).
struct RtType<'py> {
    /// Record tag this entry was resolved for.
    tag: Tag,
    cls: Bound<'py, PyAny>,
    names: Vec<Bound<'py, PyString>>,
    migrate: bool,
    current_version: u32,
    /// A caller-provided class (`classes=`) rather than a dynamic stand-in.
    real: bool,
    /// Real dataclass: list of fields absent from the data (filled from
    /// defaults). Real namedtuple with a different field list: `Some` (the
    /// serialized field names), so values are mapped by name.
    plan: Option<Bound<'py, PyAny>>,
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
        tag: Tag::Dataclass,
        cls,
        names,
        migrate: serialized > 0 && version > 0 && serialized != version,
        current_version: version,
        real: false,
        plan: None,
    })
}

fn class_version(cls: &Bound<'_, PyAny>) -> u32 {
    cls.getattr("__pysafe_pickle_version__")
        .or_else(|_| cls.getattr("__pygraph_version__"))
        .and_then(|v| v.extract())
        .unwrap_or(0)
}

/// Resolves type-table entry `td` for a record with `tag`, preferring a
/// caller-provided class from `classes` (name -> class).
fn resolve_rt<'py>(
    py: Python<'py>,
    td: &TypeDef<'_>,
    tag: Tag,
    classes: Option<&Bound<'py, PyDict>>,
) -> PyResult<RtType<'py>> {
    let provided = match classes {
        Some(m) => m.get_item(td.name)?,
        None => None,
    };
    let names = || -> Vec<Bound<'py, PyString>> { td.fields.iter().map(|f| PyString::intern(py, f)).collect() };
    let field_list = |names: &[Bound<'py, PyString>]| PyList::new(py, names);
    match (tag, provided) {
        (Tag::Dataclass, None) => resolve_type(py, td),
        (Tag::Dataclass, Some(cls)) => {
            let version = class_version(&cls);
            let serialized = td.schema_version;
            let migrate = serialized > 0 && version > 0 && serialized != version;
            let names = names();
            let missing = recon(py, "dataclass_plan")?.call1((&cls, td.name, field_list(&names)?, migrate))?;
            let plan = if missing.len()? > 0 { Some(missing) } else { None };
            Ok(RtType { tag, cls, names, migrate, current_version: version, real: true, plan })
        }
        (Tag::NamedTuple, provided) => {
            let names = names();
            let fields = field_list(&names)?;
            let (cls, real, plan) = match provided {
                Some(cls) => {
                    let same: bool = recon(py, "namedtuple_plan")?.call1((&cls, td.name, &fields))?.extract()?;
                    (cls, true, if same { None } else { Some(fields.into_any()) })
                }
                None => (recon(py, "make_namedtuple")?.call1((td.name, fields))?, false, None),
            };
            Ok(RtType { tag, cls, names, migrate: false, current_version: 0, real, plan })
        }
        (_, Some(cls)) => {
            recon(py, "check_enum_class")?.call1((&cls, td.name))?;
            Ok(RtType { tag, cls, names: Vec::new(), migrate: false, current_version: 0, real: true, plan: None })
        }
        (_, None) => Err(crate::errors::unsafe_type_error(format!(
            "Unsupported type: enum {} (pass it via classes=)",
            td.name
        ))),
    }
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
    /// Immutable value built from its children (collected on the value
    /// stack) once they are all available.
    Build(Build),
    /// OrderedDict / defaultdict / Counter (items set with `obj[k] = v`).
    Mapping(Bound<'py, PyAny>, Option<Bound<'py, PyAny>>),
    Deque(Bound<'py, PyAny>),
}

#[derive(Clone, Copy)]
enum Build {
    Fraction,
    Range,
    Slice,
    Enum { ty: usize, by_value: bool },
    NamedTuple { ty: usize },
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
/// (DateTime/Time reference a tzinfo record, which is itself always a leaf.)
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
            | Tag::Fraction
            | Tag::Range
            | Tag::Slice
            | Tag::OrderedDict
            | Tag::DefaultDict
            | Tag::Counter
            | Tag::Deque
            | Tag::Enum
            | Tag::NamedTuple
    )
}

struct Builder<'a, 'py> {
    py: Python<'py>,
    g: &'a DecodedGraph<'a>,
    memo: Vec<Option<Bound<'py, PyAny>>>,
    in_progress: Vec<bool>,
    str_cache: Vec<Option<Bound<'py, PyAny>>>,
    types: Vec<Option<RtType<'py>>>,
    /// Caller-trusted classes by name (`classes=`).
    classes: Option<&'a Bound<'py, PyDict>>,
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

#[cold]
#[inline(never)]
fn cyclic_immutable(id: usize) -> PyErr {
    crate::errors::decode_error(format!(
        "Cyclic reference involving immutable container at ref_id {} is not supported",
        id
    ))
}

impl<'a, 'py> Builder<'a, 'py> {
    /// The value of child record `cid` when it needs no frame of its own: an
    /// already-built shared record, or a leaf (memoized if shared). `None`
    /// means the caller must go through [`Self::resolve`]. Only valid when
    /// the child's depth is below `max_depth`.
    #[inline(always)]
    fn quick(&mut self, cid: usize) -> PyResult<Option<Bound<'py, PyAny>>> {
        let g = self.g;
        let r = g.recs[cid];
        if g.rec_refs[cid] >= SHARED {
            if let Some(obj) = &self.memo[cid] {
                return Ok(Some(obj.clone()));
            }
            if !is_leaf(r.tag) {
                return Ok(None);
            }
            let v = self.scalar(r)?;
            self.memo[cid] = Some(v.clone());
            Ok(Some(v))
        } else if is_leaf(r.tag) {
            Ok(Some(self.scalar(r)?))
        } else {
            Ok(None)
        }
    }

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

    #[inline]
    fn rt_type(&mut self, ty: usize, tag: Tag) -> PyResult<(Bound<'py, PyAny>, bool)> {
        match &self.types[ty] {
            Some(rt) if rt.tag == tag => Ok((rt.cls.clone(), rt.migrate)),
            _ => self.rt_type_slow(ty, tag),
        }
    }

    /// First use of type `ty` in this call (resolves it), or a kind mismatch.
    #[inline(never)]
    fn rt_type_slow(&mut self, ty: usize, tag: Tag) -> PyResult<(Bound<'py, PyAny>, bool)> {
        if self.types[ty].is_none() {
            self.types[ty] = Some(resolve_rt(self.py, &self.g.types[ty], tag, self.classes)?);
        }
        let rt = self.types[ty].as_ref().unwrap();
        if rt.tag != tag {
            return Err(invalid(format!(
                "Invalid data: type '{}' is used by records of different kinds",
                self.g.types[ty].name
            )));
        }
        Ok((rt.cls.clone(), rt.migrate))
    }

    /// The tzinfo of a DateTime/Time record (`None` when `id` is u32::MAX).
    fn tz_value(&mut self, id: u32) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        if id == u32::MAX {
            return Ok(py.None().into_bound(py));
        }
        let id = id as usize;
        let r = self.g.recs[id];
        if !matches!(r.tag, Tag::Timezone | Tag::ZoneInfo) {
            return Err(invalid("Invalid data: tzinfo must be a timezone or ZoneInfo record"));
        }
        let shared = self.g.rec_refs[id] >= SHARED;
        if shared {
            if let Some(v) = &self.memo[id] {
                return Ok(v.clone());
            }
        }
        let v = self.rare_scalar(r)?;
        if shared {
            self.memo[id] = Some(v.clone());
        }
        Ok(v)
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
            Tag::Ellipsis => py.Ellipsis().into_bound(py),
            Tag::NotImplemented => py.NotImplemented().into_bound(py),
            Tag::DateTime | Tag::Date | Tag::Time => {
                let p = &data[off..];
                let kwargs = PyDict::new(py);
                let (cls, args) = match r.tag {
                    Tag::Date => {
                        let year = u16::from_le_bytes([p[0], p[1]]);
                        let args = PyTuple::new(py, [year as u32, p[2] as u32, p[3] as u32])?;
                        (StdCtor::Date, args)
                    }
                    Tag::DateTime => {
                        let year = u16::from_le_bytes([p[0], p[1]]);
                        let us = u32_at(p, 7);
                        kwargs.set_item("fold", p[11])?;
                        let tz = self.tz_value(r.a)?;
                        let args = (year, p[2], p[3], p[4], p[5], p[6], us, tz).into_pyobject(py)?;
                        (StdCtor::DateTime, args)
                    }
                    _ => {
                        let us = u32_at(p, 3);
                        kwargs.set_item("fold", p[7])?;
                        let tz = self.tz_value(r.a)?;
                        let args = (p[0], p[1], p[2], us, tz).into_pyobject(py)?;
                        (StdCtor::Time, args)
                    }
                };
                std_ctor(py, cls)?.call(args, Some(&kwargs))?
            }
            Tag::TimeDelta => self.timedelta(off)?,
            Tag::Timezone => {
                let td = self.timedelta(off)?;
                let ctor = std_ctor(py, StdCtor::Timezone)?;
                if r.aux == 1 {
                    let start = off + 17;
                    let name = decode_utf8(py, &data[start..start + r.a as usize], Some(c"surrogatepass"))?;
                    ctor.call1((td, name))?
                } else {
                    ctor.call1((td,))?
                }
            }
            Tag::ZoneInfo => {
                let key = std::str::from_utf8(&data[off..off + r.a as usize])
                    .map_err(|_| invalid("Invalid ZoneInfo key: not UTF-8"))?;
                check_zone_key(key)?;
                std_ctor(py, StdCtor::ZoneInfo)?.call1((key,))?
            }
            Tag::Decimal => {
                let s = std::str::from_utf8(&data[off..off + r.a as usize])
                    .ok()
                    .filter(|s| s.is_ascii())
                    .ok_or_else(|| invalid("Invalid Decimal payload"))?;
                std_ctor(py, StdCtor::Decimal)?.call1((s,))?
            }
            Tag::Uuid => {
                let kwargs = PyDict::new(py);
                kwargs.set_item("bytes", PyBytes::new(py, &data[off..off + 16]))?;
                std_ctor(py, StdCtor::Uuid)?.call((), Some(&kwargs))?
            }
            Tag::Path => {
                // Concrete flavors only load as concrete paths on a matching OS.
                let ctor = match (r.aux, cfg!(windows)) {
                    (0, _) | (2, true) => StdCtor::PurePosixPath,
                    (1, _) | (3, false) => StdCtor::PureWindowsPath,
                    (2, false) => StdCtor::PosixPath,
                    _ => StdCtor::WindowsPath,
                };
                let s = decode_utf8(py, &data[off..off + r.a as usize], Some(c"surrogatepass"))?;
                std_ctor(py, ctor)?.call1((s,))?
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "internal error: non-leaf record materialized as scalar",
                ))
            }
        })
    }

    fn timedelta(&self, off: usize) -> PyResult<Bound<'py, PyAny>> {
        let d = self.g.data;
        let days = u32_at(d, off) as i32;
        std_ctor(self.py, StdCtor::TimeDelta)?.call1((days, u32_at(d, off + 4), u32_at(d, off + 8)))
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
                tag if is_leaf(tag) => self.scalar(r)?,
                _ => {
                    let total = if r.tag == Tag::Dict { r.a as usize * 2 } else { r.a as usize };
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
                        Tag::Dataclass => {
                            // Create the instance first (pickle semantics,
                            // `__init__` is not called) so that
                            // self-references resolve to it.
                            let ty = r.aux as usize;
                            let (cls, migrate) = self.rt_type(ty, Tag::Dataclass)?;
                            let obj = object_new_fn(py)?.call1((cls,))?;
                            if shared {
                                self.memo[id] = Some(obj.clone());
                            }
                            Kind::Dataclass { obj, ty, migrate }
                        }
                        _ => return self.resolve_ext(r, id, shared, depth),
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

    /// [`Self::resolve`] for the container records with tags >= 0x13 (kept
    /// out of line so the hot path for the core tags stays small).
    #[inline(never)]
    fn resolve_ext(&mut self, r: Rec, id: usize, shared: bool, depth: usize) -> PyResult<Step<'py>> {
        let py = self.py;
        let g = self.g;
        let mut off = r.b as usize;
        let total = match r.tag {
            Tag::OrderedDict | Tag::DefaultDict | Tag::Counter => r.a as usize * 2,
            Tag::Enum => 1,
            _ => r.a as usize,
        };
        let kind = match r.tag {
            Tag::Fraction | Tag::Range | Tag::Slice | Tag::Enum | Tag::NamedTuple => {
                let build = match r.tag {
                    Tag::Fraction => Build::Fraction,
                    Tag::Range => Build::Range,
                    Tag::Slice => Build::Slice,
                    Tag::Enum => {
                        let ty = r.aux as usize;
                        self.rt_type(ty, Tag::Enum)?;
                        Build::Enum { ty, by_value: r.a == 1 }
                    }
                    _ => {
                        let ty = r.aux as usize;
                        self.rt_type(ty, Tag::NamedTuple)?;
                        Build::NamedTuple { ty }
                    }
                };
                if shared {
                    self.in_progress[id] = true;
                }
                Kind::Build(build)
            }
            Tag::OrderedDict | Tag::DefaultDict | Tag::Counter => {
                let obj = match r.tag {
                    Tag::OrderedDict => std_ctor(py, StdCtor::OrderedDict)?.call0()?,
                    Tag::Counter => std_ctor(py, StdCtor::Counter)?.call0()?,
                    _ => {
                        let factory = match DEFAULT_FACTORIES[r.aux as usize] {
                            "" => py.None().into_bound(py),
                            name => py.import("builtins")?.getattr(name)?,
                        };
                        std_ctor(py, StdCtor::DefaultDict)?.call1((factory,))?
                    }
                };
                if shared {
                    self.memo[id] = Some(obj.clone());
                }
                Kind::Mapping(obj, None)
            }
            Tag::Deque => {
                let enc = u64::from_le_bytes(g.data[off..off + 8].try_into().unwrap());
                off += 12;
                let maxlen = if enc == 0 {
                    py.None().into_bound(py)
                } else if enc - 1 <= isize::MAX as u64 {
                    (enc - 1).into_pyobject(py)?.into_any()
                } else {
                    return Err(invalid("Invalid data: deque maxlen out of range"));
                };
                let obj = std_ctor(py, StdCtor::Deque)?.call1((PyTuple::empty(py), maxlen))?;
                if shared {
                    self.memo[id] = Some(obj.clone());
                }
                Kind::Deque(obj)
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "internal error: leaf record resolved as container",
                ))
            }
        };
        Ok(Step::Push(Frame {
            id: id as u32,
            kind,
            off,
            next: 0,
            total,
            base: self.values.len(),
            depth,
        }))
    }

    /// Hands a finished child value to its parent frame.
    #[inline]
    fn deliver(&mut self, frame: &mut Frame<'py>, v: Bound<'py, PyAny>) -> PyResult<()> {
        let child_idx = frame.next - 1;
        match &mut frame.kind {
            Kind::ListEarly(l) => l.append(v),
            Kind::ListLate | Kind::Tuple | Kind::FrozenSet | Kind::Build(_) => {
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
            Kind::Mapping(..) | Kind::Deque(_) => deliver_ext(&mut frame.kind, v),
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
            Kind::Mapping(m, _) | Kind::Deque(m) => Ok(m),
            Kind::Build(build) => {
                let obj = self.build(build, frame.base)?;
                if self.g.rec_refs[id] >= SHARED {
                    self.in_progress[id] = false;
                    self.memo[id] = Some(obj.clone());
                }
                Ok(obj)
            }
            Kind::Dataclass { obj, ty, migrate } => {
                if migrate {
                    self.finish_migration(&obj, ty, frame.base)?;
                } else {
                    // Fields missing from the record default to None
                    // (matches the old `cls(**kwargs)` behaviour).
                    let rt = self.types[ty].as_ref().unwrap();
                    if frame.total < rt.names.len() {
                        let none = py.None().into_bound(py);
                        for name in &rt.names[frame.total..] {
                            set_attr(&obj, name, &none)?;
                        }
                    }
                    // Real class: fields absent from the data get their defaults.
                    if let Some(missing) = &rt.plan {
                        recon(py, "fill_defaults")?.call1((&obj, &rt.cls, missing))?;
                    }
                }
                Ok(obj)
            }
        }
    }

    /// Builds an immutable value from the children collected at `base..`.
    #[inline(never)]
    fn build(&mut self, build: Build, base: usize) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let vals: Vec<Bound<'py, PyAny>> = self.values.drain(base..).collect();
        let all_ints = || vals.iter().all(|v| v.is_exact_instance_of::<PyInt>());
        match build {
            Build::Fraction => {
                if !all_ints() || vals[1].eq(0)? {
                    return Err(invalid("Invalid data: bad Fraction record"));
                }
                std_ctor(py, StdCtor::Fraction)?.call1((&vals[0], &vals[1]))
            }
            Build::Range => {
                if !all_ints() {
                    return Err(invalid("Invalid data: bad range record"));
                }
                py.import("builtins")?.getattr("range")?.call1((&vals[0], &vals[1], &vals[2]))
            }
            Build::Slice => py.get_type::<PySlice>().call1((&vals[0], &vals[1], &vals[2])),
            Build::Enum { ty, by_value } => {
                let rt = self.types[ty].as_ref().unwrap();
                recon(py, "enum_member")?.call1((&rt.cls, self.g.types[ty].name, by_value, &vals[0]))
            }
            Build::NamedTuple { ty } => {
                let rt = self.types[ty].as_ref().unwrap();
                let values = PyTuple::new(py, vals)?;
                match &rt.plan {
                    None => rt.cls.call_method1(pyo3::intern!(py, "_make"), (values,)),
                    Some(fields) => recon(py, "build_namedtuple")?
                        .call1((&rt.cls, self.g.types[ty].name, fields, values)),
                }
            }
        }
    }

    /// Schema-migration path: runs `apply_migrations` on the serialized state
    /// and sets the migrated fields (and any extra keys) on `obj`.
    #[inline(never)]
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

        if rt.real {
            // Real class: set every field (defaults for absent ones); extra
            // keys left over by the migrations are an error.
            recon(py, "apply_state")?.call1((obj, &rt.cls, td.name, migrated))?;
            return Ok(());
        }

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

/// [`Builder::deliver`] for OrderedDict / defaultdict / Counter / deque frames.
#[inline(never)]
fn deliver_ext<'py>(kind: &mut Kind<'py>, v: Bound<'py, PyAny>) -> PyResult<()> {
    match kind {
        Kind::Mapping(m, pending) => match pending.take() {
            None => {
                *pending = Some(v);
                Ok(())
            }
            Some(k) => m.set_item(k, v),
        },
        Kind::Deque(d) => d.call_method1(pyo3::intern!(d.py(), "append"), (v,)).map(|_| ()),
        _ => Err(pyo3::exceptions::PyValueError::new_err(
            "internal error: unexpected frame kind",
        )),
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

/// Normalizes the `classes=` argument (an iterable of classes or a mapping
/// name -> class) into a dict, validating it.
pub fn class_map<'py>(classes: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let m = recon(classes.py(), "class_map")?.call1((classes,))?;
    Ok(m.downcast_into::<PyDict>()?)
}

/// Phase-2 buffers kept per thread between calls (see [`Scratch`]). They are
/// always stored empty, so no Python object outlives a call; `'static` is
/// only a placeholder lifetime.
struct Scratch2 {
    values: Vec<Bound<'static, PyAny>>,
    frames: Vec<Frame<'static>>,
    memo: Vec<Option<Bound<'static, PyAny>>>,
    in_progress: Vec<bool>,
    str_cache: Vec<Option<Bound<'static, PyAny>>>,
    types: Vec<Option<RtType<'static>>>,
}

impl Scratch2 {
    const EMPTY: Scratch2 = Scratch2 {
        values: Vec::new(),
        frames: Vec::new(),
        memo: Vec::new(),
        in_progress: Vec::new(),
        str_cache: Vec::new(),
        types: Vec::new(),
    };
}

/// Materializes the root record (record 0) of a decoded graph.
///
/// `classes` maps type names to caller-trusted classes (see [`class_map`]).
pub fn reconstruct<'py>(
    py: Python<'py>,
    g: &DecodedGraph<'_>,
    classes: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    let n = g.recs.len();
    if n == 0 {
        return Err(DecodeError::InvalidReference(0).into());
    }
    // SAFETY: plain C call with the GIL held; returns the current limit.
    let limit = unsafe { pyo3::ffi::Py_GetRecursionLimit() };
    // `memo`/`in_progress` are only ever indexed for shared records and
    // `str_cache` only for shared strings, so skip filling them when nothing
    // is shared.
    let any_shared = g.rec_refs.iter().any(|&c| c >= SHARED);
    let any_shared_str = g.str_refs.iter().any(|&c| c >= SHARED);
    let s = SCRATCH2.try_with(|c| c.replace(Scratch2::EMPTY)).unwrap_or(Scratch2::EMPTY);
    let mut memo: Vec<Option<Bound<'py, PyAny>>> = recycle(s.memo);
    if any_shared {
        memo.resize_with(n, || None);
    }
    let mut in_progress = s.in_progress;
    in_progress.clear();
    if any_shared {
        in_progress.resize(n, false);
    }
    let mut str_cache: Vec<Option<Bound<'py, PyAny>>> = recycle(s.str_cache);
    if any_shared_str {
        str_cache.resize_with(g.strings.len(), || None);
    }
    let mut types: Vec<Option<RtType<'py>>> = recycle(s.types);
    types.resize_with(g.types.len(), || None);
    let mut values: Vec<Bound<'py, PyAny>> = recycle(s.values);
    values.reserve(n.min(64));
    let mut b = Builder {
        py,
        g,
        memo,
        in_progress,
        str_cache,
        types,
        classes,
        values,
        max_depth: if limit > 0 { limit as usize } else { 1000 },
    };
    let mut frames: Vec<Frame<'py>> = recycle(s.frames);
    let result = drive(&mut b, &mut frames);
    let s = Scratch2 {
        values: recycle(b.values),
        frames: recycle(frames),
        memo: recycle(b.memo),
        in_progress: recycle(b.in_progress),
        str_cache: recycle(b.str_cache),
        types: recycle(b.types),
    };
    let _ = SCRATCH2.try_with(|c| c.set(s));
    result
}

/// The reconstruction loop of [`reconstruct`] (explicit frame stack).
fn drive<'py>(b: &mut Builder<'_, 'py>, frames: &mut Vec<Frame<'py>>) -> PyResult<Bound<'py, PyAny>> {
    let g = b.g;
    match b.resolve(0, 0)? {
        Step::Value(v) => return Ok(v),
        Step::Push(f) => frames.push(f),
    }

    loop {
        let top = frames.last_mut().expect("frame stack is non-empty");
        let depth = top.depth + 1;
        let mut child_frame = None;
        // Sequence frames collecting onto the value stack: tight loop for
        // leaf children.
        if depth < b.max_depth {
            match &mut top.kind {
                Kind::ListLate | Kind::Tuple | Kind::FrozenSet | Kind::Build(_) => {
                    let ids = &g.data[top.off..top.off + 4 * top.total];
                    while top.next < top.total {
                        let o = 4 * top.next;
                        let cid = u32::from_le_bytes([ids[o], ids[o + 1], ids[o + 2], ids[o + 3]]) as usize;
                        match b.quick(cid)? {
                            Some(v) => b.values.push(v),
                            None => break,
                        }
                        top.next += 1;
                    }
                }
                // Dicts: whole (key, value) pairs inserted directly.
                Kind::Dict(d, pending) if pending.is_none() => {
                    let ids = &g.data[top.off..top.off + 4 * top.total];
                    while top.next + 1 < top.total {
                        let o = 4 * top.next;
                        let kid = u32::from_le_bytes([ids[o], ids[o + 1], ids[o + 2], ids[o + 3]]) as usize;
                        let Some(k) = b.quick(kid)? else { break };
                        let vid = u32::from_le_bytes([ids[o + 4], ids[o + 5], ids[o + 6], ids[o + 7]]) as usize;
                        match b.quick(vid)? {
                            Some(v) => {
                                top.next += 2;
                                d.set_item(k, v)?;
                            }
                            None => {
                                top.next += 1;
                                *pending = Some(k);
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        while top.next < top.total {
            let cid = u32_at(g.data, top.off + 4 * top.next) as usize;
            top.next += 1;
            // Fast paths: an already-built shared record, or an unshared leaf
            // (the common cases) skip the general resolver.
            if depth < b.max_depth {
                if let Some(v) = b.quick(cid)? {
                    b.deliver(top, v)?;
                    continue;
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
