# pysafe-pickle Codebase Context

## Overview

**pysafe-pickle** (PyPI: `pysafe-pickle`, import: `pysafe_pickle`, legacy import: `pygraph`) is a safe, fast, schema-evolvable Python object graph serialization library powered by Rust via PyO3. It provides a pickle-style API (`dumps`/`loads`/`dump`/`load`, `Pickler`/`Unpickler`) with zero arbitrary code execution on load, optional HMAC-SHA256 signing, real classes on load via `classes=`, automatic schema migrations, and a safe reader for existing standard pickle files.

**Version:** 1.4.0
**Repository:** `Abdullah-Masood-05/pygraph` (GitHub)
**License:** AGPL-3.0-only
**Python:** >=3.10 (tested 3.10 through 3.15; one abi3 wheel covers all, free-threaded builds not supported)
**Rust:** Edition 2021

---

## Architecture

```
pygraph/
├── src/                          # Rust core (compiled to _pysafe_pickle.so/.pyd)
│   ├── lib.rs                    # PyO3 module: dumps/loads/dump/load/_loads_prefix, GIL release, file positioning
│   ├── errors.rs                 # Raises UnsafeTypeError/DecodeError/HMACError (imported from pysafe_pickle.exceptions)
│   ├── integrity.rs              # HMAC-SHA256 sign/verify, decode_one (frames one payload, verifies tag first)
│   ├── graph/
│   │   ├── mod.rs
│   │   └── traversal.rs          # Walker: BFS traversal, memo table, type dispatch (dataclass/enum/namedtuple/stdlib)
│   └── format/
│       ├── mod.rs                # Tag enum (38 tags), header/flags, defaultdict factory codes, varint helpers
│       ├── encoder.rs            # RecordWriter: streaming byte-level writer (no intermediate tree)
│       └── decoder.rs            # Two-phase decoder: validate+index (Rust), reconstruct (GIL), classes=, migrations
├── python/pysafe_pickle/         # Primary Python package
│   ├── __init__.py               # Public API exports (dumps, loads, migrate, exceptions, load_pickle, etc.)
│   ├── _pysafe_pickle.pyi        # Type stubs for the Rust module
│   ├── _reconstruct.py           # make_class, generated namedtuples, classes= validation, defaults, enum lookup
│   ├── pickler.py                # Pickler/Unpickler (multi-record, read-ahead buffer)/PickleBuffer
│   ├── migrations.py             # @migrate decorator, migration chain resolution, apply_migrations
│   ├── exceptions.py             # PySafePickleError, UnsafeTypeError, DecodeError, HMACError, SchemaVersionError
│   ├── compat.py                 # Safe reader for standard pickle (load_pickle/loads_pickle/convert_pickle, SAFE_GLOBALS)
│   └── py.typed                  # PEP 561 marker
├── python/pygraph/               # Deprecated compatibility shim (re-exports pysafe_pickle)
│   ├── __init__.py               # FutureWarning on import, re-exports all public API
│   └── __init__.pyi              # Type stubs (re-exports pysafe_pickle)
├── tests/
│   ├── test_roundtrip.py         # 48 tests: primitives, containers, cycles, dataclasses, allowlist
│   ├── test_pickle_comparison.py # 41 tests: pysafe-pickle vs pickle equivalence + security tests
│   ├── test_migrations.py        # 12 tests: migration decorator, chain resolution, schema versioning
│   ├── test_accuracy.py          # 149 tests: exhaustive correctness (parametrized edge cases)
│   ├── test_cross_version.py     # 6 tests: cross-version deserialization with fixture binaries
│   ├── test_security_bounds.py   # 8 tests: malformed input, bounds checking, tag validation
│   ├── test_integrity_and_streams.py # 287 tests: HMAC wire format/tampering, exception hierarchy, multi-record streams
│   ├── test_types_and_classes.py # 117 tests: stdlib types, enums, namedtuples, classes=, automatic migrations
│   ├── test_pickle_compat.py     # 309 tests: safe pickle reader (round trips, malicious payloads, resource bounds)
│   └── fixtures/                 # Binary test fixtures (e.g., v101_user_v2.bin)
├── benchmarks/
│   ├── bench_data.py             # 16 benchmark data shapes (primitives, nested, wide, dataclasses, etc.)
│   └── test_benchmarks.py        # 160 benchmarks: dumps/loads/roundtrip/size vs pickle (3 serializers)
├── docs/                         # Documentation site (VitePress)
├── Cargo.toml                    # Rust deps: pyo3 0.24 (abi3-py310), rustc-hash 2, hmac, sha2
├── pyproject.toml                # Build: maturin, Python metadata, optional deps
└── .github/workflows/
    ├── ci.yml                    # Multi-OS x Python 3.10-3.15 CI
    ├── publish.yml               # Tag-triggered PyPI publish + GitHub Release with wheels
    └── deploy-docs.yml           # Documentation site deployment
```

