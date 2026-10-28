# pysafe-pickle

Safe, fast, schema-evolvable Python object graph serialization powered by Rust.

[![Documentation](https://img.shields.io/badge/docs-website-10b981?style=flat-square&logo=vitepress)](https://abdullah-masood-05.github.io/pygraph/)
[![PyPI](https://img.shields.io/pypi/v/pysafe-pickle?style=flat-square)](https://pypi.org/project/pysafe-pickle/)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/Abdullah-Masood-05/pygraph?style=flat-square&logo=github)](https://github.com/Abdullah-Masood-05/pygraph/stargazers)

📖 **Documentation**: [https://abdullah-masood-05.github.io/pygraph/](https://abdullah-masood-05.github.io/pygraph/)

> ⭐ **Enjoying pysafe-pickle?** If it saved you from unsafe deserialization, please [star it on GitHub](https://github.com/Abdullah-Masood-05/pygraph) or [share it](https://pypi.org/project/pysafe-pickle/). It's free and it helps other Python developers find it.

## What's New in v1.4.0

- **HMAC-SHA256 signing**: `dumps(obj, hmac_key=key)` signs a payload and `loads(data, hmac_key=key)` verifies it before any object is built. Tampering, a wrong key or a missing key raises `HMACError`.
- **Your own classes back**: `loads(data, classes=[User])` returns real `User` instances (`isinstance` works). Dataclasses are created without calling `__init__`, missing fields get their defaults, and unknown fields are an error.
- **Automatic migrations**: with `classes=`, registered `@migrate` functions now run during `loads` when the stored schema version differs from your class's version.
- **Many more types**: `datetime`/`date`/`time`/`timedelta`/`timezone`/`ZoneInfo`, `Decimal`, `UUID`, `Fraction`, `range`, `slice`, `Ellipsis`, `NotImplemented`, `pathlib` paths, `OrderedDict`, `defaultdict` (builtin factories), `Counter`, `deque`, `Enum` members (including `IntEnum`, `StrEnum`, `Flag`) and namedtuples.
- **Multi-record streams**: call `Pickler.dump()` several times and `Unpickler.load()` returns the records one by one, raising `EOFError` at the end. `load(file)` on a seekable file leaves the file just past the record it read.
- **Read existing `.pkl` files safely**: `load_pickle`, `loads_pickle` and `convert_pickle` read standard pickle data (protocols 0-5) with a restricted interpreter that never runs code chosen by the data.
- **Real exceptions**: `UnsafeTypeError`, `DecodeError`, `HMACError` and `SchemaVersionError` are now raised. They still subclass `TypeError`/`ValueError`, so existing `except TypeError` / `except ValueError` clauses keep working.

Upgrading from 1.3.x? `IntEnum` members, `OrderedDict` and `defaultdict(list)` used to raise `TypeError` in `dumps` and now serialize. See [CHANGELOG.md](CHANGELOG.md) for the full list.

## Features

- **Familiar pickle-style API**: `dumps`/`loads`/`dump`/`load` and `Pickler`/`Unpickler` (protocol 5 only; same basic call shapes)
- **No code execution on load**: no `__reduce__`, `__setstate__`, `__init__` or `__post_init__` calls, and nothing named in the data is ever imported
- **Real classes on load** via `classes=`, for dataclasses, enums and namedtuples
- **Schema versioning** with migrations that run automatically on load
- **HMAC-SHA256 integrity** for data that passes through untrusted hands
- **Type allowlist** to restrict which dataclass, enum and namedtuple names a payload may contain
- **Wide stdlib coverage**: dates and times, `Decimal`, `UUID`, `Fraction`, paths, `collections` types, arbitrary-precision `int`, `complex`, `bytearray`, lone-surrogate strings
- **Cycles and shared references** preserved
- **Multi-record streams** with `Pickler`/`Unpickler`
- **Safe reader for standard pickle files** to migrate existing data
- **Rust-powered**: `dumps` is faster than stdlib `pickle` on most shapes; `loads` is on par, faster on dataclass-heavy data and slower on some shapes (see [Compared with pickle](#compared-with-pickle))

## Installation

```bash
pip install pysafe-pickle
```

Requires Python 3.10 or newer (tested on 3.10 through 3.15). A single abi3 wheel covers all of these versions; free-threaded builds are not supported. There are no runtime dependencies.

## Quick Start

```python
import datetime as dt
from decimal import Decimal
import pysafe_pickle as psp

data = {"when": dt.date(2026, 10, 28), "price": Decimal("9.99"), "items": [1, 2, 3]}

blob = psp.dumps(data)
print(psp.loads(blob))
# {'when': datetime.date(2026, 10, 28), 'price': Decimal('9.99'), 'items': [1, 2, 3]}
```

`loads` accepts `bytes`, `bytearray` and `memoryview`. `dump(obj, file)` and `load(file)` work on binary file objects.

## Supported types

| Type | Notes |
|------|-------|
| `None`, `bool`, `float`, `complex` | |
| `int` | Any size (arbitrary precision) |
| `str` | Including lone surrogates |
| `bytes`, `bytearray` | |
| `list`, `tuple`, `dict`, `set`, `frozenset` | |
| `datetime.datetime`, `date`, `time`, `timedelta` | `fold` and microseconds kept; `tzinfo` must be a `timezone` or `ZoneInfo` |
| `datetime.timezone`, `zoneinfo.ZoneInfo` | Zone name kept; `ZoneInfo` keys are validated on load |
| `decimal.Decimal` | Exact, including `NaN`, `sNaN`, `Infinity` and `-0` |
| `uuid.UUID`, `fractions.Fraction` | |
| `range`, `slice`, `Ellipsis`, `NotImplemented` | |
| `pathlib` paths | `PurePosixPath`, `PureWindowsPath`, `PosixPath`, `WindowsPath`. A concrete path from another OS loads as the matching `Pure*` class |
| `collections.OrderedDict`, `Counter` | Order kept |
| `collections.defaultdict` | Factory must be `None` or one of `list`, `dict`, `set`, `int`, `float`, `str`, `tuple`, `bytes`, `frozenset`, `bool`, `complex` |
| `collections.deque` | `maxlen` kept |
| dataclass instances | Generated stand-in class by default, your class with `classes=` |
| namedtuples (`collections.namedtuple`, `typing.NamedTuple`) | Generated namedtuple by default, your class with `classes=` |
| `Enum` members (`Enum`, `IntEnum`, `StrEnum`, `Flag`, ...) | Loading requires the class via `classes=` |

Anything else raises `UnsafeTypeError: Unsupported type: ...` (a `TypeError` subclass). That includes subclasses of the types above (other than enums and namedtuples), `defaultdict` with a lambda or custom factory, custom `tzinfo` classes, classes that rely on `__reduce__`/`__getstate__`, NumPy arrays and PyTorch tensors.

Cycles and shared references are preserved through mutable containers (`list`, `dict`, `set`, `bytearray`, `OrderedDict`, `defaultdict`, `Counter`, `deque`) and dataclass instances. A cycle that runs through an immutable container such as a `tuple` raises `DecodeError` on load.

## Your own classes: `classes=`

By default a loaded dataclass is an instance of a generated class with the same name and fields. Pass your classes to get them back for real:

```python
from dataclasses import dataclass, field
import pysafe_pickle as psp

@dataclass
class User:
    name: str
    roles: list = field(default_factory=list)

blob = psp.dumps(User("alice", ["admin"]))

user = psp.loads(blob, classes=[User])
print(type(user) is User, user)  # True User(name='alice', roles=['admin'])

stand_in = psp.loads(blob)  # without classes=
print(isinstance(stand_in, User), stand_in)  # False User(name='alice', roles=['admin'])
```

- `classes=` takes an iterable of classes or a mapping `{"StoredName": cls}`. Classes are matched by their bare `__name__`.
- Dataclass instances are created with `object.__new__` and their fields set directly, so `__init__` and `__post_init__` do not run.
- A field that is missing from the data gets its default (or `default_factory`). A missing field without a default, or a stored field your class does not define, raises `TypeError`.
- Enum members can only be loaded with their class in `classes=`; without it, `loads` raises `UnsafeTypeError`. Namedtuples load as generated namedtuples unless you pass the class.
- `classes=` works the same way on `load()` and `Unpickler`.

## Schema Evolution

Give your dataclass a version, register migrations, and pass the class to `loads`. When the stored version differs from the class's version, the migration chain runs automatically:

```python
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class User:                     # what the class looked like at version 1
    __pysafe_pickle_version__ = 1
    name: str
    age: int

old_blob = psp.dumps(User("Alice", 30))

@dataclass
class User:                     # the current class
    __pysafe_pickle_version__ = 2
    name: str
    age: int
    email: str = ""

@psp.migrate(from_version=1, to_version=2, type_name="User")
def user_v1_to_v2(state: dict) -> dict:
    state["email"] = state["name"].lower() + "@example.com"
    return state

print(psp.loads(old_blob, classes=[User]))
# User(name='Alice', age=30, email='alice@example.com')
```

- `__pysafe_pickle_version__` is a plain class attribute (the legacy `__pygraph_version__` also works). Unversioned classes are version `0`, and version `0` on either side means "never migrate".
- Migrations receive a `dict` of the stored fields and return the new `dict`. Chains (v1 → v2 → v3) are resolved for you; a gap raises `SchemaVersionError`.
- Migrations only run when you pass `classes=`. Without it, `loads` returns a stand-in with exactly the stored fields, and you can call `psp.apply_migrations(...)` yourself.

## Streaming: Pickler / Unpickler

Write several records to one stream and read them back one at a time, just like `pickle`:

```python
import io
import pysafe_pickle as psp

buf = io.BytesIO()
pickler = psp.Pickler(buf)
for event in ["login", "click", "logout"]:
    pickler.dump({"event": event})

buf.seek(0)
unpickler = psp.Unpickler(buf)
while True:
    try:
        print(unpickler.load())
    except EOFError:
        break
# {'event': 'login'}
# {'event': 'click'}
# {'event': 'logout'}
```

`Unpickler` reads ahead in chunks, so the file position may be past the last record it returned. `psp.load(f)` on a seekable file instead leaves the file positioned right after the record, so repeated `load(f)` calls also work. Both raise `EOFError` at the end of the stream. `Pickler(hmac_key=...)` and `Unpickler(hmac_key=..., allowlist=..., classes=...)` take the same options as `dumps` and `loads`.

## Security

pysafe-pickle never calls `__reduce__`, `__setstate__`, `__init__`, `exec`, `eval` or any other code chosen by the data during deserialization, and it never imports a module named in the data. Only the types listed above can be rebuilt. Classes you pass through `classes=` are trusted by you; a few of their hooks can run on load (an `Enum` lookup, for example), but the data cannot name other classes.

### Allowlist

`allowlist` restricts which dataclass, enum and namedtuple **names** may appear in a payload. It must be a `set` of bare class names. Built-in and stdlib types are never checked against it. A rejected name raises `UnsafeTypeError` before any object is built:

```python
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class User:
    name: str
    age: int

data = psp.dumps(User("Alice", 30))

print(psp.loads(data, allowlist={"User"}))  # User(name='Alice', age=30)

try:
    psp.loads(data, allowlist={"Other"})
except psp.UnsafeTypeError as e:
    print(e)  # Type 'User' is not in the allowlist
```

### HMAC signing

When a payload travels through storage or a network you do not control, sign it. The signature is HMAC-SHA256 over the whole payload, checked before anything is decoded:

```python
import pysafe_pickle as psp

KEY = b"load-this-from-your-secret-store"

blob = psp.dumps({"user": "admin"}, hmac_key=KEY)
print(psp.loads(blob, hmac_key=KEY))  # {'user': 'admin'}

tampered = bytearray(blob)
tampered[20] ^= 1
for data, key in [(bytes(tampered), KEY), (blob, b"wrong key"), (blob, None)]:
    try:
        psp.loads(data, hmac_key=key)
    except psp.HMACError as e:
        print(e)
# HMAC verification failed: payload was tampered with or the key is wrong
# HMAC verification failed: payload was tampered with or the key is wrong
# payload is signed; pass hmac_key
```

A signed payload is the normal payload with bit 0 of the header flags set and a 32-byte tag appended. Loading an unsigned payload with a key also raises `HMACError` (`payload is not signed`), so an attacker cannot strip the signature. `dump`, `load`, `Pickler` and `Unpickler` accept `hmac_key` too. HMAC proves the data was produced by someone holding the key; it does not encrypt it.

## Reading existing pickle files

`load_pickle`, `loads_pickle` and `convert_pickle` read data written by the standard `pickle` module (protocols 0-5) without running it. They use their own small opcode interpreter: only a fixed allowlist of safe constructors (builtins, `datetime`, `Decimal`, `Fraction`, `UUID`, `collections` types, paths, `ZoneInfo`) can be called, and anything else raises `UnsafeTypeError` before it is called.

```python
import pickle
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class Point:
    x: int
    y: int

with open("points.pkl", "wb") as f:  # an existing pickle file
    pickle.dump({"origin": Point(0, 0), "size": 3}, f)

print(psp.load_pickle("points.pkl", classes=[Point]))
# {'origin': Point(x=0, y=0), 'size': 3}

# Convert it once to the pysafe-pickle format
blob = psp.convert_pickle("points.pkl", classes=[Point])
print(psp.loads(blob, classes=[Point]))
# {'origin': Point(x=0, y=0), 'size': 3}
```

A malicious pickle is rejected without running anything:

```python
import pickle
import pysafe_pickle as psp

class Exploit:
    def __reduce__(self):
        return (eval, ("__import__('os').system('echo pwned')",))

try:
    psp.loads_pickle(pickle.dumps(Exploit()))
except psp.UnsafeTypeError as e:
    print(e)
# global 'builtins.eval' is not allowed (pass the class via classes= or the pair via allow= if you trust it)
```

Your own classes (dataclasses, plain classes, enums, namedtuples, exceptions) are rebuilt only when listed in `classes=`, and their `__init__`, `__setstate__` and `__reduce__` are never called. `allow=[("module", "name")]` opts in to other globals with full pickle semantics, so only use it for code you trust completely. See the [pickle compatibility guide](https://abdullah-masood-05.github.io/pygraph/guide/pickle-compat) for details.

## Exceptions

| Exception | Also a | Raised when |
|---|---|---|
| `UnsafeTypeError` | `TypeError` | `dumps` meets an unsupported type, an allowlist rejects a type, an enum is loaded without `classes=`, or a pickle file references a global that is not allowed |
| `DecodeError` | `ValueError` | A payload is corrupt, truncated or has bad magic bytes |
| `HMACError` | `ValueError` | Signature check fails, the key is wrong or missing, or a key is given for an unsigned payload |
| `SchemaVersionError` | `ValueError` | No migration path between two versions |

All four subclass `PySafePickleError`.

## Compared with pickle

pysafe-pickle is a safe alternative to `pickle` for data, not a drop-in replacement for every use of it:

- **Payloads are larger.** Every payload carries a string table, a type table and typed records, so output is typically 1.0x to 2.8x the size of pickle protocol 5. Compress it if size matters.
- **`loads` is not always faster.** `dumps` is faster than pickle on most shapes we measure and `loads` is faster on dataclass-heavy data, but `loads` is slower than pickle on some shapes (large dicts, int lists, shared-reference graphs), by roughly 1.2x to 1.3x in our measurements.
- **Fewer types.** Classes that rely on `__reduce__`, `__getstate__` or a custom constructor cannot be serialized, and neither can NumPy arrays or PyTorch tensors (convert them to `bytes` or lists first).
- **Protocol 5 only**, and no out-of-band buffers (`PickleBuffer` is a plain helper, not a serializable value).

## Migration from v1.0.x (`pygraph`)

In v1.0.x, the package was imported as `import pygraph`. Since v1.1.0:
- The canonical import is `import pysafe_pickle as psp`.
- `import pygraph` and `from pygraph.migrations import migrate` keep working through a backward-compatible shim that emits a `FutureWarning`.
- Files written by v1.0.x (magic `PYGR`) still load. New files use the `PSPK` magic header.
- The `pygraph` shim will remain through v1.x and be removed in v2.0.0.

## Benchmarks

```bash
uv run pytest benchmarks/ -v --benchmark-only
```

See the [benchmarks page](https://abdullah-masood-05.github.io/pygraph/benchmarks/performance) for numbers and payload sizes.

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
