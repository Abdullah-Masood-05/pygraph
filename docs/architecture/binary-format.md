# Binary Format Specification

`pysafe-pickle` uses a compact, hand-written binary format designed for high-speed serialization without invoking arbitrary execution hooks. All multi-byte fixed-width integers are little-endian.

---

## Header Layout

```text
[4 bytes]  Magic: b"PSPK" (written) - the decoder also accepts b"PYGR" (v1.0.x legacy)
[2 bytes]  Format Version: 1 (u16 LE)
[4 bytes]  Schema Version: 0 (u32 LE) - reserved, always 0 in the header
[1 byte ]  Flags (u8) - bit 0: HMAC-signed, bit 1: compressed (reserved, never set)
[4 bytes]  String Table Count (u32 LE)
  For each string:
    [4 bytes]  Length (u32 LE)
    [N bytes]  UTF-8 bytes
[4 bytes]  Type Table Count (u32 LE)
  For each type:
    [2 bytes]  Type ID (u16 LE)
    [4 bytes]  Name string index (u32 LE)
    [4 bytes]  Schema version (u32 LE)
    [2 bytes]  Field count (u16 LE)
      For each field:
        [4 bytes]  Field name string index (u32 LE)
[4 bytes]  Record Count (u32 LE)
  For each record:
    [1 byte]   Tag byte
    [variable] Payload
[32 bytes] HMAC-SHA256 tag - only if flags bit 0 is set (see below)
```

The fixed header is therefore 11 bytes (4 + 2 + 4 + 1). The decoder does not act on the format version or the header schema version. Of the flags, only bit 0 (HMAC) is used.

Notes on the tables:

- **Type ID** is the entry's position in the type table (0, 1, 2, ...). The decoder uses the position, so the stored ID is informational.
- **Schema version** (per type) comes from the class attribute `__pysafe_pickle_version__` (or the legacy `__pygraph_version__`), defaulting to `0`. It is only meaningful for dataclass types; enum and namedtuple entries store `0`.
- **Fields** are the dataclass fields, the namedtuple's `_fields`, or empty for an enum type.
- Type and field names are stored as indices into the string table, so they share entries with ordinary string values.

---

## HMAC Trailer

A payload written with `dumps(obj, hmac_key=key)` is the unsigned payload with two changes:

1. Bit 0 of the flags byte (offset 10) is set.
2. A 32-byte tag is appended: `HMAC-SHA256(key, every preceding byte)`, i.e. over the header (with the flag already set), tables and records.

```text
| header (flags |= 0x01) | string table | type table | records | tag (32 bytes) |
|<------------------- HMAC-SHA256 input ------------------------>|
```

The tag is plain HMAC-SHA256, so it can be checked with the standard library:

```python
import hashlib
import hmac
import pysafe_pickle as psp

blob = psp.dumps({"k": [1, 2, 3]}, hmac_key=b"key")
body, tag = blob[:-32], blob[-32:]
print(body[10] & 1, hmac.compare_digest(tag, hmac.new(b"key", body, hashlib.sha256).digest()))  # 1 True
```

Verification on load (`integrity.rs`):

- With a key and the flag set, the decoder first assumes the tag is the last 32 bytes of the buffer and verifies it (constant-time) before parsing anything beyond the header. If that fails (for example because more records follow in a stream), it runs the bounds-checked structural scan to find the end of the records and verifies the tag found there. No Python object is created before the tag is verified.
- Flag set, no key: `HMACError: payload is signed; pass hmac_key`.
- Flag clear, key given: `HMACError: payload is not signed`. Clearing the flag and dropping the tag therefore does not downgrade a signed payload.
- An empty key is rejected with `ValueError`.

Decoders older than 1.4.0 ignore the flags byte and any trailing bytes, so they load a signed payload **without** verifying it.

The format has no total-length field. A payload ends after its last record (plus the tag if signed), which is how `Unpickler` and `load(file)` find record boundaries in a multi-record stream: records are simply concatenated.

---

## Records and Record IDs

