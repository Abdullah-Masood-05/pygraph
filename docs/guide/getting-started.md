# Installation & Quickstart

## Installation

Install `pysafe-pickle` from PyPI using your favorite package manager:

::: code-group
```bash [pip]
pip install pysafe-pickle
```

```bash [uv]
uv add pysafe-pickle
```

```bash [poetry]
poetry add pysafe-pickle
```
:::

### Optional Dependencies (Tensor Support)

If you need zero-copy NumPy or PyTorch tensor serialization:

```bash
# NumPy support
pip install pysafe-pickle[numpy]

# PyTorch support
pip install pysafe-pickle[torch]

# All extras
pip install pysafe-pickle[all]
```

---

## Quickstart

The core API mirrors Python's standard `pickle` module, making migration effortless.

### 1. Basic Serialization (`dumps` and `loads`)

```python
import pysafe_pickle as psp

# Complex nested structures with cycles
data = {
    "title": "Project Metrics",
    "values": [1, 2, 3, 4.5],
    "flags": {True, False},
    "metadata": (None, "v1.1.0", b"\x00\x01\x02"),
}

# Serialize into safe binary bytes
binary_blob = psp.dumps(data)

# Deserialize back into Python objects
restored = psp.loads(binary_blob)

assert restored["values"] == [1, 2, 3, 4.5]
```

### 2. File I/O (`dump` and `load`)

Stream serialized objects directly to and from file-like objects:

```python
import pysafe_pickle as psp

# Write to file
with open("checkpoint.psp", "wb") as f:
    psp.dump({"epoch": 42, "loss": 0.0125}, f)

# Read from file
with open("checkpoint.psp", "rb") as f:
    checkpoint = psp.load(f)

print(checkpoint)  # {'epoch': 42, 'loss': 0.0125}
```

### 3. Handling Cyclic Graphs

`pysafe-pickle` automatically tracks object identities using an internal memo table:

```python
import pysafe_pickle as psp

node_a = {"name": "Node A"}
node_b = {"name": "Node B"}

# Create mutual cycle
node_a["neighbor"] = node_b
node_b["neighbor"] = node_a

# Dumps and loads preserve exact cycle topologies without recursion errors
encoded = psp.dumps(node_a)
decoded = psp.loads(encoded)

assert decoded["neighbor"]["neighbor"] is decoded
```

---

## Next Steps

- Learn how to evolve data models over time with [Schema Evolution](/guide/schema-evolution).
- Explore [Streaming & PickleBuffer](/guide/streaming-and-buffers) for high-performance out-of-band transfers.
- Learn about [Security & Allowlist](/guide/security-and-allowlist).
