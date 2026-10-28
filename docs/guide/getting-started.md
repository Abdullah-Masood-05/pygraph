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

`pysafe-pickle` has no runtime dependencies and requires Python 3.10 or newer (tested on Python 3.10 through 3.15). A single abi3 wheel covers all of these versions; free-threaded builds are not supported.

::: warning NumPy / PyTorch not supported
There are no `numpy`, `torch`, or `all` extras, and NumPy arrays or PyTorch tensors cannot be serialized (they raise `UnsafeTypeError: Unsupported type`). Convert them to lists or `bytes` first.
:::

---

## Quickstart

The core API mirrors the shape of Python's standard `pickle` module (`dumps`, `loads`, `dump`, `load`, `Pickler`, `Unpickler`). Only pickle protocol 5 is supported, and only the types listed in [What is pysafe-pickle?](/guide/what-is-pysafe-pickle#supported-types) can be serialized.

### 1. Basic Serialization (`dumps` and `loads`)

```python
import datetime as dt
import uuid
from decimal import Decimal
import pysafe_pickle as psp

data = {
    "id": uuid.UUID("12345678-1234-5678-1234-567812345678"),
    "created": dt.datetime(2026, 10, 28, 9, 30),
    "total": Decimal("19.99"),
    "values": [1, 2, 3, 4.5],
    "metadata": (None, "v1.4.0", b"\x00\x01\x02"),
}

binary_blob = psp.dumps(data)
restored = psp.loads(binary_blob)

print(restored == data)  # True
print(restored["created"])  # 2026-10-28 09:30:00
```

Besides `bytes`, `loads` also accepts `bytearray` and `memoryview` inputs.

### 2. File I/O (`dump` and `load`)

```python
import pysafe_pickle as psp

with open("checkpoint.psp", "wb") as f:
    psp.dump({"epoch": 41, "loss": 0.0131}, f)
    psp.dump({"epoch": 42, "loss": 0.0125}, f)

with open("checkpoint.psp", "rb") as f:
    print(psp.load(f))  # {'epoch': 41, 'loss': 0.0131}
    print(psp.load(f))  # {'epoch': 42, 'loss': 0.0125}
```

Each `load(f)` reads one record and leaves the file positioned after it. For long streams of records, see [Streaming](/guide/streaming-and-buffers).

### 3. Your Own Classes

Pass your dataclasses (and enums and namedtuples) with `classes=` to get real instances back:

```python
import enum
from dataclasses import dataclass
import pysafe_pickle as psp

class Role(enum.Enum):
    ADMIN = "admin"
    USER = "user"

@dataclass
class User:
    name: str
    role: Role = Role.USER

blob = psp.dumps([User("alice", Role.ADMIN), User("bob")])
users = psp.loads(blob, classes=[User, Role])

print(users)  # [User(name='alice', role=<Role.ADMIN: 'admin'>), User(name='bob', role=<Role.USER: 'user'>)]
print(isinstance(users[0], User))  # True
```

Without `classes=`, dataclasses come back as instances of a generated class with the same name and fields (`isinstance` is `False`), and enum members cannot be loaded at all (`UnsafeTypeError`). `__init__` and `__post_init__` never run on load.

### 4. Handling Cyclic Graphs

`pysafe-pickle` tracks object identities, so cycles and shared references survive a round trip:

```python
import pysafe_pickle as psp

node_a = {"name": "Node A"}
node_b = {"name": "Node B"}
node_a["neighbor"] = node_b
node_b["neighbor"] = node_a

decoded = psp.loads(psp.dumps(node_a))
print(decoded["neighbor"]["neighbor"] is decoded)  # True
```

### 5. Signing Payloads

```python
import pysafe_pickle as psp

blob = psp.dumps({"user": "admin"}, hmac_key=b"secret")
print(psp.loads(blob, hmac_key=b"secret"))  # {'user': 'admin'}
```

A tampered payload or a wrong key raises `HMACError`. See [Security & Allowlist](/guide/security-and-allowlist#hmac).

---

## What's New in 1.4

Highlights from [`CHANGELOG.md`](https://github.com/Abdullah-Masood-05/pygraph/blob/main/CHANGELOG.md):

- **HMAC-SHA256 signing** with `hmac_key=` on every dump and load function.
- **`classes=`**: real dataclass, enum and namedtuple instances on load, with defaults for missing fields.
- **Automatic migrations** when you pass `classes=` and the stored schema version differs.
- **New types**: `datetime`, `date`, `time`, `timedelta`, `timezone`, `ZoneInfo`, `Decimal`, `UUID`, `Fraction`, `range`, `slice`, `Ellipsis`, `NotImplemented`, `pathlib` paths, `OrderedDict`, `defaultdict` (builtin factories), `Counter`, `deque`, `Enum` members and namedtuples.
- **Multi-record streams** with `Pickler`/`Unpickler` and repeated `load(f)`.
- **Safe reader for existing pickle files**: [`load_pickle`, `loads_pickle`, `convert_pickle`](/guide/pickle-compat).
- **Real exceptions**: `UnsafeTypeError` (a `TypeError`), `DecodeError`, `HMACError`, `SchemaVersionError` (all `ValueError`s).

::: tip Behavior changes from 1.3
Exceptions are now the package's own classes, but they subclass the built-in ones raised before, so `except TypeError` / `except ValueError` still work. `IntEnum` members, `OrderedDict`, `defaultdict(list)` and namedtuples used to raise `TypeError` in `dumps` and now serialize.
:::

---

## Next Steps

- Evolve data models over time with [Schema Evolution](/guide/schema-evolution).
- Write and read many records with [Streaming & PickleBuffer](/guide/streaming-and-buffers).
- Lock down untrusted input with [Security & Allowlist](/guide/security-and-allowlist).
- Read old `.pkl` files safely with [Reading Existing Pickle Files](/guide/pickle-compat).
