"""Schema evolution decorators and migration registry for pygraph."""
from typing import Callable


_migration_registry: dict[str, list[tuple[int, int, Callable]]] = {}


def migrate(*, from_version: int, to_version: int, type_name: str | None = None):
    """Decorator to register a migration function between schema versions.

    The decorated function receives the old state as a dict and must return
    a dict with the new state.
    """
    def decorator(func: Callable) -> Callable:
        if type_name is not None:
            cls_name = type_name
        else:
            cls_name = func.__qualname__.rsplit(".", 1)[0]
        if cls_name not in _migration_registry:
            _migration_registry[cls_name] = []
        _migration_registry[cls_name].append((from_version, to_version, func))
        func._pygraph_migration = True
        func._pygraph_from_version = from_version
        func._pygraph_to_version = to_version
        return func
    return decorator


def get_migrations(type_name: str) -> list[tuple[int, int, Callable]]:
    """Get all registered migrations for a type."""
    return _migration_registry.get(type_name, [])


def get_migration_chain(type_name: str, from_version: int, to_version: int) -> list[tuple[int, int, Callable[[dict], dict]]]:
    """Get the ordered migration chain from from_version to to_version."""
    all_migrations = _migration_registry.get(type_name, [])
    if from_version >= to_version:
        return []

    chain = []
    current = from_version

    while current < to_version:
        best = None
        for entry in all_migrations:
            f, t, func = entry
            if f == current and t <= to_version:
                if best is None or t > best[1]:
                    best = entry

        if best is None:
            raise ValueError(
                f"No migration path from version {current} to {to_version} "
                f"for type '{type_name}'"
            )

        chain.append(best)
        current = best[1]

    return chain


def apply_migrations(type_name: str, state: dict, from_version: int, to_version: int) -> dict:
    """Apply the migration chain to transform state from one version to another."""
    if from_version >= to_version:
        return state

    chain = get_migration_chain(type_name, from_version, to_version)
    current_state = state.copy()

    for from_v, to_v, func in chain:
        current_state = func(current_state)
        current_state["__pygraph_version__"] = to_v

    return current_state


def get_schema_version(obj) -> int | None:
    """Get the schema version of an object, or None if not versioned."""
    if hasattr(obj, "__pygraph_version__"):
        return getattr(obj, "__pygraph_version__")
    if hasattr(obj, "__dataclass_fields__"):
        cls = type(obj)
        if hasattr(cls, "__pygraph_version__"):
            return getattr(cls, "__pygraph_version__")
    return None


def set_schema_version(obj, version: int) -> None:
    """Set the schema version on an object's class."""
    cls = type(obj) if not isinstance(obj, type) else obj
    cls.__pygraph_version__ = version
