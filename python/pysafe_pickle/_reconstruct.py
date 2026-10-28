"""Internal helpers for reconstructing dataclass, namedtuple and enum instances.

The Rust decoder creates instances with ``object.__new__(cls)`` and sets the
fields directly (pickle semantics: ``__init__`` is not called), so the
instance exists before its fields are decoded and self-referencing graphs work.

Classes passed via ``loads(..., classes=...)`` are trusted by the caller; this
module never imports anything named by the data.
"""
import collections
import dataclasses
import enum
import keyword
import reprlib
from collections.abc import Mapping
from typing import Any

_class_cache: dict[tuple[str, tuple[str, ...]], type] = {}


def _make_field(fn: str) -> Any:
    f = dataclasses.field()
    f.name = fn
    f.type = object
    f._field_type = dataclasses._FIELD  # type: ignore[attr-defined]
    return f


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
    # Real Field objects (not None) so dataclasses.fields() works and a loaded
    # instance can be passed straight back to dumps().
    cls.__dataclass_fields__ = {fn: _make_field(fn) for fn in fields}  # type: ignore[attr-defined]

    _class_cache[key] = cls
    return cls


# ---------------------------------------------------------------------------
# namedtuples without classes=: dynamic stand-in classes
# ---------------------------------------------------------------------------

_nt_cache: dict[tuple[str, tuple[str, ...]], type] = {}
_NT_CACHE_MAX = 4096


def _valid_identifier(name: object) -> bool:
    return isinstance(name, str) and name.isidentifier() and not keyword.iskeyword(name)


def make_namedtuple(name: str, field_names: list[str]) -> type:
    """Build (and cache) a ``collections.namedtuple`` stand-in class.

    The names come from untrusted data, so they are validated here before
    ``namedtuple`` sees them (it generates code from the field names).
    """
    key = (name, tuple(field_names))
    cls = _nt_cache.get(key)
    if cls is not None:
        return cls
    if (
        not _valid_identifier(name)
        or not all(_valid_identifier(f) and not f.startswith("_") for f in field_names)
        or len(set(field_names)) != len(field_names)
    ):
        raise ValueError(f"Invalid namedtuple type in data: {name!r} {field_names!r}")
    cls = collections.namedtuple(name, field_names)  # type: ignore[misc]
    if len(_nt_cache) >= _NT_CACHE_MAX:
        _nt_cache.clear()
    _nt_cache[key] = cls
    return cls


# ---------------------------------------------------------------------------
# classes= support (caller-trusted classes)
# ---------------------------------------------------------------------------

_VERSION_KEYS = frozenset({"__pysafe_pickle_version__", "__pygraph_version__"})
_CLASSES_USAGE = "classes= must be an iterable of classes or a mapping {name: class}"


def class_map(classes: Any) -> dict[str, type]:
    """Normalize ``classes=`` (iterable of classes or mapping name -> class)."""
    if isinstance(classes, Mapping):
        items = list(classes.items())
    elif isinstance(classes, (str, bytes, type)):
        raise TypeError(_CLASSES_USAGE)
    else:
        try:
            items = [(getattr(c, "__name__", None), c) for c in classes]
        except TypeError:
            raise TypeError(_CLASSES_USAGE) from None
    out: dict[str, type] = {}
    for name, cls in items:
        if not isinstance(cls, type):
            raise TypeError(f"classes= entries must be classes, got {cls!r}")
        if not isinstance(name, str):
            raise TypeError(f"classes= names must be str, got {name!r}")
        prev = out.get(name)
        if prev is not None and prev is not cls:
            raise ValueError(
                f"classes= contains two different classes named {name!r}; "
                "pass a dict {name: class} to disambiguate"
            )
        out[name] = cls
    return out


def _dc_fields(cls: type, type_name: str) -> dict[str, Any]:
    if not dataclasses.is_dataclass(cls):
        raise TypeError(
            f"classes[{type_name!r}] must be a dataclass to load dataclass data, got {cls!r}"
        )
    return {f.name: f for f in dataclasses.fields(cls)}