---

## Binary Format

The custom binary format (`PSPK` magic header, with legacy `PYGR` accepted on read):

```
[4 bytes]  Magic: b"PSPK" (legacy: b"PYGR")
[2 bytes]  Format version: 1 (u16 LE)
[4 bytes]  Schema version: 0 (u32 LE)
[1 byte]   Flags: bit 0 = HMAC-signed, bit 1 = compressed (reserved, never set)
[4 bytes]  String table count (u32 LE)
  For each string:
    [4 bytes]  Length (u32 LE)
    [N bytes]  UTF-8 bytes
[4 bytes]  Type table count (u32 LE)
  For each type:
    [2 bytes]  Type ID (u16 LE)
    [4 bytes]  Name string index (u32 LE)
    [4 bytes]  Schema version (u32 LE)
    [2 bytes]  Field count (u16 LE)
      For each field:
        [4 bytes]  Field name string index (u32 LE)
[4 bytes]  Record count (u32 LE)
  For each record:
    [1 byte]   Tag byte (see below)
    [variable] Payload (depends on tag)
[32 bytes] HMAC-SHA256 tag over all preceding bytes (only if flags bit 0 is set)
```

A multi-record stream is just concatenated payloads (there is no length field; a payload ends after its last record, plus the tag if signed).

**Tags (38 types):**
| Tag | Byte | Payload |
|-----|------|---------|
| None | 0x01 | -- |
| True | 0x02 | -- |
| False | 0x03 | -- |
| Int | 0x04 | Zigzag-encoded i64 varint |
| Float | 0x05 | 8 bytes LE f64 |
| String | 0x06 | String table index (u32 LE) |
| Bytes | 0x07 | Length (u32 LE) + raw bytes |
| List | 0x08 | Count (u32 LE) + record refs (u32 LE each) |
| Tuple | 0x09 | Count (u32 LE) + record refs (u32 LE each) |
| Dict | 0x0A | Count (u32 LE) + key/value ref pairs (u32 LE each) |
| Set | 0x0B | Count (u32 LE) + record refs (u32 LE each) |
| FrozenSet | 0x0C | Count (u32 LE) + record refs (u32 LE each) |
| Dataclass | 0x0D | Type ID (u16 LE) + field count (u16 LE) + field refs (u32 LE each) |
| Reference | 0x0E | Record ID (u32 LE) -- legacy; serializer no longer emits this tag |
| BigInt | 0x0F | u32 LE byte length N, then N bytes little-endian two's-complement signed |
| ByteArray | 0x10 | u32 len + raw bytes (mutable, memoized by identity) |
| Complex | 0x11 | 16 bytes: real f64 LE then imag f64 LE |
| StrRaw | 0x12 | u32 len + UTF-8 bytes with surrogatepass encoding |
| DateTime | 0x13 | year u16, month/day/hour/minute/second u8, microsecond u32, fold u8, tzinfo record id (u32::MAX = naive) |
| Date | 0x14 | year u16, month u8, day u8 |
| Time | 0x15 | hour/minute/second u8, microsecond u32, fold u8, tzinfo record id |
| TimeDelta | 0x16 | days i32, seconds u32, microseconds u32 |
| Timezone | 0x17 | offset (TimeDelta payload), has_name u8, optional name blob |
| ZoneInfo | 0x18 | key blob (validated on load) |
| Decimal | 0x19 | str(d) ASCII blob |
| Uuid | 0x1A | 16 raw bytes |
| Fraction | 0x1B | numerator id, denominator id |
| Range | 0x1C | start, stop, step ids |
| Slice | 0x1D | start, stop, step ids |
| Ellipsis | 0x1E | -- |
| NotImplemented | 0x1F | -- |
| Path | 0x20 | flavor u8 (PurePosix/PureWindows/Posix/Windows) + str blob |
| OrderedDict | 0x21 | pair count + (key id, value id) pairs |
| DefaultDict | 0x22 | factory code u8 + pair count + pairs |
| Counter | 0x23 | pair count + pairs |
| Deque | 0x24 | maxlen u64 (0 = None, else maxlen+1) + count + item ids |
| Enum | 0x25 | type id u16, mode u8 (0 name / 1 value), child id |
| NamedTuple | 0x26 | type id u16, count u32, item ids |

