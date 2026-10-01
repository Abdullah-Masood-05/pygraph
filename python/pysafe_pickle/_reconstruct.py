"""Internal helpers for reconstructing dataclass instances.

The Rust decoder creates instances with ``object.__new__(cls)`` and sets the
fields directly (pickle semantics: ``__init__`` is not called), so the
instance exists before its fields are decoded and self-referencing graphs work.
"""
import reprlib
from typing import Any

_class_cache: dict[tuple[str, tuple[str, ...]], type] = {}


def make_class(name: str, field_names: list[str]) -> type:
    """Dynamically create a simple dataclass-like class with the given field names."""
    key = (name, tuple(field_names))
    if key in _class_cache:
        return _class_cache[key]

    fields = tuple(field_names)
    anns: dict[str, type] = {fn: object for fn in fields}

    # `self` is taken positionally so that a field literally named "self"
    # can still be passed as a keyword argument.
    def __init__(*args: Any, **kwargs: Any) -> None:
        if len(args) != 1:
            raise TypeError(f"{name}() takes only keyword arguments")
        self = args[0]
        for fn in fields:
            object.__setattr__(self, fn, kwargs.get(fn))

    @reprlib.recursive_repr()
    def __repr__(self: Any) -> str:
        parts = ", ".join(f"{fn}={getattr(self, fn)!r}" for fn in fields)
        return f"{name}({parts})"

    def _astuple(self: Any) -> tuple:
        return tuple(getattr(self, fn) for fn in fields)

    # Compare/hash field tuples (like dataclasses) so that identical members
    # short-circuit and self-referencing instances do not recurse forever.
    def __eq__(self: Any, other: object) -> bool:
        if self is other:
            return True
        if type(self) is not type(other):
            return NotImplemented  # type: ignore[return-value]
        return _astuple(self) == _astuple(other)

    def __hash__(self: Any) -> int:
        return hash(_astuple(self))

    cls = type(name, (), {"__annotations__": anns})
    cls.__init__ = __init__  # type: ignore[assignment]
    cls.__repr__ = __repr__  # type: ignore[assignment]
    cls.__eq__ = __eq__  # type: ignore[assignment]
    cls.__hash__ = __hash__  # type: ignore[assignment]
    cls.__dataclass_fields__ = {fn: None for fn in fields}  # type: ignore[attr-defined]

    _class_cache[key] = cls
    return cls