def _has_default(f: Any) -> bool:
    return f.default is not dataclasses.MISSING or f.default_factory is not dataclasses.MISSING


def _default(f: Any) -> Any:
    if f.default is not dataclasses.MISSING:
        return f.default
    return f.default_factory()


def _extra_error(cls: type, type_name: str, extra: list[str]) -> TypeError:
    return TypeError(
        f"Data for {type_name!r} has field(s) {', '.join(map(repr, extra))} that "
        f"{cls.__qualname__} does not define; add them or register a @migrate migration"
    )


def _missing_error(cls: type, type_name: str, name: str) -> TypeError:
    return TypeError(
        f"Data for {type_name!r} has no value for field {name!r} of "
        f"{cls.__qualname__}, which has no default"
    )


def dataclass_plan(cls: type, type_name: str, serialized: list[str], migrating: bool) -> list[str]:
    """Check serialized fields against ``cls``; return the fields to default.

    Without a migration, unknown serialized fields and missing fields without
    a default are errors. With a migration the checks run on the migrated
    state instead (see :func:`apply_state`).
    """
    fields = _dc_fields(cls, type_name)
    if migrating:
        return []
    extra = [n for n in serialized if n not in fields]
    if extra:
        raise _extra_error(cls, type_name, extra)
    present = set(serialized)
    missing = [n for n in fields if n not in present]
    for n in missing:
        if not _has_default(fields[n]):
            raise _missing_error(cls, type_name, n)
    return missing


def fill_defaults(obj: Any, cls: type, missing: list[str]) -> None:
    fields = cls.__dataclass_fields__  # type: ignore[attr-defined]
    for n in missing:
        object.__setattr__(obj, n, _default(fields[n]))


def apply_state(obj: Any, cls: type, type_name: str, state: dict) -> None:
    """Set a migrated ``state`` on a real dataclass instance (pickle semantics)."""
    fields = _dc_fields(cls, type_name)
    extra = [k for k in state if k not in fields and k not in _VERSION_KEYS]
    if extra:
        raise _extra_error(cls, type_name, extra)
    for n, f in fields.items():
        if n in state:
            v = state[n]
        elif _has_default(f):
            v = _default(f)
        else:
            raise _missing_error(cls, type_name, n)
        object.__setattr__(obj, n, v)


def namedtuple_plan(cls: type, type_name: str, serialized: list[str]) -> bool:
    """Validate a namedtuple class; True if values map 1:1 onto ``cls._fields``."""
    if not (
        issubclass(cls, tuple)
        and hasattr(cls, "_make")
        and isinstance(getattr(cls, "_fields", None), tuple)
    ):
        raise TypeError(
            f"classes[{type_name!r}] must be a namedtuple class to load namedtuple data, got {cls!r}"
        )
    fields = cls._fields  # type: ignore[attr-defined]
    if tuple(serialized) == fields:
        return True
    extra = [n for n in serialized if n not in fields]
    if extra:
        raise _extra_error(cls, type_name, extra)
    return False


def build_namedtuple(cls: Any, type_name: str, serialized: list[str], values: tuple) -> Any:
    """Map serialized values onto ``cls._fields`` by name (defaults for absent ones)."""
    state = dict(zip(serialized, values))
    defaults = cls._field_defaults
    out = []
    for n in cls._fields:
        if n in state:
            out.append(state[n])
        elif n in defaults:
            out.append(defaults[n])
        else:
            raise _missing_error(cls, type_name, n)
    return cls._make(out)


def check_enum_class(cls: type, type_name: str) -> None:
    if not isinstance(cls, enum.EnumMeta):
        raise TypeError(
            f"classes[{type_name!r}] must be an Enum class to load enum data, got {cls!r}"
        )


def enum_member(cls: Any, type_name: str, by_value: bool, key: Any) -> Any:
    if by_value:
        return cls(key)
    if not isinstance(key, str):
        raise ValueError(f"Invalid data: enum member name for {type_name!r} is not a str")
    try:
        return cls[key]
    except KeyError:
        raise ValueError(f"{cls.__qualname__} has no member named {key!r}") from None
