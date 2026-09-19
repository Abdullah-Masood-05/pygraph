# Schema Migrations API Reference

Functions and decorators for managing schema versions and migrating serialized dataclasses.

---

## `@migrate(*, from_version: int, to_version: int, type_name: str | None = None)`

Decorator to register a migration function for a dataclass between two schema versions.

### Parameters:
- **`from_version`** (*int*): The source schema version.
- **`to_version`** (*int*): The target schema version.
- **`type_name`** (*str*, optional): The name of the dataclass type. Inferred from `func.__qualname__` if omitted.

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

Retrieves all registered migration tuples for a specific type name.

---

## `get_migration_chain(type_name: str, from_version: int, to_version: int) -> list[tuple[int, int, Callable]]`

Resolves the ordered chain of migration functions needed to transform a type from `from_version` to `to_version`.

### Raises:
- **`ValueError`**: If no valid path exists between the two versions.

---

## `apply_migrations(type_name: str, state: dict, from_version: int, to_version: int) -> dict`

Executes the resolved migration chain against the provided `state` dictionary and updates the schema version attributes.

---

## `get_schema_version(obj: Any) -> int | None`

Inspects an object or class for `__pysafe_pickle_version__` or legacy `__pygraph_version__`.

---

## `set_schema_version(obj: Any, version: int) -> None`

Explicitly sets both `__pysafe_pickle_version__` and `__pygraph_version__` on an object or class.
