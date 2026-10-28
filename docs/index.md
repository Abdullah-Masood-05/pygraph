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
    details: Unlike Python's standard pickle, pysafe-pickle never invokes __reduce__, __setstate__, __init__, exec, or eval during deserialization, and never imports a module named in the data. Only a fixed set of types can be rebuilt.
  - icon: ⚡
    title: Rust-Native Speed
    details: Traversal, encoding, and decoding are implemented in Rust with PyO3. dumps is faster than pickle on most data shapes; loads is faster on dataclass-heavy data and slower on some shapes. Payloads are larger than pickle's.
  - icon: 🧩
    title: Your Classes, Real Types
    details: Pass classes= to get real dataclass, enum and namedtuple instances back. Dates and times, Decimal, UUID, Fraction, paths and collections types are supported out of the box.
  - icon: 🔄
    title: Automatic Schema Migrations
    details: Dataclass schema versions are embedded in the payload. Register @psp.migrate functions and they run during loads when the stored version differs from your class.
  - icon: 🔐
    title: HMAC Signing & Allowlist
    details: Sign payloads with HMAC-SHA256 (hmac_key=) and verify them before anything is decoded. Restrict which class names a payload may contain with an allowlist.
  - icon: 📦
    title: Pickle-Style API & Streams
    details: dumps, loads, dump, load, plus Pickler / Unpickler for multi-record streams. Read existing .pkl files safely with load_pickle and convert them with convert_pickle.
---

## Supported Types

`None`, `bool`, arbitrary-size `int`, `float`, `complex`, `str` (including lone surrogates), `bytes`, `bytearray`, `list`, `tuple`, `dict`, `set`, `frozenset`, `datetime`/`date`/`time`/`timedelta`/`timezone`, `ZoneInfo`, `Decimal`, `UUID`, `Fraction`, `range`, `slice`, `Ellipsis`, `NotImplemented`, `pathlib` paths, `OrderedDict`, `defaultdict` (builtin factories), `Counter`, `deque`, `@dataclass` instances, namedtuples and `Enum` members. Anything else raises `UnsafeTypeError: Unsupported type`.

## Quick Example

```python
import datetime as dt
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class User:
    name: str
    joined: dt.date
    email: str = ""

# Serialize (and sign) with Rust
data = psp.dumps(User("Alice", dt.date(2026, 10, 28)), hmac_key=b"secret")

# Verify, then deserialize without running any code from the payload
user = psp.loads(data, hmac_key=b"secret", classes=[User])
print(user)  # User(name='Alice', joined=datetime.date(2026, 10, 28), email='')
print(isinstance(user, User))  # True
```

## Community & Ecosystem

- **PyPI Package**: [`pysafe-pickle`](https://pypi.org/project/pysafe-pickle/)
- **Repository**: [`Abdullah-Masood-05/pygraph`](https://github.com/Abdullah-Masood-05/pygraph)
- **License**: AGPL-3.0
