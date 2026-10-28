# Changelog

All notable changes to pysafe-pickle are documented here.

## [1.4.0] - 2026-10-28

### Added
- HMAC-SHA256 signing: `dumps`/`dump`/`Pickler` take `hmac_key=` and sign the payload; `loads`/`load`/`Unpickler` take `hmac_key=` and verify it (constant-time) before any object is built. A signed payload sets bit 0 of the header flags and appends a 32-byte tag computed over every preceding byte. `hmac_key` accepts any bytes-like object; an empty key raises `ValueError`
- `HMACError` for a failed verification, a wrong key, a signed payload loaded without a key, and a key given for an unsigned payload (so a stripped signature is detected)
- New types (tags `0x13`-`0x26`): `datetime.datetime`/`date`/`time`/`timedelta`/`timezone`, `zoneinfo.ZoneInfo`, `decimal.Decimal`, `uuid.UUID`, `fractions.Fraction`, `range`, `slice`, `Ellipsis`, `NotImplemented`, `pathlib` paths, `collections.OrderedDict`, `defaultdict` (builtin factories only), `Counter`, `deque` (with `maxlen`), `Enum` members and namedtuples
- `classes=` on `loads`, `load` and `Unpickler`: an iterable of classes or a `{name: class}` mapping. Listed dataclasses, enums and namedtuples load as the real classes (`isinstance` works). Dataclasses are created without calling `__init__`/`__post_init__`, missing fields get their defaults, and stored fields the class does not define raise `TypeError`
- Multi-record streams: repeated `Unpickler.load()` calls return successive records and raise `EOFError` at the end of the stream. `load(file)` leaves a seekable file positioned just after the record it read, so repeated `load` calls work too
- Safe reader for standard pickle data: `load_pickle`, `loads_pickle` and `convert_pickle` (protocols 0-5). A self-contained opcode interpreter calls only an allowlist of safe constructors (`pysafe_pickle.compat.SAFE_GLOBALS`), never imports a module named in the data, and never calls user `__reduce__`/`__setstate__`/`__init__`. Your own classes are opted in with `classes=`, other trusted globals with `allow=`. Resource limits: input-proportional copy budget and `max_depth`
- `DecodeError` (a `ValueError`) for corrupt, truncated or invalid payloads

### Changed
- Exceptions are now raised as the package's own classes: `UnsafeTypeError` (subclass of `TypeError`) for unsupported types and allowlist rejections, `DecodeError`/`HMACError`/`SchemaVersionError` (subclasses of `ValueError`), all subclasses of `PySafePickleError`. Existing `except TypeError` / `except ValueError` clauses keep working
- With `classes=`, schema migrations now run automatically during `loads` when the stored version and the class's `__pysafe_pickle_version__` are both non-zero and differ
- `IntEnum`/`StrEnum`/`Flag` and other enum members, `OrderedDict`, `defaultdict(list)` (and other builtin factories), namedtuples and the new stdlib types now serialize instead of raising `TypeError: Unsupported type`. Code that relied on that error to reject them must check types itself
- The `allowlist` also applies to enum and namedtuple type names, not only dataclasses
- Loading an enum member without its class in `classes=` raises `UnsafeTypeError`
- `load(file)` on an empty stream raises `EOFError` (was `ValueError`)

### Fixed
- `Unpickler.load()` no longer consumes the whole stream as a single record
- `dumps(hmac_key=...)` no longer raises `NotImplementedError`

## [1.3.4] - 2026-10-14

### Added
- Python 3.15 support: added to the CI test matrix (Linux, Windows, macOS) and the PyPI classifiers. The same abi3 wheel was also verified locally on Python 3.10 through 3.15 (424 tests each, no warnings from Python itself)

### Changed
- Added the Python 3.14 and 3.15 PyPI classifiers (supported: Python 3.10 through 3.15)
- The single abi3 wheel (`cp310-abi3`) covers all supported versions; free-threaded builds are not supported

## [1.3.3] - 2026-10-11

### Fixed
- Dataclass instances returned by `loads()` can now be passed back to `dumps()` (previously raised `AttributeError`)

### Changed
- Documentation rewritten and verified against the code: API reference, binary format (18 tags), Rust engine internals, guides and benchmarks (re-measured for this version)
- Corrected docs/README claims that did not match the code (tensor support, HMAC, allowlist semantics, automatic migrations, `Unpickler` behavior)
- Removed unused Rust source files (`src/migration`, `src/graph/types.rs`)

## [1.3.2] - 2026-10-11

### Changed
- README: added a GitHub stars badge and a short request to star the project
- No code changes; only the README (the PyPI page description) and version numbers changed

## [1.3.1] - 2026-10-03

### Changed
- Removed dead code: unused `migration` and `graph::types` Rust modules
- Suppressed warnings on reserved `FLAG_HMAC`/`FLAG_COMPRESSED` constants
- CI version check is now dynamic (no more hardcoded version strings)
- Added Python 3.14 to CI test matrix
- Zero compiler warnings on release builds

## [1.3.0] - 2026-10-03

### Performance
- Serialization (`dumps`) is now faster than stdlib `pickle` on all tested data shapes
- Deserialization (`loads`) is roughly on par with `pickle` and still slower on some shapes (large dicts, int/string lists); payloads are larger than pickle's
- Dataclass bulk serialization ~6x faster, deserialization ~22x faster
- String-heavy payloads ~2x faster to serialize
- Large bytes payloads ~5x faster to serialize
- Reduced per-object overhead through arena allocation and type-dispatch caching

### Added
- Arbitrary-precision integer support (BigInt tag 0x0F)
- `bytearray` support with identity/cycle preservation (ByteArray tag 0x10)
- `complex` number support (Complex tag 0x11)
- Strings with lone surrogates via surrogatepass encoding (StrRaw tag 0x12)
- `loads()` now accepts `bytes`, `bytearray`, and `memoryview` inputs
- 150+ new accuracy and fuzz tests
- Hypothesis property-based roundtrip tests

### Fixed
- Dataclass self-referential cycles no longer cause RecursionError on load
- `ClassVar` fields are no longer incorrectly included in serialized output
- `InitVar` pseudo-fields are no longer serialized
- Two distinct dataclasses sharing a `__name__` no longer corrupt each other
- Dataclass fields named `self` no longer break deserialization
- Out-of-range string indices in type table now raise ValueError instead of silently defaulting

### Changed
- Shared/cyclic object references are now encoded inline (no separate Reference records) — reduces payload size and decode overhead
- Wire format remains v1-compatible; old decoders can still read payloads that don't use new tags

## [1.2.0] - 2024-09-24

### Changed
- Removed unused crate dependencies (hmac, sha2, bincode, serde, rayon)
- Added rustc-hash for faster hash maps
- Optimized type dispatch and scalar memo skipping

## [1.1.0] - 2024-09-19

### Added
- Rebranded to pysafe-pickle with pygraph backward-compatibility shim

## [1.0.0] - 2024-09-11

### Added
- Initial release as pygraph
- Safe serialization with no arbitrary code execution
- Schema versioning and migration support
- Custom binary format with cycle detection
