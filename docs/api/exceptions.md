# Exceptions Reference

All custom exceptions raised by `pysafe-pickle` inherit from `PySafePickleError`.

---

## Hierarchy

```text
Exception
 └── PySafePickleError (alias: PyGraphError)
      ├── UnsafeTypeError
      ├── SchemaVersionError
      └── HMACError
```

---

## `PySafePickleError`
*(Alias: `PyGraphError`)*

Base exception for all errors raised by `pysafe-pickle`.

---

## `UnsafeTypeError`

Raised during deserialization when an object type is not in the allowlist or is an unsupported Python type (e.g. lambdas, arbitrary executable functions, system modules).

---

## `SchemaVersionError`

Raised when a version mismatch between serialized data and the runtime class definition cannot be resolved by any registered migration chain.

---

## `HMACError`

Raised when cryptographic HMAC-SHA256 signature verification fails, indicating that the serialized data has been altered, truncated, or tampered with.
