# Security & Allowlist

`pysafe-pickle` was engineered with a security-first architecture.

---

## 1. No Code Execution on Load

Unlike standard Python `pickle`, `pysafe-pickle` **never** calls:
- `__reduce__` or `__reduce_ex__`
- `__getstate__` or `__setstate__`
- `exec()` or `eval()`

Only a fixed set of types can be encoded at all (see [supported types](/guide/what-is-pysafe-pickle#supported-types)). When an object is loaded, the Rust engine reads the explicit type metadata and rebuilds built-in containers directly. Dataclass instances are created with `object.__new__` and their fields set directly, so your `__init__` and `__post_init__` never run.

::: warning Not a substitute for resource limits
Decoding is bounded against malformed input (truncated data, bad indices, and so on raise `ValueError`), but you should still cap the size of payloads you accept from untrusted sources.
:::

---

## 2. Type Allowlisting

In multi-tenant or untrusted environments, you can restrict which **dataclass names** may appear in a payload. Pass a `set` of names to `loads`, `load`, or `Unpickler(allowlist=...)`:

```python
import pysafe_pickle as psp
from dataclasses import dataclass

@dataclass
class Record:
    token: str

@dataclass
class Other:
    value: int

data = psp.dumps(Record("secret"))

# Allowed: the dataclass name is in the set
print(psp.loads(data, allowlist={"Record"}))   # Record(token='secret')

# Rejected: "Other" is not the type in the payload
try:
    psp.loads(data, allowlist={"Other"})
except TypeError as e:
    print(e)    # Type 'Record' is not in the allowlist
```

Details to be aware of:
- The allowlist must be a `set` (a `list` raises `TypeError`).
- Entries are matched against the dataclass `__name__` (for example `"Record"`), not a module-qualified path.
- Built-in types (`dict`, `list`, `int`, ...) are always allowed. Entries like `"builtins.dict"` are accepted in the set but have no effect, and an empty set still permits built-in data.
- A rejected type raises `TypeError`, not `UnsafeTypeError` (that exception class exists for compatibility but is not raised by the current decoder).
- Dataclasses are matched by name only, so two different classes with the same name are indistinguishable to the allowlist.

---

## 3. Tamper Detection (HMAC): Not Yet Supported

The `hmac_key` parameter exists on `dumps` (and on `Pickler`) as a reserved placeholder, but HMAC signing and verification are **not implemented**. Passing a key raises an error rather than silently producing unsigned data:

```python
import pysafe_pickle as psp

try:
    psp.dumps({"user": "admin"}, hmac_key=b"secret")
except NotImplementedError as e:
    print(e)    # HMAC signatures are not yet supported
```

`loads` does not accept `hmac_key`. `psp.HMACError` is defined but is not raised today. If you need integrity protection now, sign the bytes yourself:

```python
import hmac, hashlib
import pysafe_pickle as psp

KEY = b"your-secret-key"
blob = psp.dumps({"user": "admin"})
tag = hmac.new(KEY, blob, hashlib.sha256).digest()

# ... transmit blob + tag ...

assert hmac.compare_digest(tag, hmac.new(KEY, blob, hashlib.sha256).digest())
data = psp.loads(blob)
```
