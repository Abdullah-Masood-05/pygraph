---
layout: home

hero:
  name: "Pysafe Pickle"
  text: "Safe, Fast, Schema-Evolvable Object Graph Serialization"
  tagline: "Powered by Rust. No arbitrary code execution on load. Familiar pickle-style API."
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
    title: No Code Execution on Load
    details: Unlike Python's standard pickle, pysafe-pickle never invokes __reduce__, __setstate__, exec, or eval during deserialization. Only a fixed set of built-in types and dataclasses can be rebuilt.
  - icon: ⚡
    title: Rust-Native Speed
    details: Traversal, encoding, and decoding are implemented in Rust with PyO3. dumps is faster than pickle on most data shapes; loads is roughly on par, and slower on some. Payloads are larger than pickle's.
  - icon: 🔄
    title: Schema Versions & Migrations
    details: Dataclass schema versions are embedded in the payload, and @psp.migrate registers migration chains. Migrations are applied explicitly via apply_migrations (see the guide for current limits).
  - icon: 📦
    title: Pickle-Style API
    details: dumps, loads, dump, load, plus Pickler / Unpickler classes. loads accepts bytes, bytearray, and memoryview. Only protocol 5 is supported.
  - icon: 🔁
    title: Cycles & Shared References
    details: Cyclic and shared references are preserved for lists, dicts, sets, bytearrays, and dataclasses, including self-referential dataclasses.
  - icon: 🔐
    title: Type Allowlist
    details: Pass an allowlist to loads to restrict which dataclass types may be reconstructed. HMAC signing is not yet supported (hmac_key raises NotImplementedError).
---

## Supported Types

`None`, `bool`, arbitrary-size `int`, `float`, `complex`, `str` (including lone surrogates), `bytes`, `bytearray`, `list`, `tuple`, `dict`, `set`, `frozenset`, and `@dataclass` instances. Anything else raises `TypeError: Unsupported type`.

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

# Serialize with Rust
data = psp.dumps(User(name="Alice", age=30, email="alice@example.com"))

# Deserialize without arbitrary code execution
user = psp.loads(data)
assert user.name == "Alice"
# Note: `user` is an instance of a dynamically created class named "User",
# not of your original class.
assert type(user).__name__ == "User" and type(user) is not User
```

## Community & Ecosystem

- **PyPI Package**: [`pysafe-pickle`](https://pypi.org/project/pysafe-pickle/)
- **Repository**: [`Abdullah-Masood-05/pygraph`](https://github.com/Abdullah-Masood-05/pygraph)
- **License**: AGPL-3.0