- A **record ID** is the zero-based position of a record in the record section. Container records refer to their children by record ID (`u32 LE`).
- **The root object is always record 0.**
- The encoder assigns IDs in **breadth-first** order: the root first, then each record's not-yet-seen children receive the next free IDs when their parent is written. A first-time child therefore always has a higher ID than its parent.
- **Shared and cyclic objects are not written as separate `Reference` records.** The parent simply stores the record ID of the object's existing record, so a cycle appears as a child ID that points back to a lower (or equal) ID. For example `l = []; l.append(l)` is a single `List` record whose only child is record 0.
- The decoder still accepts the legacy `Reference` tag (`0x0E`, written by v1.0-1.2) and follows it to its target, but the current encoder never emits it.

---

## Tag Definitions (38 tags)

| Tag | Byte | Payload Structure |
| :--- | :---: | :--- |
| `None` | `0x01` | No payload |
| `True` | `0x02` | No payload |
| `False` | `0x03` | No payload |
| `Int` | `0x04` | Zigzag + LEB128 varint of a signed 64-bit integer (1-10 bytes) |
| `Float` | `0x05` | 8-byte IEEE 754 little-endian `f64` |
| `String` | `0x06` | String table index (u32 LE) |
| `Bytes` | `0x07` | Length (u32 LE) + raw bytes |
| `List` | `0x08` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Tuple` | `0x09` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Dict` | `0x0A` | Pair count (u32 LE) + (key ID, value ID) pairs (u32 LE each, 8 bytes per pair) |
| `Set` | `0x0B` | Count (u32 LE) + child record IDs (u32 LE each) |
| `FrozenSet` | `0x0C` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Dataclass` | `0x0D` | Type ID (u16 LE) + field count (u16 LE) + field record IDs (u32 LE each, in type-table field order) |
| `Reference` | `0x0E` | Record ID (u32 LE). **Legacy**: decoded but no longer written |
| `BigInt` | `0x0F` | Length (u32 LE) + signed two's-complement little-endian bytes. Only for ints outside the `i64` range; length is `(bit_length + 8) // 8` |
| `ByteArray` | `0x10` | Length (u32 LE) + raw bytes |
| `Complex` | `0x11` | 16 bytes: real `f64` LE followed by imaginary `f64` LE |
| `StrRaw` | `0x12` | Length (u32 LE) + UTF-8 bytes encoded with `surrogatepass`. Only for strings that are not valid strict UTF-8 (lone surrogates) |

Tags `0x13` to `0x26` were added in v1.4.0. In the table below, "id" is a u32 LE record ID, "blob" is a u32 LE length followed by that many bytes, and all integers are little-endian.

