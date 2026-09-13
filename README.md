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
import pygraph

# Serialize
data = {"key": "value", "numbers": [1, 2, 3]}
encoded = pygraph.dumps(data)

# Deserialize
decoded = pygraph.loads(encoded)
assert decoded == data
```

## Schema Evolution

Define versioned dataclasses and register migrations:

```python
from dataclasses import dataclass
import pygraph

@dataclass
class User:
    name: str
    age: int
    email: str = ""
    __pygraph_version__ = 2

@pygraph.migrate(from_version=1, to_version=2, type_name="User")
def migrate_v1_to_v2(state: dict) -> dict:
    """V1 had 'name' + 'age', V2 adds 'email'."""
    state["email"] = ""
    state["__pygraph_version__"] = 2
    return state
```

### How it works

- Set `__pygraph_version__` as a class attribute on your dataclass
- Register migration functions with `@pygraph.migrate(from_version=N, to_version=M, type_name="ClassName")`
- Migration functions receive a `dict` of the old state and return a `dict` with the new state
- Chains of migrations are resolved automatically (e.g., v1 → v2 → v3)

## Pickler / Unpickler

Use pickle-compatible streaming classes:

```python
import pygraph
import io

# Streaming dump
buf = io.BytesIO()
pickler = pygraph.Pickler(buf, protocol=5)
pickler.dump({"data": [1, 2, 3]})

# Streaming load
buf.seek(0)
unpickler = pygraph.Unpickler(buf)
result = unpickler.load()
```

## Security

pygraph never calls `__reduce__`, `__setstate__`, or any arbitrary code during deserialization. Only allowlisted types can be loaded.

```python
# Restrict deserialization to specific types
pygraph.loads(data, allowlist={"builtins.dict", "builtins.list"})
```

## Allowlisted types

By default, pygraph supports:

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
encoded = pygraph.dumps(data, hmac_key=key)
decoded = pygraph.loads(encoded, hmac_key=key)  # raises HMACError if tampered
```

## Benchmarks

Run the benchmark suite to compare pygraph vs pickle:

```bash
pytest benchmarks/ -v --benchmark-only
```

## Development

```bash
# Install dev tools
pip install -e ".[dev]"

# Build the Rust extension
maturin develop

# Run tests
cargo nextest run
pytest tests/ -v
```

## License

AGPL-3.0-only
