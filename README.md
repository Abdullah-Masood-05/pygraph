# pygraph

Safe, fast, schema-evolvable Python object graph serialization powered by Rust.

## Features

- **Drop-in pickle API** — `dumps`/`loads`/`dump`/`load` with the same signatures
- **Zero arbitrary code execution** — no `__reduce__` or `__setstate__` calls during deserialization
- **Schema versioning** — migration hooks for evolving your data models
- **Zero-copy tensor support** — NumPy and PyTorch integration
- **Rust-native performance** — PyO3 bindings for speed

## Installation

```bash
pip install pygraph
```

For tensor support:

```bash
pip install pygraph[numpy]
pip install pygraph[torch]
pip install pygraph[all]
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

## Security

pygraph never calls `__reduce__`, `__setstate__`, or any arbitrary code during deserialization. Only allowlisted types can be loaded.

## License

MIT
