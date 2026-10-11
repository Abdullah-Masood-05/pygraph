# Schema Migrations API Reference

Functions and decorators for managing schema versions and migrating serialized dataclasses.

---

## How migration is triggered

On `loads`, a dataclass record is migrated when its serialized schema version and the version of the class it is loaded into are both greater than `0` and differ. The serialized version comes from the original class attribute `__pysafe_pickle_version__` (or legacy `__pygraph_version__`) at `dumps` time. The "current" version is read from the dynamic class created for the loaded type (see `make_class` in [Core Functions](./core.md)), not from your original class, so it must be set on that class (e.g. with `set_schema_version`) before the first `loads` of that type in the process; the result is cached afterwards.

Migrated fields are set on the loaded instance. Keys returned by the migration that are not fields of the loaded class are stored as a dict in the `__pysafe_pickle_extra__` attribute (also `__pygraph_extra__`). A migration must return a `dict`, otherwise `TypeError: Migration function must return a dict` is raised.

---

## `@migrate(*, from_version: int, to_version: int, type_name: str | None = None)`

Decorator to register a migration function for a dataclass between two schema versions. The function receives the old state as a `dict` and must return a `dict` with the new state. The decorated function is returned unchanged.

### Parameters:
- **`from_version`** (*int*): The source schema version.
- **`to_version`** (*int*): The target schema version.
- **`type_name`** (*str*, optional): The bare class name of the dataclass (e.g. `"UserProfile"`). If omitted it is inferred as everything before the last `.` of `func.__qualname__` (e.g. the class name for a method defined in a class body). For a module-level function this yields the function's own name, so pass `type_name` explicitly there.

### Example:
```python
import pysafe_pickle as psp

@psp.migrate(from_version=1, to_version=2, type_name="UserProfile")
def upgrade_v1_to_v2(state: dict) -> dict:
    state["display_name"] = state.get("username", "")
    return state
```

---

## `get_migrations(type_name: str) -> list[tuple[int, int, Callable]]`

Retrieves all registered `(from_version, to_version, func)` tuples for a specific type name. Returns an empty list if none are registered.

---

## `get_migration_chain(type_name: str, from_version: int, to_version: int) -> list[tuple[int, int, Callable[[dict], dict]]]`

Resolves the ordered chain of migration functions needed to transform a type from `from_version` to `to_version`. At each step the registered migration starting at the current version with the highest `to_version` not exceeding the target is chosen. Returns `[]` if `from_version >= to_version`.

### Raises:
- **`ValueError`**: If no valid path exists between the two versions (`No migration path from version 1 to 2 for type 'X'`).

---

## `apply_migrations(type_name: str, state: dict, from_version: int, to_version: int) -> dict`

Executes the resolved migration chain against a copy of the provided `state` dictionary. After each step it sets `__pysafe_pickle_version__` and `__pygraph_version__` keys in the state to that step's target version. If `from_version >= to_version`, `state` is returned unchanged.

```python
psp.apply_migrations("UserProfile", {"username": "x"}, 1, 2)
# {'username': 'x', 'display_name': 'x', '__pysafe_pickle_version__': 2, '__pygraph_version__': 2}
```

---

## `get_schema_version(obj: Any) -> int | None`

Looks for `__pysafe_pickle_version__`, then legacy `__pygraph_version__`, on the object (which includes attributes inherited from its class). Returns `None` if the object is not versioned.

---

## `set_schema_version(obj: Any, version: int) -> None`

Sets both `__pysafe_pickle_version__` and `__pygraph_version__` on the object's class (or on `obj` itself if it is a class).
