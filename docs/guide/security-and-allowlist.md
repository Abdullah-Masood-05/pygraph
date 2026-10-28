# Security & Allowlist

`pysafe-pickle` was engineered with a security-first architecture. This page covers what loading untrusted data can and cannot do, how to restrict which types a payload may contain, and how to detect tampering.

---

## 1. No Code Execution on Load

Unlike standard Python `pickle`, `pysafe-pickle` **never** calls, during deserialization:
- `__reduce__` or `__reduce_ex__`
- `__getstate__` or `__setstate__`
- `__init__` or `__post_init__` of your classes
- `exec()` or `eval()`

and it never imports a module named in the payload. Only the [supported types](/guide/what-is-pysafe-pickle#supported-types) can be encoded at all. When an object is loaded, the Rust engine reads the explicit type metadata and rebuilds built-in and stdlib values directly. Dataclass instances are created with `object.__new__` and their fields set directly.

Your own classes are only used when you list them in `classes=`. That is a decision you make in code, not something the payload can request. A listed class can still run a little of its own code on load: enum members are looked up with `Cls[name]` or `Cls(value)`, and namedtuples are built with `_make`. Treat the classes you list as trusted code.

::: warning Not a substitute for resource limits
Decoding is bounded against malformed input: every count and index is checked against the payload before anything is allocated, and corrupt data raises `DecodeError`. Still cap the size of payloads you accept from untrusted sources.
:::

---

## 2. Type Allowlisting

In multi-tenant or untrusted environments you can restrict which **class names** may appear in a payload. Pass a `set` of names to `loads`, `load`, or `Unpickler(allowlist=...)`. It applies to dataclasses, enums and namedtuples, and it is checked before any object is built:

```python
import pysafe_pickle as psp
from dataclasses import dataclass

@dataclass
class Record:
    token: str

data = psp.dumps(Record("secret"))

print(psp.loads(data, allowlist={"Record"}, classes=[Record]))  # Record(token='secret')

try:
    psp.loads(data, allowlist={"Other"})
except psp.UnsafeTypeError as e:
    print(e)  # Type 'Record' is not in the allowlist
```

Details to be aware of:
- The allowlist must be a `set` (a `list` raises `TypeError`).
- Entries are matched against the bare class `__name__` (for example `"Record"`), not a module-qualified path.
- Built-in and stdlib types (`dict`, `datetime`, `Decimal`, `OrderedDict`, ...) are always allowed. An empty set still permits them.
- A rejected type raises `UnsafeTypeError`, which is a `TypeError` subclass, so older `except TypeError` code still catches it.
- Names are all that is checked, so two classes with the same name are indistinguishable to the allowlist. Combine it with `classes=` to control exactly which classes are instantiated.

---

## 3. Tamper Detection (HMAC) {#hmac}

If a payload passes through storage or a network you do not fully control, sign it. With `hmac_key`, `dumps` appends an HMAC-SHA256 tag computed over the entire payload, and `loads` checks it, in constant time, **before** decoding anything:

```python
import pysafe_pickle as psp

KEY = b"load-this-from-your-secret-store"

blob = psp.dumps({"user": "admin", "role": "viewer"}, hmac_key=KEY)
print(psp.loads(blob, hmac_key=KEY))  # {'user': 'admin', 'role': 'viewer'}

forged = blob.replace(b"viewer", b"owner!")
try:
    psp.loads(forged, hmac_key=KEY)
except psp.HMACError as e:
    print(e)  # HMAC verification failed: payload was tampered with or the key is wrong
```

The rules are strict on purpose:

| Payload | Key passed to `loads` | Result |
| :--- | :--- | :--- |
| signed | correct key | Loaded |
| signed | wrong key, or payload modified/truncated | `HMACError: HMAC verification failed: ...` |
| signed | none | `HMACError: payload is signed; pass hmac_key` |
| unsigned | any key | `HMACError: payload is not signed` |
| unsigned | none | Loaded (no integrity check) |

The last-but-one row means an attacker cannot strip the signature and send unsigned data to code that expects a signed payload. `HMACError` is a `ValueError` subclass. An empty key raises `ValueError` in both `dumps` and `loads`; any bytes-like key (`bytes`, `bytearray`, `memoryview`) works.

`hmac_key` is accepted by `dumps`, `loads`, `dump`, `load`, `Pickler` and `Unpickler`. In a stream, every record is signed and verified separately.

Things HMAC does **not** do:
- It does not encrypt; anyone can still read the payload.
- It does not stop replay of an old, validly signed payload. Put a timestamp or nonce inside the data if that matters.
- Its security rests entirely on the key. Keep it out of source control and rotate it like any other secret.

---

## 4. Reading Untrusted `.pkl` Files

Existing standard pickle files can be read with `psp.load_pickle`, which uses a restricted interpreter instead of `pickle.load`. See [Reading Existing Pickle Files](/guide/pickle-compat).
