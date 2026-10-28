# Streaming API Reference

Classes for stream-oriented serialization, modelled on the standard `pickle.Pickler` and `pickle.Unpickler` APIs.

---

## `Pickler`

<!-- no-run -->
```python
class Pickler:
    def __init__(
        self,
        file: BinaryIO,
        *,
        protocol: int = 5,
        fix_imports: bool = True,
        buffer_callback: Any = None,
        schema_version: int | None = None,
        hmac_key: bytes | None = None,
    ): ...
```

`fix_imports` and `buffer_callback` are accepted for signature compatibility but ignored. `schema_version` is passed to `dumps`, which ignores it.

### Methods:
- **`dump(obj: Any) -> None`**: Serializes `obj` with `dumps(obj, protocol=..., schema_version=..., hmac_key=...)` and writes the bytes to the stream. Each call appends one record. Raises `ValueError` if `protocol != 5` or `hmac_key` is empty, and `UnsafeTypeError` for unsupported types; these are raised at `dump` time, not at construction.
- **`proto`**: The protocol number passed to the constructor (`5` by default).

---

## `Unpickler`

<!-- no-run -->
```python
class Unpickler:
    def __init__(
        self,
        file: BinaryIO,
        *,
        fix_imports: bool = True,
        encoding: str = "ASCII",
        errors: str = "strict",
        buffers: Any = None,
        allowlist: set[str] | None = None,
        hmac_key: bytes | None = None,
        classes: Iterable[type] | Mapping[str, type] | None = None,
    ): ...
```

`fix_imports`, `encoding`, `errors` and `buffers` are accepted for signature compatibility but ignored. `allowlist`, `hmac_key` and `classes` have the same meaning as in [`loads`](./core.md#loads) and apply to every record.

### Methods:
- **`load() -> Any`**: Returns the next record in the stream. Repeated calls return the records written by repeated `Pickler.dump()` / `dump()` calls, in order.
  - Raises **`EOFError`** (`Ran out of input`) at a clean end of the stream.
  - Raises **`DecodeError`** if the stream ends inside a record or contains trailing bytes that are not a valid record, and **`HMACError`** if a signed record fails verification.
  - The file is read in chunks that grow as needed, and bytes read past a record are kept in an internal buffer for the next call. The file position can therefore be ahead of the last record returned. Works on non-seekable streams (pipes, sockets) and with short reads.

```python
import io
import pysafe_pickle as psp

buf = io.BytesIO()
p = psp.Pickler(buf, hmac_key=b"key")
p.dump({"a": [1, 2, 3]})
p.dump("second")

buf.seek(0)
u = psp.Unpickler(buf, hmac_key=b"key")
print(u.load())  # {'a': [1, 2, 3]}
print(u.load())  # second
try:
    u.load()
except EOFError as e:
    print("EOFError:", e)  # EOFError: Ran out of input
```

If you need to hand the file to other code after reading part of it, use [`load(file)`](./core.md#load) on a seekable file instead: it leaves the position exactly after the record it returned.

---

## `PickleBuffer`

PEP 574-style buffer wrapper. It is a plain helper class: `dumps` / `Pickler.dump` do not accept it as a value (`UnsafeTypeError: Unsupported type: PickleBuffer`), and `buffer_callback` / `buffers` are not wired up. There is no out-of-band buffer transfer.

<!-- no-run -->
```python
class PickleBuffer:
    def __init__(self, buf: bytes, *, readonly: bool = False): ...
    def raw(self) -> memoryview: ...
    @property
    def buf(self) -> memoryview: ...
    @property
    def readonly(self) -> bool: ...
```
