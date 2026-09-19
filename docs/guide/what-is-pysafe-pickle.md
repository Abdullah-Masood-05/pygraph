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

`pysafe-pickle` was built to solve this dilemma: providing the **expressiveness and convenience of Python object graph serialization** without the security hazards.

### 1. Zero Arbitrary Code Execution
During deserialization (`loads`), `pysafe-pickle` **never calls** `__reduce__`, `__reduce_ex__`, `__setstate__`, `exec`, or `eval`. Deserialization constructs objects only through safe, controlled internal reconstructors.

### 2. Allowlisted Types by Default
By default, only primitive data types and registered `@dataclass` structures can be deserialized:
- `None`, `bool`, `int`, `float`, `str`, `bytes`
- `list`, `tuple`, `dict`, `set`, `frozenset`
- Any Python `@dataclass` instance

Any unregistered or disallowed type encountered during deserialization raises `UnsafeTypeError`.

### 3. Native Rust Performance
The object graph traversal, cycle detection, binary encoding, and decoding are powered by **Rust** via PyO3. This delivers performance superior to pure-Python alternatives while enforcing memory safety.

### 4. Schema Evolution
Production systems evolve. Fields get added, removed, or renamed. `pysafe-pickle` has first-class support for **schema versioning and migration chains**, allowing old data to be automatically upgraded to newer dataclass schemas at load time.

---

## Comparison Table

| Feature | Standard `pickle` | JSON / msgpack | `pysafe-pickle` |
| :--- | :---: | :---: | :---: |
| **Arbitrary Code Execution Risk** | ⚠️ **High** | ✅ None | ✅ **None** |
| **Circular Reference Handling** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Shared Object References** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Python `@dataclass` Support** | ✅ Yes | ⚠️ Manual | ✅ **Native** |
| **Schema Evolution / Migrations** | ❌ No | ❌ No | ✅ **Built-in** |
| **PEP 574 Out-of-band Buffers** | ✅ Yes | ❌ No | ✅ **Yes** |
| **Tamper Detection (HMAC)** | ❌ No | ❌ No | ✅ **Built-in** |
| **Speed / Language Core** | C / Python | C / Rust | **Rust (PyO3)** |
