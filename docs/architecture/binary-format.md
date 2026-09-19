# Binary Format Specification

`pysafe-pickle` uses a compact, hand-written binary format designed for high-speed serialization without invoking arbitrary execution hooks.

---

## Header Layout

```text
[4 bytes]  Magic: b"PSPK" (new) or b"PYGR" (v1.0.x legacy)
[2 bytes]  Format Version: 1 (u16 LE)
[4 bytes]  Flags: 0 (u32 LE) - bit 0: HMAC enabled
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
```

---

## 14 Tag Definitions

| Tag | Byte | Payload Structure |
| :--- | :---: | :--- |
| `None` | `0x01` | No payload |
| `True` | `0x02` | No payload |
| `False` | `0x03` | No payload |
| `Int` | `0x04` | Zigzag-encoded variable-length 64-bit integer (`u64` varint) |
| `Float` | `0x05` | 8-byte IEEE 754 little-endian `f64` |
| `String` | `0x06` | String table index (u32 LE) |
| `Bytes` | `0x07` | Length (u32 LE) + raw bytes |
| `List` | `0x08` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Tuple` | `0x09` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Dict` | `0x0A` | Count (u32 LE) + key/value record ID pairs (u32 LE each) |
| `Set` | `0x0B` | Count (u32 LE) + child record IDs (u32 LE each) |
| `FrozenSet` | `0x0C` | Count (u32 LE) + child record IDs (u32 LE each) |
| `Dataclass` | `0x0D` | Type ID (u16 LE) + field count (u16 LE) + field record IDs (u32 LE each) |
| `Reference` | `0x0E` | Record ID (u32 LE) — handles cycles and shared object pointers |

---

## String Interning

All dictionary keys, field names, and string values are interned into a deduplicated string table at the front of the binary payload. This dramatically reduces payload sizes for tabular datasets and dataclasses with repeating field keys.
