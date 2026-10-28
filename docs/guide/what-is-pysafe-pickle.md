# What is pysafe-pickle?

Python's built-in `pickle` module is the standard way to serialize and deserialize arbitrary Python objects. However, it has one fatal flaw: **it is fundamentally insecure**.

<!-- no-run -->
```python
# WARNING: Standard pickle allows arbitrary code execution!
import pickle

# A malicious payload can execute any shell command upon unpickling:
class Malicious:
    def __reduce__(self):
        import os
        return (os.system, ("echo 'System compromised!'",))

pickle.loads(pickle.dumps(Malicious()))
```

According to the official Python documentation:
> *"Warning: The `pickle` module is not secure. Only unpickle data you trust."*

In modern distributed computing, microservices, RPC architectures, and AI model caching, data often travels over networks or originates from untrusted sources.

---

## Why pysafe-pickle?

`pysafe-pickle` was built to give you **Python object graph serialization without the code-execution hazard of pickle**.

### 1. No Code Execution on Load
During deserialization, `pysafe-pickle` **never calls** `__reduce__`, `__reduce_ex__`, `__setstate__`, `__init__`, `__post_init__`, `exec`, or `eval`, and it never imports a module named in the data. Objects are built only by fixed reconstructors for the supported types. Your own classes are used only when you pass them in `classes=`, and dataclass instances are created without calling `__init__`.

### 2. A Fixed Set of Supported Types {#supported-types}
Only the following types can be serialized and deserialized:
- `None`, `bool`, `int` (arbitrary size), `float`, `complex`
- `str` (including strings with lone surrogates), `bytes`, `bytearray`
- `list`, `tuple`, `dict`, `set`, `frozenset`
- `datetime.datetime`, `date`, `time`, `timedelta`, `timezone`, `zoneinfo.ZoneInfo`
- `decimal.Decimal`, `uuid.UUID`, `fractions.Fraction`
- `range`, `slice`, `Ellipsis`, `NotImplemented`
- `pathlib` paths (`PurePosixPath`, `PureWindowsPath`, `PosixPath`, `WindowsPath`)
- `collections.OrderedDict`, `Counter`, `deque` (with `maxlen`), `defaultdict` (factory `None` or a builtin type such as `list`, `int`, `set`)
- `@dataclass` instances, namedtuples, and `Enum` members (`IntEnum`, `StrEnum`, `Flag`, ...)

Anything else (subclasses of the types above other than enums and namedtuples, custom `tzinfo` classes, non-dataclass custom classes, functions, NumPy arrays, PyTorch tensors, ...) raises `UnsafeTypeError: Unsupported type: ...` at `dumps` time. You can additionally pass an `allowlist` to `loads` to restrict which dataclass, enum and namedtuple names may appear; see [Security & Allowlist](/guide/security-and-allowlist).

### 3. Rust Core, Honest Performance
Graph traversal, cycle detection, binary encoding, and decoding are implemented in **Rust** via PyO3. Compared with stdlib `pickle` (protocol 5):
- `dumps` is faster on most data shapes, and much faster on dataclass-heavy payloads.
- `loads` is faster on dataclass-heavy data, roughly on par on many shapes, and **slower on some** (large dicts, int lists, shared-reference graphs).
- Serialized payloads are **larger** than pickle's (typically 1.0x to 2.8x), since there is no compression and each payload carries a string table and a type table.

Results depend on the data shape. See [Benchmarks](/benchmarks/performance) and measure with your own data.

### 4. Schema Versions and Migrations
Dataclasses can declare `__pysafe_pickle_version__`, which is stored in the payload, and you register migration functions with `@psp.migrate`. When you load with `classes=[YourClass]` and the stored version differs from the class's version, the migrations run automatically. See [Schema Evolution](/guide/schema-evolution).

### 5. Integrity
`dumps(obj, hmac_key=key)` appends an HMAC-SHA256 tag, and `loads(data, hmac_key=key)` verifies it before decoding anything. See [Security & Allowlist](/guide/security-and-allowlist#hmac).

### 6. A Way Out of Existing Pickle Files
`psp.load_pickle` reads standard `.pkl` files with a restricted interpreter that only calls known-safe constructors, and `psp.convert_pickle` turns them into pysafe-pickle payloads. See [Reading Existing Pickle Files](/guide/pickle-compat).

---

## Things to Know

- **Pass `classes=` to get your classes back.** Without it, a loaded dataclass is an instance of a generated class with the same `__name__` and fields, and `isinstance(obj, User)` is `False`. Enum members need `classes=` to load at all.
- **`__init__` and `__post_init__` are not called** on load, so validation in them does not run. Missing fields get their dataclass defaults.
- **`ClassVar` and `InitVar` pseudo-fields are not serialized.**
- **Protocol 5 only.** `dumps(..., protocol=4)` raises `ValueError`.
- **No NumPy / PyTorch / zero-copy support**, and classes that need `__reduce__` or a custom constructor cannot be serialized.
- **Payloads are larger than pickle's**, and `loads` is slower than pickle on some shapes.
- **Free-threaded Python builds are not supported.**

---

## Comparison Table

| Feature | Standard `pickle` | JSON / msgpack | `pysafe-pickle` |
| :--- | :---: | :---: | :---: |
| **Arbitrary Code Execution Risk on Load** | ⚠️ **High** | ✅ None | ✅ **None** |
| **Circular Reference Handling** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Shared Object References** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Python `@dataclass` Support** | ✅ Yes | ⚠️ Manual | ✅ Yes (your class with `classes=`) |
| **`datetime`, `Decimal`, `UUID`, enums** | ✅ Yes | ⚠️ Manual | ✅ Yes |
| **Arbitrary Custom Classes** | ✅ Yes | ❌ No | ❌ **No** (dataclasses, enums, namedtuples) |
| **Schema Versions / Migrations** | ❌ No | ❌ No | ✅ Yes (automatic with `classes=`) |
| **PEP 574 Out-of-band Buffers** | ✅ Yes | ❌ No | ❌ **No** |
| **Tamper Detection (HMAC)** | ❌ No | ❌ No | ✅ **Yes** (HMAC-SHA256) |
| **Reads Existing `.pkl` Files** | ✅ (unsafely) | ❌ No | ✅ Safely, allowlisted types only |
| **Payload Size** | Smaller | Varies | Larger than pickle |
| **Speed / Language Core** | C / Python | C / Rust | **Rust (PyO3)** |
