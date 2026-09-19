# Security & Allowlist

`pysafe-pickle` was engineered with a security-first architecture.

---

## 1. Zero Arbitrary Code Execution

Unlike standard Python `pickle`, `pysafe-pickle` **never** calls:
- `__reduce__` or `__reduce_ex__`
- `__getstate__` or `__setstate__`
- `exec()` or `eval()`

When an object is loaded, the Rust engine inspects the explicit type metadata and reconstructs objects through registered factories.

---

## 2. Strict Type Allowlisting

In multi-tenant or untrusted environments, you can enforce an explicit allowlist of permitted types.

```python
import pysafe_pickle as psp

# Restrict deserialization to dictionaries and lists
allowed = {"builtins.dict", "builtins.list"}

valid_data = psp.dumps({"key": "value"})
obj = psp.loads(valid_data, allowlist=allowed)  # OK!

# If an unauthorized dataclass or type is present:
from dataclasses import dataclass

@dataclass
class UnauthorizedRecord:
    token: str

unauthorized_data = psp.dumps(UnauthorizedRecord("secret"))

# Raises TypeError: Type 'UnauthorizedRecord' is not in the allowlist
psp.loads(unauthorized_data, allowlist=allowed)
```

---

## 3. Cryptographic Tamper Detection (HMAC)

To guarantee that serialized bytes have not been tampered with or modified in transit, provide an `hmac_key`:

```python
import pysafe_pickle as psp

SECRET_KEY = b"your-super-secret-key-32-bytes"

# Serialize with HMAC-SHA256 signature embedded
signed_blob = psp.dumps({"user": "admin", "role": "root"}, hmac_key=SECRET_KEY)

# Deserialization validates the HMAC signature before decoding
data = psp.loads(signed_blob, hmac_key=SECRET_KEY)

# Tampering with even a single byte will trigger HMACError
tampered_blob = bytearray(signed_blob)
tampered_blob[-1] ^= 0xFF

try:
    psp.loads(bytes(tampered_blob), hmac_key=SECRET_KEY)
except psp.HMACError:
    print("Tamper detected! Payload rejected.")
```
