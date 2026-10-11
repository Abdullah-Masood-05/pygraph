# pysafe-pickle

Safe, fast, schema-evolvable Python object graph serialization powered by Rust.

[![Documentation](https://img.shields.io/badge/docs-website-10b981?style=flat-square&logo=vitepress)](https://abdullah-masood-05.github.io/pygraph/)
[![PyPI](https://img.shields.io/pypi/v/pysafe-pickle?style=flat-square)](https://pypi.org/project/pysafe-pickle/)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/Abdullah-Masood-05/pygraph?style=flat-square&logo=github)](https://github.com/Abdullah-Masood-05/pygraph/stargazers)

📖 **Documentation**: [https://abdullah-masood-05.github.io/pygraph/](https://abdullah-masood-05.github.io/pygraph/)

> ⭐ **Enjoying pysafe-pickle?** If it saved you from unsafe deserialization, please [star it on GitHub](https://github.com/Abdullah-Masood-05/pygraph) or [share it](https://pypi.org/project/pysafe-pickle/). It's free and it helps other Python developers find it.

## What's New in v1.3.0

> v1.3.2 changed only the README. v1.3.3 fixes re-dumping loaded dataclass instances and corrects the documentation (see [CHANGELOG.md](CHANGELOG.md)).

### Performance
- **Faster `dumps`** — faster than stdlib `pickle` on every benchmark shape measured; `loads` is roughly on par with `pickle` and slower on some shapes. Payloads are larger than `pickle`'s
- Dataclass serialization is **6x faster** (dumps) and **22x faster** (loads) for bulk dataclass payloads
- String-heavy payloads serialize **2x faster**

### New Type Support
- Arbitrary-precision integers (`2**100`, `10**1000`, etc.)
- `bytearray` (with identity/cycle preservation)
- `complex` numbers
- Strings with lone surrogates (`"\ud800"`)

### Bug Fixes
- Dataclass self-referential cycles now load correctly
- `ClassVar` and `InitVar` fields are no longer incorrectly serialized
- Two dataclasses sharing a class name no longer corrupt each other's fields
- Fields named `self` no longer break deserialization
- `loads()`/`load()` now accept `bytes`, `bytearray`, and `memoryview` inputs
- Dataclass instances returned by `loads()` can be passed to `dumps()` again

## Features

- **Familiar pickle-style API** — `dumps`/`loads`/`dump`/`load` (protocol 5 only; same basic call shapes)
- **Zero arbitrary code execution** — no `__reduce__` or `__setstate__` calls during deserialization
- **Schema versioning** — migration hooks for evolving your data models
- **Fast serialization** — Rust-powered `dumps` is faster than stdlib pickle on most data shapes; `loads` is roughly on par (slower on some shapes) and payloads are larger
- **Extended type support** — `bytearray`, `complex`, arbitrary-precision `int`, and surrogate strings
- **Pickle-compatible streaming** — `Pickler`/`Unpickler` classes, `PickleBuffer` (a simple buffer wrapper; out-of-band transfer is not supported)
- **HMAC integrity** — not yet supported (planned); `dumps(hmac_key=...)` raises `NotImplementedError`

## Installation

```bash
pip install pysafe-pickle
```

## Quick Start

```python
import pysafe_pickle as psp

# Serialize
data = {"key": "value", "numbers": [1, 2, 3]}
encoded = psp.dumps(data)

# Deserialize
decoded = psp.loads(encoded)
assert decoded == data
```

## Schema Evolution

Declare a schema version on your dataclasses and register migrations:

```python
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class User:
    name: str
    age: int
    email: str = ""
    __pysafe_pickle_version__ = 2

@psp.migrate(from_version=1, to_version=2, type_name="User")
def migrate_v1_to_v2(state: dict) -> dict:
    """V1 had 'name' + 'age', V2 adds 'email'."""
    state["email"] = ""
    state["__pysafe_pickle_version__"] = 2
    return state
```

### How it works

- Set `__pysafe_pickle_version__` as a class attribute on your dataclass (legacy `__pygraph_version__` is also fully supported)
- Register migration functions with `@psp.migrate(from_version=N, to_version=M, type_name="ClassName")`
- Migration functions receive a `dict` of the old state and return a `dict` with the new state
- Chains of migrations are resolved (e.g., v1 → v2 → v3) by `psp.apply_migrations(...)`
- **`loads` does not apply migrations automatically in normal use**: loaded dataclasses are dynamically generated classes that carry no version. Track the version the data was written with, load, then call `psp.apply_migrations("User", vars(obj), from_version, to_version)` and build your current class from the result. See the [Schema Evolution guide](docs/guide/schema-evolution.md) for the full workflow.

## Pickler / Unpickler

Use pickle-compatible streaming classes:

```python
import pysafe_pickle as psp
import io

# Streaming dump
buf = io.BytesIO()
pickler = psp.Pickler(buf, protocol=5)
pickler.dump({"data": [1, 2, 3]})

# Streaming load
buf.seek(0)
unpickler = psp.Unpickler(buf)
result = unpickler.load()
```

`Unpickler.load()` reads the whole stream, so use one record per stream (one `dump` per buffer).

## Security

pysafe-pickle never calls `__reduce__`, `__setstate__`, or any arbitrary code during deserialization. Only the supported types below can be loaded.

The optional `allowlist` restricts which **dataclass** names may be loaded. It must be a `set` of class `__name__` strings. Built-in types are never checked against it.

```python
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class User:
    name: str
    age: int

data = psp.dumps(User("Alice", 30))

psp.loads(data, allowlist={"User"})   # OK
psp.loads(data, allowlist={"Other"})  # TypeError: Type 'User' is not in the allowlist
```

## Supported types

| Type | Notes |
|------|-------|
| `None`, `bool`, `float`, `complex` | Primitives |
| `int` | Any size (arbitrary precision) |
| `str` | Including lone surrogates |
| `bytes`, `bytearray` | Binary |
| `list`, `tuple` | Sequences |
| `dict` | Mappings |
| `set`, `frozenset` | Sets |
| dataclass instances | Any `@dataclass` instance; subject to `allowlist` by class name |

Anything else (subclasses of the above, `datetime`, NumPy arrays, PyTorch tensors, etc.) raises `TypeError: Unsupported type`.

## Integrity verification

HMAC integrity verification is **not yet supported** (planned). `dumps(..., hmac_key=...)` raises `NotImplementedError`, and `loads`/`load` have no `hmac_key` parameter.

## Migration from v1.0.x (`pygraph`)

In v1.0.x, the package was imported as `import pygraph`. In v1.1.0:
- The canonical import is now `import pysafe_pickle as psp`.
- **Zero code breaks**: Existing code using `import pygraph` or `from pygraph.migrations import migrate` continues working identically via a backward-compatible shim, emitting a `FutureWarning`.
- **Binary compatibility**: Files serialized with v1.0.x (magic `PYGR`) continue to deserialize seamlessly with zero warnings. New files are encoded with the `PSPK` magic header.
- The `pygraph` shim will remain supported through v1.x and will be removed in v2.0.0.

```python
# Legacy (deprecated in v1.1.0, emits warning, still works)
import pygraph
pygraph.dumps(data)

# Recommended
import pysafe_pickle as psp
psp.dumps(data)
```

## Benchmarks

Run the benchmark suite to compare pysafe-pickle vs pickle. In our measurements `dumps` is faster than pickle on every shape tested, `loads` is roughly on par (slower on some shapes), and payloads are larger than pickle's:

```bash
uv run pytest benchmarks/ -v --benchmark-only
```

## Development

```bash
# Install and build with uv / maturin
uv run maturin develop

# Run tests
cargo nextest run
uv run pytest tests/ -v
```

## License

AGPL-3.0-only
