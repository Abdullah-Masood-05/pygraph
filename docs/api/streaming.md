# Streaming API Reference

Classes for stream-oriented serialization, modelled on the standard `pickle.Pickler` and `pickle.Unpickler` APIs.

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

`fix_imports` and `buffer_callback` are accepted for signature compatibility but ignored.

### Methods:
- **`dump(obj: Any) -> None`**: Serializes `obj` with `dumps` (passing `protocol`, `schema_version` and `hmac_key`) and writes the bytes to the underlying stream. Raises `ValueError` if `protocol != 5` and `NotImplementedError` if `hmac_key` is set; both are raised at `dump` time, not at construction.
- **`proto`**: The protocol number passed to the constructor (`5` by default).

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

`fix_imports`, `encoding`, `errors` and `buffers` are accepted for signature compatibility but ignored. `allowlist` has the same semantics as in `loads` (set of dataclass names).

### Methods:
- **`load() -> Any`**: Reads the **entire** remaining stream with one `.read()` and deserializes it as a single payload. Each stream holds one payload: writing several objects to one stream and calling `load()` repeatedly does not work (a second `load()` on an exhausted stream raises `ValueError: Invalid data: too short for header`).

```python
import io
import pysafe_pickle as psp

buf = io.BytesIO()
psp.Pickler(buf).dump({"a": [1, 2, 3]})
buf.seek(0)
print(psp.Unpickler(buf).load())  # {'a': [1, 2, 3]}
```

---

## `PickleBuffer`

PEP 574-style buffer wrapper. It is a plain helper class: `dumps` / `Pickler.dump` do not support it as a payload value (`TypeError: Unsupported type: PickleBuffer`), and `buffer_callback` / `buffers` are not wired up.

```python
class PickleBuffer:
    def __init__(self, buf: bytes, *, readonly: bool = False): ...
    def raw(self) -> memoryview: ...
    @property
    def buf(self) -> memoryview: ...
    @property
    def readonly(self) -> bool: ...
```
