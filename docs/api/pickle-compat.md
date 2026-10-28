# Pickle Compatibility API Reference

Functions for reading data written by the standard `pickle` module (protocols 0-5) without executing it. They live in `pysafe_pickle.compat` and are re-exported from the package root. For background and examples, see the [Reading Existing Pickle Files guide](../guide/pickle-compat.md).

---

## Common keyword arguments {#common}

| Argument | Default | Meaning |
| :--- | :--- | :--- |
| `classes` | `None` | Iterable of your classes (dataclasses, plain classes, slotted classes, namedtuples, enums, exceptions) that may be rebuilt. Matched by `(cls.__module__, cls.__qualname__)` against the names stored in the pickle. Instances are created with `cls.__new__` (or `object.__new__` / the builtin base's `__new__` for `copyreg._reconstructor`), and state is written directly into `__dict__` and declared slots. `__init__`, `__setstate__`, `__reduce__`, `__setattr__`, `append` and `__setitem__` are never called. A mapping is **not** accepted here. |
| `allow` | `None` | Iterable of `(module, name)` tuples you trust completely. They are imported on demand and called with full pickle semantics (including `__setstate__`). |
| `encoding`, `errors` | `"ASCII"`, `"strict"` | How Python 2 `str` data (protocols 0-2) is decoded, as in `pickle.loads`. `encoding="bytes"` keeps it as `bytes`. |
| `fix_imports` | `True` | Map Python 2 module names to Python 3 names for protocols below 3. |
| `max_depth` | `1000` | Maximum container nesting depth (positive `int`). |

---

## `loads_pickle(data, *, classes=None, allow=None, encoding="ASCII", errors="strict", fix_imports=True, max_depth=1000) -> Any` {#loads-pickle}

Safely loads one pickle from `data` (`bytes`, `bytearray` or `memoryview`). Bytes after the pickle's `STOP` opcode are ignored, as with `pickle.loads`.

```python
import pickle
import pysafe_pickle as psp

print(psp.loads_pickle(pickle.dumps({"a": (1, 2.5, b"x")}, protocol=0)))
# {'a': (1, 2.5, b'x')}
```

---

## `load_pickle(data_or_file, *, classes=None, allow=None, encoding="ASCII", errors="strict", fix_imports=True, max_depth=1000) -> Any` {#load-pickle}

Safely loads one pickle from:
- bytes-like data,
- a `str` or `os.PathLike` path (the file is opened in binary mode and read), or
- a binary file object with `read()`. On a seekable file the position is left right after the pickle's `STOP` opcode, so several pickles written back to back can be read with repeated calls. A non-seekable stream is read to EOF.

Raises `EOFError` on empty input, and `TypeError` for other argument types or a file opened in text mode.

---

## `convert_pickle(data, *, classes=None, allow=None, encoding="ASCII", errors="strict", fix_imports=True, max_depth=1000, **dumps_kwargs) -> bytes` {#convert-pickle}

Loads one pickle with `load_pickle` (so `data` may be bytes, a path or a file) and returns `psp.dumps(obj, **dumps_kwargs)`. Use `dumps_kwargs` for `hmac_key=` (and `protocol=`/`schema_version=`).

The result can only hold types that `psp.dumps` supports (see [Core Functions](./core.md#supported-types)). A plain (non-dataclass) class loaded from the pickle makes `dumps` raise `UnsafeTypeError`. To read the result back as your own classes, pass the same classes to `psp.loads(..., classes=...)`.

---

## `SAFE_GLOBALS`

`pysafe_pickle.compat.SAFE_GLOBALS` is a `frozenset` of the `(module, name)` pairs the reader resolves without `classes=` or `allow=`:

| Module | Names |
| :--- | :--- |
| `builtins` | `set`, `frozenset`, `list`, `tuple`, `dict`, `bytearray`, `bytes`, `int`, `float`, `bool`, `str`, `complex`, `slice`, `range`, `object` (as a value only, not callable), `getattr` (an inert stand-in that only resolves `ZoneInfo._unpickle` and enum members of listed classes) |
| `codecs`, `_codecs` | `encode` (only `encode(<str>, "latin1")`, which protocols 0-2 use for `bytes`) |
| `copyreg` | `_reconstructor` (only for listed classes and safe builtin bases) |
| `datetime` | `datetime`, `date`, `time`, `timedelta`, `timezone` |
| `decimal`, `fractions`, `uuid` | `Decimal`, `Fraction`, `UUID` |
| `collections` | `OrderedDict`, `Counter`, `deque`, `defaultdict` (factory must be `None` or a builtin type) |
| `pathlib`, `pathlib._local` | `PurePath`, `PurePosixPath`, `PureWindowsPath`, `Path`, `PosixPath`, `WindowsPath` |
| `zoneinfo` (and `_zoneinfo`, `zoneinfo._zoneinfo`) | `ZoneInfo` (value only), `ZoneInfo._unpickle` (key validated) |

Every callable has a handler that validates its arguments before calling it. Forms that would allocate huge amounts of memory from a small input (`bytes(10**12)`, `Fraction("1e999999999")`) are rejected.

---

## Exceptions

- **`UnsafeTypeError`** (a `TypeError` and `PySafePickleError`): a global or operation that is not allowed, raised before anything is called.
- **`pickle.UnpicklingError`**: malformed input, truncated data, depth or copy-budget limits.
- **`ValueError`**, **`TypeError`**: invalid arguments to an allowed constructor, invalid keyword arguments.
- **`EOFError`**: empty input.
