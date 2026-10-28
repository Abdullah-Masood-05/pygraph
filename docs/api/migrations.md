# Schema Migrations API Reference

Functions and decorators for managing schema versions and migrating serialized dataclasses. For a walkthrough, see the [Schema Evolution guide](../guide/schema-evolution.md).

---

## How migration is triggered

Each dataclass type in a payload carries the schema version its class had at `dumps` time: the class attribute `__pysafe_pickle_version__` (or legacy `__pygraph_version__`), `0` if absent.

On `loads`/`load`/`Unpickler.load`, a dataclass record is migrated when:
1. its class is passed in `classes=` (the "current" version is read from that class), and
2. the stored version and the current version are both greater than `0`, and
3. they differ.

Then the stored fields are collected into a `dict`, `apply_migrations(type_name, state, stored_version, current_version)` runs the chain registered for the **stored** type name, and the resulting `dict` is applied to a new instance of your class (created without calling `__init__`): fields absent from the result get their defaults, a missing field without a default raises `TypeError`, and a key that is not a field of the class raises `TypeError`. The version keys that `apply_migrations` adds are ignored. A migration must return a `dict` (`TypeError: Migration function must return a dict` otherwise), and a missing chain raises `SchemaVersionError`.

If the stored version is **higher** than the class's version (data from a newer release read by older code), no migration function runs; the stored fields are applied as they are, with the same missing/unknown field checks.

Without `classes=`, dataclasses load as generated stand-in classes that carry no version, so no migration runs and the object has exactly the stored fields. You can still migrate by hand with `apply_migrations` (see the guide).

---

## `@migrate(*, from_version: int, to_version: int, type_name: str | None = None)`

Decorator to register a migration function for a dataclass between two schema versions. The function receives the old state as a `dict` and must return a `dict` with the new state. The decorated function is returned unchanged.

### Parameters:
- **`from_version`** (*int*): The source schema version.
- **`to_version`** (*int*): The target schema version.
- **`type_name`** (*str*, optional): The bare class name the data was stored under (e.g. `"UserProfile"`). If omitted it is inferred as everything before the last `.` of `func.__qualname__` (e.g. the class name for a method defined in a class body). For a module-level function this yields the function's own name, so pass `type_name` explicitly there.

### Example:
```python
import pysafe_pickle as psp

@psp.migrate(from_version=1, to_version=2, type_name="UserProfile")
def upgrade_v1_to_v2(state: dict) -> dict:
    state["display_name"] = state.get("username", "")
    return state

for from_v, to_v, func in psp.get_migrations("UserProfile"):
    print(from_v, to_v, func.__name__)  # 1 2 upgrade_v1_to_v2
```

---

## `get_migrations(type_name: str) -> list[tuple[int, int, Callable]]`

Retrieves all registered `(from_version, to_version, func)` tuples for a specific type name. Returns an empty list if none are registered.

---

## `get_migration_chain(type_name: str, from_version: int, to_version: int) -> list[tuple[int, int, Callable[[dict], dict]]]`

Resolves the ordered chain of migration functions needed to transform a type from `from_version` to `to_version`. At each step the registered migration starting at the current version with the highest `to_version` not exceeding the target is chosen. Returns `[]` if `from_version >= to_version`.

### Raises:
- **`SchemaVersionError`** (a `ValueError`): If no valid path exists between the two versions (`No migration path from version 1 to 2 for type 'X'`).

---

## `apply_migrations(type_name: str, state: dict, from_version: int, to_version: int) -> dict`

Executes the resolved migration chain against a copy of the provided `state` dictionary. After each step it sets `__pysafe_pickle_version__` and `__pygraph_version__` keys in the state to that step's target version. If `from_version >= to_version`, `state` is returned unchanged.

```python
import pysafe_pickle as psp

@psp.migrate(from_version=1, to_version=2, type_name="UserProfile")
def upgrade_v1_to_v2(state: dict) -> dict:
    state["display_name"] = state.get("username", "")
    return state

print(psp.apply_migrations("UserProfile", {"username": "x"}, 1, 2))
# {'username': 'x', 'display_name': 'x', '__pysafe_pickle_version__': 2, '__pygraph_version__': 2}
```

### Raises:
- **`SchemaVersionError`**: No migration path.

---

## `get_schema_version(obj: Any) -> int | None`

Looks for `__pysafe_pickle_version__`, then legacy `__pygraph_version__`, on the object (which includes attributes inherited from its class). Returns `None` if the object is not versioned. Objects loaded with `classes=` report your class's version; generated stand-in objects (no `classes=`) return `None`.

---

## `set_schema_version(obj: Any, version: int) -> None`

Sets both `__pysafe_pickle_version__` and `__pygraph_version__` on the object's class (or on `obj` itself if it is a class).