---

## Serialization Pipeline

### `dumps(obj)` -> `bytes`

1. **Walker** (`traversal.rs`): BFS traversal of the Python object graph
   - Records are assigned IDs in breadth-first order and written straight into the output buffer (no intermediate record tree)
   - `memo: FxHashMap<usize, u32>` maps Python object pointers -> record IDs (identity dedup)
   - Shared/cyclic objects reuse the record ID of the first occurrence directly -- containers point to existing record IDs without emitting separate Reference records
   - Immutable scalars are deduplicated by value: one record each for None/True/False, and one per distinct int, float (bit pattern) and string
   - Strings < 4096 bytes are interned by content; larger ones are memoized by identity and spliced at assembly
   - Dataclasses use `dataclasses.fields()` to enumerate fields, which correctly skips ClassVar and InitVar pseudo-fields; metadata is cached process-wide
   - Enums, namedtuples and exact stdlib types are dispatched via a per-type cache; subclasses of supported types are rejected
   - Unknown objects -> `UnsafeTypeError` (the walker raises `TypeError("Unsupported type: ...")`, `errors.rs` re-raises it as `UnsafeTypeError`)

2. **RecordWriter** (`encoder.rs`): Streaming byte-level writer
   - Writes records directly into output buffers as they are assigned
   - String table and type table accumulated in separate buffers
   - Large payloads (>= 4096 bytes) are referenced in place and copied once at assembly
   - Writer buffers are recycled per thread to avoid regrowing on every call

3. **HMAC** (optional): If `hmac_key` is provided, `integrity::sign` sets flags bit 0 and appends HMAC-SHA256(key, payload)

### `loads(data, *, allowlist, hmac_key, classes)` -> Python object

0. **Framing + HMAC** (`integrity.rs::decode_one`): key/flag mismatch -> `HMACError`; signed payloads are verified (constant time) before any Python object is created; returns bytes consumed so streams can continue

1. **Decoder phase 1** (`decoder.rs::decode`): Pure Rust, may run without the GIL
   - Validates the entire payload and builds a compact index (`Rec`, 16 bytes each)
   - All references are bounds-checked; incoming reference counts tracked (saturating at 2)
   - Strings validated in place as `&str` slices into the input

2. **Decoder phase 2** (`decoder.rs::reconstruct`): Needs the GIL
   - Explicit frame stack (no native recursion), depth limited by interpreter recursion limit
   - Mutable containers (list/dict/set/dataclass/bytearray) are created and memoized before their children (supports arbitrary DAGs and cycles)
   - Immutable values (tuple/frozenset/namedtuple/...) that are shared and hit a cycle raise `DecodeError`
   - Types resolve to the caller's `classes=` entry by stored name, else a generated class (dataclass/namedtuple); enums without `classes=` raise `UnsafeTypeError`

3. **Allowlist check** (between phases): every type-table name (dataclass, enum, namedtuple) must be in the allowlist, else `UnsafeTypeError`

4. **Migration**: only for `classes=` dataclasses whose stored and current versions are both > 0 and differ; runs `apply_migrations` and applies the result with default/extra-field checks

### `Unpickler.load()` / `load(file)`

- `Unpickler` keeps a read-ahead buffer and calls `_loads_prefix(buf, offset=...)`; incomplete records are retried after reading more; `EOFError` at a clean end
- `load(file)` reads the rest of the file, decodes one payload and seeks back over the unused bytes (seekable files)

### `load_pickle` / `loads_pickle` / `convert_pickle` (compat.py)

- Pure-Python opcode VM for pickle protocols 0-5; globals resolved only from `SAFE_GLOBALS`, `classes=` (matched by module + qualname) or `allow=`; no data-driven imports; user `__init__`/`__setstate__`/`__reduce__` never called; copy budget and `max_depth` limits

