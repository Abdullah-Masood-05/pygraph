"""Internal helpers for reconstructing dataclass instances."""

_class_cache = {}


def make_class(name: str, field_names: list):
    """Dynamically create a simple dataclass-like class with the given field names."""
    key = (name, tuple(field_names))
    if key in _class_cache:
        return _class_cache[key]

    anns = {fn: object for fn in field_names}

    def __init__(self, **kwargs):
        for fn in field_names:
            object.__setattr__(self, fn, kwargs.get(fn))

    def __repr__(self):
        parts = ", ".join(f"{fn}={getattr(self, fn)!r}" for fn in field_names)
        return f"{name}({parts})"

    def __eq__(self, other):
        if type(self) is not type(other):
            return NotImplemented
        return all(getattr(self, fn) == getattr(other, fn) for fn in field_names)

    def __hash__(self):
        return hash(tuple(getattr(self, fn) for fn in field_names))

    cls = type(name, (), {"__annotations__": anns})
    cls.__init__ = __init__
    cls.__repr__ = __repr__
    cls.__eq__ = __eq__
    cls.__hash__ = __hash__
    cls.__dataclass_fields__ = {fn: None for fn in field_names}

    _class_cache[key] = cls
    return cls
