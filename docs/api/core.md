# Core Functions API Reference

The primary serialization and deserialization functions provided by `pysafe_pickle`.

---

## `dumps(obj, *, protocol=5, schema_version=None, hmac_key=None) -> bytes`

Serializes a Python object graph into bytes using Rust-powered binary encoding.

### Parameters:
- **`obj`** (*Any*): The Python object, container, or dataclass to serialize.
- **`protocol`** (*int*, optional): Protocol version (default: `5`).
- **`schema_version`** (*int*, optional): Explicit schema version override.
- **`hmac_key`** (*bytes*, optional): Secret key for HMAC-SHA256 signature calculation.

### Returns:
- **`bytes`**: Serialized binary payload starting with `PSPK` magic bytes.

### Example:
```python
import pysafe_pickle as psp

payload = psp.dumps({"user": "alice", "active": True})
```

---

## `loads(data, *, allowlist=None, hmac_key=None) -> Any`

Deserializes a binary payload back into Python objects.

### Parameters:
- **`data`** (*bytes*): The binary payload to deserialize (accepts both `PSPK` and `PYGR` magic headers).
- **`allowlist`** (*set[str]*, optional): Set of permitted type name strings (e.g. `{"builtins.dict", "my_module.User"}`).
- **`hmac_key`** (*bytes*, optional): Secret key used to verify the HMAC signature.

### Returns:
- **`Any`**: Reconstructed Python object graph.

### Raises:
- **`UnsafeTypeError`**: If an unallowlisted type is encountered.
- **`HMACError`**: If HMAC verification fails or data was tampered with.
- **`ValueError`**: If binary format header or payload is corrupted.

---

## `dump(obj, file, **kwargs) -> None`

Serializes `obj` and writes the resulting bytes directly to a file-like object.

### Parameters:
- **`obj`** (*Any*): The Python object to serialize.
- **`file`** (*BinaryIO*): A writable binary stream (must implement `.write()`).
- **`**kwargs`**: Additional options passed to `dumps`.

---

## `load(file, *, allowlist=None) -> Any`

Reads serialized bytes from a file-like object and deserializes the object graph.

### Parameters:
- **`file`** (*BinaryIO*): A readable binary stream (must implement `.read()`).
- **`allowlist`** (*set[str]*, optional): Optional type allowlist.
