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

`pysafe-pickle` has no runtime dependencies and requires Python 3.10 or newer.

::: warning NumPy / PyTorch not supported yet
There are no `numpy`, `torch`, or `all` extras, and NumPy arrays or PyTorch tensors cannot be serialized (they raise `TypeError: Unsupported type`). Convert them to lists or `bytes` first.
:::

---

## Quickstart

The core API mirrors the shape of Python's standard `pickle` module (`dumps`, `loads`, `dump`, `load`). Only pickle protocol 5 is supported, and only the types listed in [What is pysafe-pickle?](/guide/what-is-pysafe-pickle#supported-types) can be serialized.

### 1. Basic Serialization (`dumps` and `loads`)

```python
import pysafe_pickle as psp

# Nested structures, including sets, tuples and bytes
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

Besides `bytes`, `loads` also accepts `bytearray` and `memoryview` inputs.

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

`pysafe-pickle` tracks object identities, so cycles and shared references survive a round trip. This also works for self-referential dataclasses:

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

### 4. Extended Scalar Types

```python
import pysafe_pickle as psp

for value in [2**200, bytearray(b"ab"), 1 + 2j, "a\ud800b"]:
    assert psp.loads(psp.dumps(value)) == value
```

---

## What's New in 1.3

Highlights from [`CHANGELOG.md`](https://github.com/Abdullah-Masood-05/pygraph/blob/main/CHANGELOG.md) (1.3.0 to 1.3.2):

- **Performance**: `dumps` is faster than stdlib `pickle` on the benchmarked shapes, and `loads` matches or beats it on most (not all) shapes. Dataclass-heavy payloads see the largest gains. Payloads are larger than pickle's. See [Benchmarks](/benchmarks/performance) for numbers.
- **New types**: arbitrary-precision `int`, `bytearray` (identity and cycles preserved), `complex`, and `str` containing lone surrogates.
- **`loads` input**: accepts `bytes`, `bytearray`, and `memoryview`.
- **Dataclass fixes**: self-referential cycles load without `RecursionError`; `ClassVar` and `InitVar` pseudo-fields are no longer serialized; two distinct dataclasses sharing a `__name__` no longer corrupt each other; fields named `self` work; out-of-range string indices in the type table raise `ValueError`.
- **Wire format**: shared and cyclic references are encoded inline (smaller, faster to decode). The format remains v1-compatible.
- **1.3.1 / 1.3.2**: dead code removal, Python 3.14 in CI, README updates. No behavior changes.

::: tip Loaded dataclasses are dynamic classes
`loads` rebuilds dataclass instances as instances of a dynamically created class with the same name and field names, not your original class. `isinstance(obj, User)` is `False`, and `==` against an original `User` is `False`. See [Schema Evolution](/guide/schema-evolution) for how this affects migrations.
:::

---

## Next Steps

- Learn how to evolve data models over time with [Schema Evolution](/guide/schema-evolution).
- Read [Streaming & PickleBuffer](/guide/streaming-and-buffers) for the `Pickler` / `Unpickler` classes and their current limits.
- Learn about [Security & Allowlist](/guide/security-and-allowlist).