---

## Schema Evolution

### Defining a versioned dataclass

```python
from dataclasses import dataclass

@dataclass
class User:
    name: str
    age: int
    email: str = ""
    __pysafe_pickle_version__ = 2  # class attribute (legacy: __pygraph_version__)
```

### Registering migrations

```python
import pysafe_pickle

@pysafe_pickle.migrate(from_version=1, to_version=2, type_name="User")
def migrate_v1_to_v2(state: dict) -> dict:
    state["email"] = ""
    return state

# loads(old_blob, classes=[User]) now runs this automatically for version-1 data
```

- `@migrate` stores functions in `_migration_registry: dict[str, list[tuple[int, int, Callable]]]`
- `type_name` parameter is required for standalone functions (not class methods)
- Migration chains are resolved by `get_migration_chain()` (greedy: always picks largest version jump)
- Applied by `apply_migrations()` during `loads(..., classes=[...])` when the stored and class versions differ (both > 0); without `classes=` nothing is migrated

---

## Key Design Decisions

1. **No arbitrary code execution**: Unlike pickle, `__reduce__`, `__setstate__`, `exec`, `eval` are never called
2. **Closed type set**: Only built-in/stdlib types, dataclasses, namedtuples and enums are encodable; the decoder never imports anything named in the data (user classes come only from `classes=`)
3. **No Reference records**: Shared/cyclic objects reuse existing record IDs directly; containers point to the first occurrence's ID (no separate Reference tag emitted)
4. **String interning**: Short strings (< 4096 bytes) stored once in a table, referenced by index; large strings spliced by pointer at assembly
5. **Scalar dedup**: Lossy direct-mapped cache for ints/floats avoids hash-map overhead; repeated values are deduplicated
6. **Schema version per type**: Each dataclass carries a `u32` version in the binary format
7. **abi3 wheels**: PyO3 `abi3-py310` feature -> single wheel works for Python 3.10+
8. **No serde usage**: The binary format is hand-written (zigzag varints, custom tags)
9. **Two-phase decode**: Phase 1 (pure Rust, no GIL) validates and indexes; phase 2 (GIL) materializes Python objects
10. **Thread-local buffer recycling**: Walker scratch (queue, memo) and writer buffers are reused across calls
11. **Exceptions subclass builtins**: `UnsafeTypeError(TypeError)`, `DecodeError`/`HMACError`/`SchemaVersionError(ValueError)` keep pre-1.4 `except` clauses working
12. **HMAC before decode**: the tag is checked before any Python object is built; an unsigned payload loaded with a key is rejected (no downgrade)

---

## Building & Testing

```bash
# Local dev
pip install maturin
maturin develop
cargo nextest run          # 18 Rust tests
pytest tests/ -v           # 977 Python tests
pytest benchmarks/ -v      # 160+ benchmarks

# CI: Multi-OS x Python 3.10-3.15
# Publish: tag v* -> PyPI + GitHub Release
```

---

## Common Modification Points

| Task | Files to edit |
|------|---------------|
| Add new Python type support | `traversal.rs` (class_info/write_std), `encoder.rs` (RecordWriter methods), `decoder.rs` (parse + reconstruct), `format/mod.rs` (add Tag); also `compat.py` SAFE table if pickle files should load it |
| Change HMAC / framing | `integrity.rs`, `lib.rs` (`encode`, `loads_from_slice`) |
| Change exception mapping | `errors.rs`, `python/pysafe_pickle/exceptions.py` |
| Change pickle-file reader policy | `python/pysafe_pickle/compat.py` |
| Add new Tag | `format/mod.rs` (Tag enum + from_u8), `encoder.rs`, `decoder.rs` |
| Change binary format header | `format/mod.rs` (MAGIC_WRITE, FORMAT_VERSION, SCHEMA_VERSION) |
| Modify migration logic | `python/pysafe_pickle/migrations.py` |
| Change public API | `python/pysafe_pickle/__init__.py`, `python/pysafe_pickle/_pysafe_pickle.pyi`, `src/lib.rs` |
| Add Rust dependencies | `Cargo.toml` |
| Add Python dependencies | `pyproject.toml` |
| Update CI matrix | `.github/workflows/ci.yml` |
| Change publish triggers | `.github/workflows/publish.yml` |
