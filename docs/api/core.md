# Core Functions API Reference

The primary serialization and deserialization functions provided by `pysafe_pickle`. For reading standard `pickle` data, see [Pickle Compatibility](./pickle-compat.md).

---

## Supported types

| Type | Notes |
| :--- | :--- |
| `None`, `bool`, `int` (any size), `float`, `complex` | |
| `str` (including lone surrogates), `bytes`, `bytearray` | |
| `list`, `tuple`, `dict`, `set`, `frozenset` | |
| `datetime.datetime`, `date`, `time`, `timedelta` | `fold` kept; `tzinfo` must be `None`, a `timezone` or a `ZoneInfo` |
| `datetime.timezone`, `zoneinfo.ZoneInfo` | `timezone` name kept; `ZoneInfo` is rebuilt from its key, which is validated on load |
| `decimal.Decimal`, `uuid.UUID`, `fractions.Fraction` | `Decimal` stored as `str(d)` (exact, specials included) |
| `range`, `slice`, `Ellipsis`, `NotImplemented` | |
| `pathlib.PurePosixPath`, `PureWindowsPath`, `PosixPath`, `WindowsPath` | A concrete path loads as the concrete class on a matching OS, otherwise as the `Pure*` class |
| `collections.OrderedDict`, `Counter`, `deque` | Order and `maxlen` kept |
| `collections.defaultdict` | `default_factory` must be `None`, `list`, `dict`, `set`, `int`, `float`, `str`, `tuple`, `bytes`, `frozenset`, `bool` or `complex` |
| dataclass instances | Including frozen and slotted ones |
| namedtuple instances | `collections.namedtuple` and `typing.NamedTuple` |
| `Enum` members | Any `Enum` subclass (`IntEnum`, `StrEnum`, `Flag`, ...). Loading requires `classes=` |

Other objects raise `UnsafeTypeError: Unsupported type: <name>`. This includes subclasses of the types above (except enums and namedtuples), a `defaultdict` with any other factory (`Unsupported type: defaultdict with factory ...`), a datetime with a custom `tzinfo`, `PickleBuffer`, functions and arbitrary class instances.

Graph behavior:
- Shared identity and cycles are preserved through `list`, `dict`, `set`, `bytearray`, `OrderedDict`, `defaultdict`, `Counter`, `deque` and dataclass instances (including dataclass self-cycles).
- A cycle that passes through an immutable container (`tuple`, `frozenset`, namedtuple, ...) serializes, but raises `DecodeError` on `loads`.
- Only `dataclasses.fields()` are serialized, so `ClassVar` and `InitVar` pseudo-fields are skipped.
- Enum members are stored by member name (or by value for composite `Flag` values), namedtuples by their `_fields` and values.

### Loaded classes and `classes=` {#classes}

Without `classes=`, dataclass instances are rebuilt as instances of a generated class with the same name and field names (it is *not* your class, `isinstance(obj, User)` is `False`), and namedtuples as generated `collections.namedtuple` classes. Enum members cannot be rebuilt without their class and raise `UnsafeTypeError: Unsupported type: enum Color (pass it via classes=)`.

With `classes=`, the listed classes are used instead. `classes=` is an iterable of classes (matched by bare `__name__`) or a mapping `{"StoredName": cls}`. Two different classes with the same name in an iterable raise `ValueError`; use a mapping to disambiguate.

- **Dataclasses** are created with `object.__new__(cls)` and fields set with `object.__setattr__`, so `__init__`, `__post_init__` and `__setattr__` overrides do not run (pickle semantics; frozen and slotted dataclasses work). Fields missing from the data get their `default`/`default_factory`. A missing field without a default, or a stored field the class does not define, raises `TypeError`. If the stored schema version differs from the class's, migrations run first (see [Schema Migrations](./migrations.md)).
- **Namedtuples** with the same fields are rebuilt directly; otherwise values are mapped by field name, absent fields take the class's defaults, and unknown stored fields raise `TypeError`.
- **Enums** are looked up by member name (`Cls[name]`) or by value (`Cls(value)`). An unknown member raises `ValueError`.
- A class of the wrong kind (for example a non-dataclass for dataclass data) raises `TypeError`.

---

## `dumps(obj, *, protocol=5, schema_version=None, hmac_key=None) -> bytes` {#dumps}

Serializes a Python object graph into bytes.

### Parameters:
- **`obj`** (*Any*): The object to serialize.
- **`protocol`** (*int*, optional): Only `5` is supported (default: `5`).
- **`schema_version`** (*int*, optional): Accepted for compatibility but ignored. The schema version written for each dataclass comes from its class attribute `__pysafe_pickle_version__` (or legacy `__pygraph_version__`).
- **`hmac_key`** (*bytes-like*, optional): If given, the payload is signed with HMAC-SHA256: the header's HMAC flag is set and a 32-byte tag over all preceding bytes is appended. Must be non-empty.

### Returns:
- **`bytes`**: A payload starting with the `PSPK` magic bytes. Output is deterministic for equal inputs.

