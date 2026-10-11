# Exceptions Reference

All custom exceptions defined by `pysafe-pickle` inherit from `PySafePickleError`.

::: warning Which exceptions are actually raised
In the current release the core functions raise built-in exceptions (`TypeError`, `ValueError`, `NotImplementedError`, `RecursionError`), not the classes below. See the table at the end of this page. The custom classes are exported for compatibility and for your own `except` clauses, but nothing in the library raises them yet.
:::

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
*(Alias: `PyGraphError`, the same object)*

Base exception for all pysafe-pickle exception classes.

---

## `UnsafeTypeError`

Intended for an unallowlisted type encountered during deserialization. Currently, allowlist violations raise `TypeError` instead.

---

## `SchemaVersionError`

Intended for a version mismatch that cannot be resolved by any registered migration chain. Currently, a missing migration path raises `ValueError` from `get_migration_chain` / `apply_migrations`.

---

## `HMACError`

Intended for HMAC-SHA256 verification failures. HMAC signing is not implemented yet: `dumps(..., hmac_key=...)` raises `NotImplementedError`.

---

## What is raised today

| Situation | Exception | Message |
|---|---|---|
| Unsupported type (function, subclass, ...) | `TypeError` | `Unsupported type: <name>` |
| Dataclass name not in `allowlist` | `TypeError` | `Type '<name>' is not in the allowlist` |
| Non-bytes input to `loads` | `TypeError` | `a bytes-like object is required, not '<type>'` |
| `protocol != 5` | `ValueError` | `Unsupported protocol N; only protocol 5 is supported` |
| `hmac_key` passed to `dumps` | `NotImplementedError` | `HMAC signatures are not yet supported` |
| Corrupt / truncated payload, bad magic | `ValueError` | e.g. `Invalid magic bytes` |
| Cycle through `tuple` / `frozenset` on load | `ValueError` | `Cyclic reference involving immutable container ...` |
| No migration path | `ValueError` | `No migration path from version 1 to 2 for type '...'` |
