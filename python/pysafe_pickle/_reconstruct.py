"""Internal helpers for reconstructing dataclass instances."""
from typing import Any

_class_cache: dict[tuple[str, tuple[str, ...]], type] = {}


def make_class(name: str, field_names: list[str]) -> type:
    """Dynamically create a simple dataclass-like class with the given field names."""
    key = (name, tuple(field_names))
    if key in _class_cache:
        return _class_cache[key]

    anns: dict[str, type] = {fn: object for fn in field_names}

    def __init__(self: Any, **kwargs: Any) -> None:
        for fn in field_names:
            object.__setattr__(self, fn, kwargs.get(fn))

    def __repr__(self: Any) -> str:
        parts = ", ".join(f"{fn}={getattr(self, fn)!r}" for fn in field_names)
        return f"{name}({parts})"

    def __eq__(self: Any, other: object) -> bool:
        if type(self) is not type(other):
            return NotImplemented  # type: ignore[return-value]
        return all(getattr(self, fn) == getattr(other, fn) for fn in field_names)

    def __hash__(self: Any) -> int:
        return hash(tuple(getattr(self, fn) for fn in field_names))

    cls = type(name, (), {"__annotations__": anns})
    cls.__init__ = __init__  # type: ignore[assignment]
    cls.__repr__ = __repr__  # type: ignore[assignment]
    cls.__eq__ = __eq__  # type: ignore[assignment]
    cls.__hash__ = __hash__  # type: ignore[assignment]
    cls.__dataclass_fields__ = {fn: None for fn in field_names}  # type: ignore[attr-defined]

    _class_cache[key] = cls
    return cls