| Tag | Byte | Payload Structure |
| :--- | :---: | :--- |
| `DateTime` | `0x13` | year u16, month u8, day u8, hour u8, minute u8, second u8, microsecond u32, fold u8, tzinfo id (u32; `0xFFFFFFFF` = naive, otherwise a `Timezone` or `ZoneInfo` record). 16 bytes |
| `Date` | `0x14` | year u16, month u8, day u8 |
| `Time` | `0x15` | hour u8, minute u8, second u8, microsecond u32, fold u8, tzinfo id (as for `DateTime`). 12 bytes |
| `TimeDelta` | `0x16` | days i32, seconds u32, microseconds u32 |
| `Timezone` | `0x17` | offset as a `TimeDelta` payload (12 bytes), has_name u8, then (if has_name) the name as a UTF-8 (`surrogatepass`) blob |
| `ZoneInfo` | `0x18` | key as a UTF-8 blob, rebuilt with `ZoneInfo(key)`. Keys that are empty, absolute, or contain `\`, `:`, NUL, empty components, `.` or `..` are rejected |
| `Decimal` | `0x19` | `str(d)` as an ASCII blob (covers `NaN`, `sNaN`, `Infinity`, `-0`) |
| `Uuid` | `0x1A` | 16 raw bytes (`UUID.bytes`) |
| `Fraction` | `0x1B` | numerator id, denominator id (both int records) |
| `Range` | `0x1C` | start id, stop id, step id (all int records) |
| `Slice` | `0x1D` | start id, stop id, step id (any records) |
| `Ellipsis` | `0x1E` | No payload |
| `NotImplemented` | `0x1F` | No payload |
| `Path` | `0x20` | flavor u8 (0 `PurePosixPath`, 1 `PureWindowsPath`, 2 `PosixPath`, 3 `WindowsPath`), then `str(path)` as a UTF-8 (`surrogatepass`) blob. A concrete flavor loads as the concrete class when it matches the running OS, otherwise as the matching `Pure*` class |
| `OrderedDict` | `0x21` | pair count u32, then (key id, value id) pairs in iteration order |
| `DefaultDict` | `0x22` | factory u8, pair count u32, then (key id, value id) pairs. Factory codes: 0 `None`, 1 `list`, 2 `dict`, 3 `set`, 4 `int`, 5 `float`, 6 `str`, 7 `tuple`, 8 `bytes`, 9 `frozenset`, 10 `bool`, 11 `complex` |
| `Counter` | `0x23` | pair count u32, then (key id, value id) pairs |
| `Deque` | `0x24` | maxlen u64 (0 = `None`, otherwise maxlen + 1), item count u32, then item ids |
| `Enum` | `0x25` | type id u16 (type-table entry with no fields), mode u8 (0 = by name: the child is a `String` record with the member name; 1 = by value), child id |
| `NamedTuple` | `0x26` | type id u16 (type-table entry whose fields are `_fields`), item count u32, then item ids |

Any other tag byte makes the decoder raise `DecodeError: Unknown tag: 0x..`.

Enum members are written by name when `Cls[member.name] is member`, and by value otherwise (for example composite `Flag` values such as `Perm.R | Perm.W`).

---

## Integer Encoding

`Int` values use zigzag mapping followed by an unsigned LEB128 varint (7 data bits per byte, low group first, high bit = "more bytes follow"):

```text
zigzag(v) = (v << 1) ^ (v >> 63)        # arithmetic shift, i64 -> u64
```

| Value | Zigzag | Encoded bytes |
| :--- | :--- | :--- |
| `0` | 0 | `00` |
| `-1` | 1 | `01` |
| `1` | 2 | `02` |
| `36` | 72 | `48` |
| `64` | 128 | `80 01` |
| `300` | 600 | `D8 04` |

The full `i64` range fits in at most 10 bytes; a varint that continues past 64 bits is rejected. Python ints outside `i64` are written as `BigInt` instead.

---

## String Interning

String values, string dictionary keys, type names and field names all live in a deduplicated string table at the front of the payload. A `String` record stores only the table index. This dramatically reduces payload sizes for tabular datasets and dataclasses with repeating field keys.

Deduplication rules in the encoder:

- **Strings shorter than 4096 bytes** are interned by value: equal contents share one string-table entry *and* one `String` record, so every reference to `"a"` points at the same record ID.
- **Strings of 4096 bytes or more** are not hashed. Each distinct string object gets its own string-table entry and record, deduplicated by object identity only.
- **Lone-surrogate strings** are written as `StrRaw` records (not in the string table) and are deduplicated by object identity.
- **Scalars:** one record each for `None`, `True` and `False`. `int` and `float` values go through a small direct-mapped cache (floats keyed by bit pattern, so `0.0` and `-0.0` stay distinct); a cache miss just emits another identical record, which is harmless.
- **Everything else** (lists, dicts, `bytes`, dataclass instances, ...) is deduplicated by object identity, which is what preserves sharing and cycles.

---

## Type Table

Each distinct dataclass, namedtuple or enum type used in the object graph gets one entry (name, schema version, ordered field names). Entries are shared only between classes with the **same name and the same field list**; two classes that merely share a `__name__` but have different fields receive distinct type IDs. Fields are those reported by `dataclasses.fields()`, so `ClassVar` and `InitVar` pseudo-fields are excluded. At most 65535 types, and 65535 fields per type, can be encoded.

---

## Worked Examples

`psp.dumps([1, "a", "a"])` produces 52 bytes:

```text
50 53 50 4b              magic "PSPK"
01 00                    format version 1
00 00 00 00              schema version 0
00                       flags 0
01 00 00 00              string count = 1
  01 00 00 00 61         string 0: length 1, "a"
