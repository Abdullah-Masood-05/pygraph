# Core Functions API Reference

The primary serialization and deserialization functions provided by `pysafe_pickle`.

---

## Supported types

`None`, `bool`, `int` (arbitrary size), `float`, `complex`, `str` (including lone surrogates), `bytes`, `bytearray`, `list`, `tuple`, `dict`, `set`, `frozenset` and dataclass instances (including frozen and slotted ones).

Only these exact types are accepted. Subclasses (e.g. `IntEnum`, `str` subclasses, `OrderedDict`) and any other object (functions, `PickleBuffer`, ...) raise `TypeError: Unsupported type: <name>`.

Graph behavior:
- Shared identity is preserved for mutable containers (`list`, `dict`, `set`, `bytearray`) and dataclass instances, and cycles through them (including dataclass self-cycles) round-trip.
- A cycle that passes through an immutable container (`tuple` or `frozenset`) serializes, but raises `ValueError` on `loads`.
- Only `dataclasses.fields()` are serialized, so `ClassVar` and `InitVar` pseudo-fields are skipped.
- Loaded dataclasses are **not** instances of the original class. They are dynamic classes created by `make_class` (module `pysafe_pickle._reconstruct`) with the same name and field names, built without calling `__init__`.

---

## `dumps(obj, *, protocol=5, schema_version=None, hmac_key=None) -> bytes`

Serializes a Python object graph into bytes using Rust-powered binary encoding.

### Parameters:
- **`obj`** (*Any*): The Python object, container, or dataclass to serialize.
- **`protocol`** (*int*, optional): Protocol version. Only `5` is supported (default: `5`).
- **`schema_version`** (*int*, optional): Accepted for compatibility but currently ignored. The schema version written for each dataclass is read from the class attribute `__pysafe_pickle_version__` (or legacy `__pygraph_version__`).
- **`hmac_key`** (*bytes*, optional): Not implemented yet. Passing any non-`None` value raises `NotImplementedError`.

### Returns:
- **`bytes`**: Serialized binary payload starting with `PSPK` magic bytes. Output is deterministic for equal inputs.

### Raises:
- **`ValueError`**: If `protocol` is not `5` (`Unsupported protocol 4; only protocol 5 is supported`), or a size exceeds the format's limits.
- **`NotImplementedError`**: If `hmac_key` is given (`HMAC signatures are not yet supported`).
- **`TypeError`**: If the graph contains an unsupported type (`Unsupported type: function`).
- **`RecursionError`**: If the graph is nested too deeply.

### Example:
```python
import pysafe_pickle as psp

payload = psp.dumps({"user": "alice", "active": True})
```

---

## `loads(data, *, allowlist=None) -> Any`

Deserializes a binary payload back into Python objects.

### Parameters:
- **`data`** (*bytes | bytearray | memoryview*): The binary payload to deserialize (accepts both `PSPK` and `PYGR` magic headers). Any object supporting the buffer protocol works; non-`bytes` input is copied first.
- **`allowlist`** (*set[str]*, optional): Set of permitted **dataclass names** (the bare class name, e.g. `{"User", "Address"}`, not module-qualified). It must be a real `set` (a `list` or `frozenset` raises `TypeError`). Built-in types are always allowed.

### Returns:
- **`Any`**: Reconstructed Python object graph.

### Raises:
- **`TypeError`**: If a dataclass name in the payload is not in `allowlist` (`Type 'User' is not in the allowlist`), or if `data` is not bytes-like (`a bytes-like object is required, not 'str'`).
- **`ValueError`**: If the header or payload is corrupted or truncated (e.g. `Invalid magic bytes`, `Invalid data: too short for header`), or if a reference cycle passes through a `tuple` or `frozenset`.
- **`RecursionError`**: If the payload nesting is too deep.

`loads` does not take an `hmac_key` argument; passing one raises `TypeError`.

### Example:
```python
import pysafe_pickle as psp

data = psp.dumps({"n": 10**40, "raw": bytearray(b"ab"), "z": 1 + 2j})
print(psp.loads(memoryview(data)))
# {'n': 10000000000000000000000000000000000000000, 'raw': bytearray(b'ab'), 'z': (1+2j)}
```

---

## `dump(obj, file, **kwargs) -> None`

Serializes `obj` and writes the resulting bytes directly to a file-like object.

### Parameters:
- **`obj`** (*Any*): The Python object to serialize.
- **`file`** (*BinaryIO*): A writable binary stream (must implement `.write()`).
- **`**kwargs`**: Accepted but currently ignored. Unlike `dumps`, `dump` does not validate `protocol` or reject `hmac_key`.

---

## `load(file, *, allowlist=None) -> Any`

Reads serialized bytes from a file-like object and deserializes the object graph.

### Parameters:
- **`file`** (*BinaryIO*): A readable binary stream (must implement `.read()`). The whole stream is read with a single `.read()` call, so one payload per stream.
- **`allowlist`** (*set[str]*, optional): Optional dataclass-name allowlist, same semantics as `loads`.

### Raises:
- **`TypeError`**: If `file.read()` does not return a bytes-like object (`file.read() did not return bytes: ...`), or on an allowlist violation.
- **`ValueError`**: If the payload is corrupted or empty.
