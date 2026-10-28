# Exceptions Reference

All custom exceptions inherit from `PySafePickleError`. Since v1.4.0 the library raises them itself. Each one also subclasses the built-in exception that was raised before (`TypeError` or `ValueError`), so `except TypeError:` and `except ValueError:` clauses written for older versions keep working.

---

## Hierarchy

```text
Exception
 └── PySafePickleError (alias: PyGraphError)
      ├── UnsafeTypeError     (also a TypeError)
      ├── DecodeError         (also a ValueError)
      ├── HMACError           (also a ValueError)
      └── SchemaVersionError  (also a ValueError)
```

All of them are importable from the package root (`psp.UnsafeTypeError`) and from `pysafe_pickle.exceptions`.

---

## `PySafePickleError`
*(Alias: `PyGraphError`, the same object)*

Base class. `except psp.PySafePickleError` catches every error the library raises on purpose. It does not catch plain `TypeError`/`ValueError` raised for bad arguments, or the errors listed under [Other exceptions](#other-exceptions).

---

## `UnsafeTypeError`

Subclass of `PySafePickleError` and `TypeError`. Raised when:
- `dumps`/`dump`/`Pickler.dump` meets a type it cannot serialize (`Unsupported type: function`, `Unsupported type: defaultdict with factory <lambda>`).
- A type name in the payload is not in `allowlist` (`Type 'User' is not in the allowlist`).
- An enum member is loaded without its class (`Unsupported type: enum Color (pass it via classes=)`).
- The [pickle reader](./pickle-compat.md) meets a global or operation that is not allowed (`global 'builtins.eval' is not allowed ...`).

---

## `DecodeError`

Subclass of `PySafePickleError` and `ValueError`. Raised when a payload is corrupt, truncated or otherwise invalid: bad magic bytes, counts larger than the data, out-of-range references, unknown tags, or a reference cycle through an immutable container. `Unpickler.load()` raises it when a stream ends in the middle of a record.

---

## `HMACError`

Subclass of `PySafePickleError` and `ValueError`. Raised by `loads`, `load` and `Unpickler.load` when:
- The tag does not match: the payload was modified, truncated, or the key is wrong (`HMAC verification failed: payload was tampered with or the key is wrong`).
- The payload is signed but no `hmac_key` was given (`payload is signed; pass hmac_key`).
- An `hmac_key` was given but the payload is not signed (`payload is not signed`). This also catches an attacker clearing the HMAC flag and stripping the tag.

---

## `SchemaVersionError`

Subclass of `PySafePickleError` and `ValueError`. Raised by `get_migration_chain` / `apply_migrations`, and therefore by `loads(..., classes=...)` when it migrates, if no registered chain leads from the stored version to the current one (`No migration path from version 1 to 2 for type 'User'`).

---

## Example

```python
import pysafe_pickle as psp

try:
    psp.dumps(lambda: 0)
except psp.UnsafeTypeError as e:
    print(e, isinstance(e, TypeError))  # Unsupported type: function True

try:
    psp.loads(b"not a payload")
except psp.DecodeError as e:
    print(e, isinstance(e, ValueError))  # Invalid magic bytes True
```

---

## What is raised where

| Situation | Exception | Message |
|---|---|---|
| Unsupported type in `dumps` | `UnsafeTypeError` | `Unsupported type: <name>` |
| Type name not in `allowlist` | `UnsafeTypeError` | `Type '<name>' is not in the allowlist` |
| Enum loaded without `classes=` | `UnsafeTypeError` | `Unsupported type: enum <name> (pass it via classes=)` |
| Pickle reader: global not allowed | `UnsafeTypeError` | `global '<module>.<name>' is not allowed ...` |
| Corrupt / truncated payload, bad magic | `DecodeError` | e.g. `Invalid magic bytes` |
| Cycle through `tuple` / `frozenset` on load | `DecodeError` | `Cyclic reference involving immutable container ...` |
| Tampered data or wrong key | `HMACError` | `HMAC verification failed: ...` |
| Signed payload, no key | `HMACError` | `payload is signed; pass hmac_key` |
| Key given, payload unsigned | `HMACError` | `payload is not signed` |
| No migration path | `SchemaVersionError` | `No migration path from version 1 to 2 for type '...'` |
| Empty `hmac_key` | `ValueError` | `hmac_key must be a non-empty bytes-like object` |
| `protocol != 5` | `ValueError` | `Unsupported protocol N; only protocol 5 is supported` |
| Non-bytes input to `loads` | `TypeError` | `a bytes-like object is required, not '<type>'` |
| Data does not fit a class in `classes=` | `TypeError` | e.g. `Data for 'User' has field(s) 'x' that User does not define; ...` |
| End of stream (`load`, `Unpickler.load`, `load_pickle`) | `EOFError` | `Ran out of input` |

## Other exceptions

- **`RecursionError`**: nesting deeper than `sys.getrecursionlimit()` in `dumps` or `loads`.
- **`pickle.UnpicklingError`**, `ValueError`, `TypeError`, `EOFError`: malformed or over-limit input to the [pickle reader](./pickle-compat.md). These are not `PySafePickleError` subclasses; only a disallowed global or operation raises `UnsafeTypeError` there.