### Raises:
- **`UnsafeTypeError`** (a `TypeError`): The graph contains an unsupported type (`Unsupported type: function`).
- **`ValueError`**: `protocol` is not `5` (`Unsupported protocol 4; only protocol 5 is supported`), `hmac_key` is empty (`hmac_key must be a non-empty bytes-like object`), or a size exceeds the format's limits.
- **`TypeError`**: `hmac_key` is not bytes-like.
- **`RecursionError`**: The graph is nested too deeply.

### Example:
```python
import pysafe_pickle as psp

payload = psp.dumps({"user": "alice", "active": True})
signed = psp.dumps({"user": "alice", "active": True}, hmac_key=b"secret")
print(len(signed) - len(payload))  # 32
```

---

## `loads(data, *, allowlist=None, hmac_key=None, classes=None) -> Any` {#loads}

Deserializes one payload. Bytes after the end of the payload are ignored.

### Parameters:
- **`data`** (*bytes | bytearray | memoryview*): The payload (both `PSPK` and legacy `PYGR` headers are accepted). Any object supporting the buffer protocol works; non-`bytes` input is copied first.
- **`allowlist`** (*set[str]*, optional): Bare names of the dataclass, enum and namedtuple types the payload may contain (e.g. `{"User", "Color"}`). It must be a real `set`. Built-in and stdlib types are always allowed. Checked before any object is built.
- **`hmac_key`** (*bytes-like*, optional): Verify the payload's HMAC-SHA256 tag with this key before decoding. Required for signed payloads, rejected for unsigned ones.
- **`classes`** (*Iterable[type] | Mapping[str, type]*, optional): Your classes to rebuild dataclasses, enums and namedtuples with. See [Loaded classes and `classes=`](#classes).

### Returns:
- **`Any`**: The reconstructed object graph.

### Raises:
- **`UnsafeTypeError`** (a `TypeError`): A type name is not in `allowlist` (`Type 'User' is not in the allowlist`), or an enum is loaded without its class.
- **`HMACError`** (a `ValueError`): The tag does not match (`HMAC verification failed: payload was tampered with or the key is wrong`), the payload is signed but no key was given (`payload is signed; pass hmac_key`), or a key was given for an unsigned payload (`payload is not signed`). A truncated signed payload also raises `HMACError`.
- **`DecodeError`** (a `ValueError`): The payload is corrupt or truncated (e.g. `Invalid magic bytes`, `Invalid data: too short for header`), or a cycle passes through an immutable container.
- **`SchemaVersionError`** (a `ValueError`): With `classes=`, no migration path exists between the stored and current versions.
- **`TypeError`**: `data` is not bytes-like (`a bytes-like object is required, not 'str'`), `allowlist` is not a `set`, `classes=` is malformed, or the data does not fit a class in `classes=` (missing or unknown fields).
- **`ValueError`**: Empty `hmac_key`, an unknown enum member, or an invalid value in the data (for example a bad `ZoneInfo` key).
- **`RecursionError`**: The payload nesting is too deep.

### Example:
```python
import pysafe_pickle as psp

data = psp.dumps({"n": 10**40, "raw": bytearray(b"ab"), "z": 1 + 2j})
print(psp.loads(memoryview(data)))
# {'n': 10000000000000000000000000000000000000000, 'raw': bytearray(b'ab'), 'z': (1+2j)}
```

---

## `dump(obj, file, **kwargs) -> None` {#dump}

Serializes `obj` and writes the bytes to `file` with a single `.write()` call.

### Parameters:
- **`obj`** (*Any*): The object to serialize.
- **`file`** (*BinaryIO*): A writable binary stream.
- **`hmac_key`** (*bytes-like*, optional, keyword): Sign the payload, as in `dumps`.

Other keyword arguments are accepted and ignored. In particular `dump` does not validate `protocol`, and a misspelled `hmac_key` is silently ignored (the record is written unsigned), so double-check the keyword name.

---

## `load(file, *, allowlist=None, hmac_key=None, classes=None) -> Any` {#load}

Reads one payload from a binary file object.

### Parameters:
- **`file`** (*BinaryIO*): A readable binary stream. `load` calls `file.read()` once (the payload format has no length prefix), decodes the first payload, and, if the file is seekable, seeks back so the file is positioned right after that payload. Repeated `load(f)` calls therefore return successive records. On a non-seekable stream the bytes after the first payload are lost; use [`Unpickler`](./streaming.md) there. Because every call reads the whole rest of the file into memory, reading many records from a large file is faster with `Unpickler`.
- **`allowlist`**, **`hmac_key`**, **`classes`**: Same as `loads`.

### Raises:
- **`EOFError`**: The stream is empty (`Ran out of input`).
- **`TypeError`**: `file.read()` does not return a bytes-like object (`file.read() did not return bytes: ...`).
- Everything `loads` raises.

### Example:
```python
import io
import pysafe_pickle as psp

f = io.BytesIO()
psp.dump({"epoch": 1}, f)
psp.dump({"epoch": 2}, f)
f.seek(0)
print(psp.load(f), psp.load(f))  # {'epoch': 1} {'epoch': 2}
```

---

## Reading standard pickle data

`load_pickle`, `loads_pickle` and `convert_pickle` are documented in [Pickle Compatibility](./pickle-compat.md).
