---
layout: home

hero:
  name: "Pysafe Pickle"
  text: "Safe, Fast, Schema-Evolvable Object Graph Serialization"
  tagline: "Powered by Rust. Zero arbitrary code execution. Drop-in pickle API."
  image:
    src: /logo.svg
    alt: pysafe-pickle logo
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: Why pysafe-pickle?
      link: /guide/what-is-pysafe-pickle
    - theme: alt
      text: View on GitHub
      link: https://github.com/Abdullah-Masood-05/pygraph

features:
  - icon: 🛡️
    title: Zero Arbitrary Code Execution
    details: Unlike Python's standard pickle, pysafe-pickle never invokes __reduce__, __setstate__, exec, or eval during deserialization.
  - icon: ⚡
    title: Rust-Native Performance
    details: Core traversal, custom binary encoding, and decoding implemented in Rust with PyO3 bindings for maximum throughput.
  - icon: 🔄
    title: Schema Evolution & Migrations
    details: First-class support for evolving dataclass models with @psp.migrate decorators, automated chain resolution, and versioning.
  - icon: 📦
    title: Drop-in Pickle API
    details: Seamless dumps, loads, dump, load, and streaming Pickler / Unpickler classes matching standard pickle semantics.
  - icon: 🚀
    title: Zero-Copy Tensors & Streaming
    details: Native integration with NumPy and PyTorch tensors, plus PEP 574 PickleBuffer out-of-band streaming support.
  - icon: 🔐
    title: Allowlist & HMAC Verification
    details: Built-in strict type allowlisting and optional HMAC-SHA256 data tampering detection.
---

## Quick Example

```python
import pysafe_pickle as psp
from dataclasses import dataclass

@dataclass
class User:
    name: str
    age: int
    email: str = ""
    __pysafe_pickle_version__ = 1

# Serialize safely with Rust
data = psp.dumps(User(name="Alice", age=30, email="alice@example.com"))

# Deserialize without arbitrary code execution
user = psp.loads(data)
assert user.name == "Alice"
```

## Community & Ecosystem

- **PyPI Package**: [`pysafe-pickle`](https://pypi.org/project/pysafe-pickle/)
- **Repository**: [`Abdullah-Masood-05/pygraph`](https://github.com/Abdullah-Masood-05/pygraph)
- **License**: AGPL-3.0