00 00 00 00              type count = 0
03 00 00 00              record count = 3
  08 03 00 00 00         record 0: List, 3 children
     01 00 00 00           -> record 1
     02 00 00 00           -> record 2
     02 00 00 00           -> record 2 (shared String record)
  04 02                  record 1: Int, zigzag varint 0x02 = 1
  06 00 00 00 00         record 2: String, string index 0 ("a")
```

`psp.dumps(None)` is 24 bytes: the 11-byte header, a string count of `0`, a type count of `0`, a record count of `1`, and the single record byte `01`.

A self-referencing list (`l = []; l.append(l)`) is 32 bytes (11 header + 4 + 4 + 4 + 9) and its only record is `08 01 00 00 00 00 00 00 00` (List, one child, record 0).

`psp.dumps(datetime.date(2026, 10, 28))` is 28 bytes, and its only record is `14 ea 07 0a 1c` (Date, year `0x07ea` = 2026, month 10, day 28).

`psp.dumps(None, hmac_key=b"key")` is 56 bytes: the 24-byte unsigned payload with the flags byte (offset 10) set to `01`, followed by the 32-byte tag.

For `class Color(enum.Enum): RED = 1`, `psp.dumps(Color.RED)` (64 bytes) has strings `Color, RED`, one type entry (`name=0, version=0, fields=[]`) and two records: `25 00 00 00 01 00 00 00` (Enum, type 0, by name, child record 1) and `06 01 00 00 00` (String 1, `"RED"`).

A dataclass `P(x=1, y="hi")` (84 bytes) has strings `P, x, y, hi`, one type entry (`name=0, version=0, fields=[1, 2]`) and three records: `0d 00 00 02 00 01 00 00 00 02 00 00 00` (Dataclass, type 0, 2 fields -> records 1 and 2), `04 02` (Int 1) and `06 03 00 00 00` (String 3, `"hi"`).

---

## Decoder Safety Bounds

The decoder validates the entire payload once before building any Python object. Structural failures raise `DecodeError` (a `ValueError`), except as noted:

- **Count vs. remaining bytes.** Every declared count is checked against what the remaining input could possibly hold before anything is allocated: strings (at least 4 bytes each), types (at least 12 bytes each), fields per type and per dataclass (4 bytes each), records (at least 1 byte each), list/tuple/set/frozenset items (4 bytes each) and dict pairs (8 bytes each). A forged count such as `0xFFFFFFFF` fails fast with `Invalid ... count ... exceeds maximum possible elements`.
- **Length prefixes.** String, bytes, bytearray, bigint and raw-string lengths must fit in the remaining input; `Complex` needs exactly 16 bytes.
- **Index validation.** Every child record ID, string index and dataclass type ID must be in range (`Invalid reference`, `Invalid string index`, `Invalid type_id`). Magic bytes and tag bytes are checked as well.
- **UTF-8.** Type and field names are validated during decoding; string values are decoded strictly when materialized (`UnicodeDecodeError`, a `ValueError` subclass). `StrRaw` is decoded with `surrogatepass`.
- **Depth limit.** Reconstruction uses an explicit frame stack (no native recursion) and is limited to the interpreter recursion limit (`sys.getrecursionlimit()`); deeper nesting raises `RecursionError`. The encoder applies the same limit.
- **Value checks.** Dates, times, timezones and `ZoneInfo` keys are validated when built (invalid values raise `ValueError`); a `DateTime`/`Time` tzinfo id must point at a `Timezone` or `ZoneInfo` record; `Fraction`/`Range` children must be ints; `DefaultDict` factory codes and `Path` flavors must be known.
- **No code execution.** Tags produce built-in and stdlib values, generated dataclass-like or namedtuple classes, or classes the caller passed in `classes=`. Nothing in the payload can name an importable callable; type names are only looked up in the caller's `classes=` mapping. An enum type that is not in `classes=` raises `UnsafeTypeError`.

Reconstruction semantics for shared data: a record that is referenced more than once and is a list, dict, set, `OrderedDict`, `defaultdict`, `Counter`, `deque` or dataclass instance is created and memoized *before* its children are filled in, so arbitrary DAGs and cycles round-trip. A cycle that passes through an immutable value (`tuple`, `frozenset`, namedtuple, `slice`, ...) cannot be rebuilt and raises `DecodeError` (`Cyclic reference involving immutable container ...`).
