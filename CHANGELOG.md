# Changelog

All notable changes to pysafe-pickle are documented here.

## [1.3.0] - 2026-10-03

### Performance
- Serialization (`dumps`) is now faster than stdlib `pickle` on all tested data shapes
- Deserialization (`loads`) matches or beats `pickle` on most shapes
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
