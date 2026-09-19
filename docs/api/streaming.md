# Streaming API Reference

Classes for stream-oriented serialization, matching standard `pickle.Pickler` and `pickle.Unpickler` APIs.

---

## `Pickler`

A pickle-compatible pickler using `pysafe-pickle`'s safe serialization.

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

### Methods:
- **`dump(obj: Any) -> None`**: Serializes `obj` and writes to the underlying stream.
- **`proto`**: The active protocol number (`5`).

---

## `Unpickler`

A pickle-compatible unpickler using `pysafe-pickle`'s safe deserialization.

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
    ): ...
```

### Methods:
- **`load() -> Any`**: Reads from the underlying stream and reconstructs the next object.

---

## `PickleBuffer`

PEP 574 out-of-band buffer wrapper for zero-copy memory transfers.

```python
class PickleBuffer:
    def __init__(self, buf: bytes, *, readonly: bool = False): ...
    def raw(self) -> memoryview: ...
    @property
    def buf(self) -> memoryview: ...
    @property
    def readonly(self) -> bool: ...
```
