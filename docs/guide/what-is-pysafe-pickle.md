# What is pysafe-pickle?

Python's built-in `pickle` module is the standard way to serialize and deserialize arbitrary Python objects. However, it has one fatal flaw: **it is fundamentally insecure**.

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
During deserialization (`loads`), `pysafe-pickle` **never calls** `__reduce__`, `__reduce_ex__`, `__setstate__`, `exec`, or `eval`. Objects are constructed only by built-in reconstructors, and dataclass instances are created without calling your `__init__`.

### 2. A Fixed Set of Supported Types {#supported-types}
Only the following types can be serialized and deserialized:
- `None`, `bool`, `int` (arbitrary size), `float`, `complex`
- `str` (including strings with lone surrogates), `bytes`, `bytearray`
- `list`, `tuple`, `dict`, `set`, `frozenset`
- Any Python `@dataclass` instance (not subclasses of the built-in containers)

Anything else (custom classes that are not dataclasses, `datetime`, `Decimal`, `OrderedDict`, functions, NumPy arrays, PyTorch tensors, ...) raises `TypeError: Unsupported type: ...` at `dumps` time. You can additionally pass an `allowlist` to `loads` to restrict which dataclass names may appear; see [Security & Allowlist](/guide/security-and-allowlist).

### 3. Rust Core, Honest Performance
Graph traversal, cycle detection, binary encoding, and decoding are implemented in **Rust** via PyO3. Compared with stdlib `pickle` (protocol 5):
- `dumps` is faster on most data shapes, and much faster on dataclass-heavy payloads.
- `loads` is roughly on par, and slower on some shapes.
- Serialized payloads are **larger** than pickle's, since there is no compression and each payload carries a type table.

Results depend on the data shape. See [Benchmarks](/benchmarks/performance) and measure with your own data.

### 4. Schema Versions and Migrations
Dataclasses can declare `__pysafe_pickle_version__`, which is stored in the payload, and you can register migration functions with `@psp.migrate`. Note that migrations are currently applied by calling `psp.apply_migrations(...)` yourself; `loads` does not run them automatically. See [Schema Evolution](/guide/schema-evolution) for the exact behavior.

### 5. Cycles and Shared References
Cyclic and shared references are preserved, including self-referential dataclasses.

---

## Things to Know

- **Loaded dataclasses are dynamic classes.** `loads` returns instances of a generated class with the same `__name__` and field names, not your original class. `isinstance(obj, User)` is `False`, and the loaded object does not compare equal to the original. Attribute access, `repr`, `==`, and `hash` between loaded objects work as expected.
- **Dataclass `__init__` is not called**, so defaults, `__post_init__`, and validation do not run on load.
- **`ClassVar` and `InitVar` pseudo-fields are not serialized.**
- Loaded dataclass instances can be passed back to `dumps` (since v1.3.2), but they are still instances of a generated class, not your original one.
- **Protocol 5 only.** `dumps(..., protocol=4)` raises `ValueError`.
- **No HMAC yet.** `hmac_key` is accepted by the signature but raises `NotImplementedError`.
- **No NumPy / PyTorch / zero-copy support.**

---

## Comparison Table

| Feature | Standard `pickle` | JSON / msgpack | `pysafe-pickle` |
| :--- | :---: | :---: | :---: |
| **Arbitrary Code Execution Risk on Load** | ⚠️ **High** | ✅ None | ✅ **None** |
| **Circular Reference Handling** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Shared Object References** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Python `@dataclass` Support** | ✅ Yes | ⚠️ Manual | ✅ Yes (rebuilt as a dynamic class) |
| **Arbitrary Custom Classes** | ✅ Yes | ❌ No | ❌ **No** (dataclasses only) |
| **Schema Versions / Migrations** | ❌ No | ❌ No | ⚠️ Versions embedded; migrations applied manually |
| **PEP 574 Out-of-band Buffers** | ✅ Yes | ❌ No | ❌ **Not yet** |
| **Tamper Detection (HMAC)** | ❌ No | ❌ No | ❌ **Not yet** |
| **Payload Size** | Smaller | Varies | Larger than pickle |
| **Speed / Language Core** | C / Python | C / Rust | **Rust (PyO3)** |
