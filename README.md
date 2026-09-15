# pysafe-pickle

Safe, fast, schema-evolvable Python object graph serialization powered by Rust.

## Features

- **Drop-in pickle API** — `dumps`/`loads`/`dump`/`load` with the same signatures
- **Zero arbitrary code execution** — no `__reduce__` or `__setstate__` calls during deserialization
- **Schema versioning** — migration hooks for evolving your data models
- **Zero-copy tensor support** — NumPy and PyTorch integration
- **Rust-native performance** — PyO3 bindings for speed
- **Pickle-compatible streaming** — `Pickler`/`Unpickler` classes, `PickleBuffer` (PEP 574)
- **HMAC integrity** — optional tamper detection on serialized data

## Installation

```bash
pip install pysafe-pickle
```

For tensor support:

```bash
pip install pysafe-pickle[numpy]
pip install pysafe-pickle[torch]
pip install pysafe-pickle[all]
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

Define versioned dataclasses and register migrations:

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
- Chains of migrations are resolved automatically (e.g., v1 → v2 → v3)

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

## Security

pysafe-pickle never calls `__reduce__`, `__setstate__`, or any arbitrary code during deserialization. Only allowlisted types can be loaded.

```python
# Restrict deserialization to specific types
psp.loads(data, allowlist={"builtins.dict", "builtins.list"})
```

## Allowlisted types

By default, pysafe-pickle supports:

| Type | Notes |
|------|-------|
| `None`, `bool`, `int`, `float` | Primitives |
| `str`, `bytes` | Strings and binary |
| `list`, `tuple` | Sequences |
| `dict` | Mappings |
| `set`, `frozenset` | Sets |
| `dataclasses` | Any `@dataclass` instance |

Any type not in this list raises `UnsafeTypeError` unless added to the allowlist.

## Integrity verification

```python
key = b"my-secret-key"
encoded = psp.dumps(data, hmac_key=key)
decoded = psp.loads(encoded, hmac_key=key)  # raises HMACError if tampered
```

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

Run the benchmark suite to compare pysafe-pickle vs pickle:

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
