"""Safe reader for standard ``pickle`` streams (protocols 0-5).

This module lets pysafe-pickle read existing ``.pkl`` files produced by the
standard library ``pickle`` module **without ever executing code chosen by
the data**.

Design: a self-contained opcode VM (not a ``pickle._Unpickler`` subclass)
-------------------------------------------------------------------------
``pickle._Unpickler`` is a poor base for a security boundary:

* Its dispatch table is a private, version-dependent implementation detail.
  Opcodes added in a future Python release would silently be inherited and
  run with stock (unsafe) semantics.
* Overriding ``find_class`` alone is not enough: ``REDUCE``, ``NEWOBJ``,
  ``BUILD`` (``__setstate__``), ``INST``/``OBJ`` and ``EXT*`` all call into
  arbitrary objects, ``APPEND``/``SETITEM`` call ``append``/``__setitem__`` on
  whatever is on the stack, and none of them bound resource usage. Making
  every one of them safe means overriding nearly the whole dispatch table
  anyway.

So :class:`_SafeVM` is a small interpreter written from scratch against the
documented opcode set (see ``pickletools``). Properties that make it safe:

1. **Closed opcode set.** Every opcode of protocols 0-5 is handled
   explicitly. Unknown bytes raise :class:`pickle.UnpicklingError`.
2. **No imports driven by data.** ``GLOBAL``/``STACK_GLOBAL``/``INST`` names
   are looked up in a static table of pre-imported, known-safe objects. A
   module name in the stream is never passed to ``import``. (Only pairs given
   explicitly via ``allow=`` are imported, because the caller asked for it.)
3. **No generic calls.** ``REDUCE``/``NEWOBJ``/``NEWOBJ_EX`` find a
   per-callable *handler* by object identity. Each handler checks the
   argument types and shapes first and only then calls the constructor. Any
   callable without a handler raises :class:`UnsafeTypeError` before
   anything is called.
4. **No user hooks.** Classes passed via ``classes=`` are rebuilt with
   ``cls.__new__`` (which pickle's ``NEWOBJ`` protocol requires) or with
   ``object.__new__``/``base.__new__`` for ``copyreg._reconstructor``. State
   is written straight into ``__dict__`` and declared slots with
   ``object.__setattr__``. User ``__reduce__``, ``__setstate__``,
   ``__init__``, ``__setattr__``, ``__setitem__``, ``append`` and ``extend``
   are never called. ``REDUCE`` on a user class works only for ``Enum``
   (member lookup by value) and ``BaseException`` subclasses (``cls.__new__``
   plus ``args``). Other classes would need their ``__init__`` run, so they
   are refused unless the class is also listed in ``allow=``.
5. **Restricted mutation.** ``APPEND(S)``, ``SETITEM(S)`` and ``ADDITEMS``
   only accept ``list``/``deque``, ``dict``-family and ``set`` targets, and
   they call the *builtin* base method, not a subclass override. ``BUILD``
   only accepts instances of allowed classes and ``uuid.UUID``.
6. **Bounded resources.** Length-prefixed opcodes are checked against the
   remaining input before any slicing. Copying constructors (``list(x)``,
   ``bytearray(b)`` and so on) draw from an item budget proportional to the
   input size, so repeated memo references can't blow up memory
   quadratically. Integer-argument forms that allocate (``bytes(10**12)``)
   and ``Fraction`` strings with exponents (``'1e999999999'``) are rejected.
   Nesting depth is capped both while parsing (MARK depth) and on the final
   object graph (longest container path).

Known-safe globals (the default allowlist)
------------------------------------------
See :data:`SAFE_GLOBALS` for the exact list. In short: ``datetime`` types,
``decimal.Decimal``, ``fractions.Fraction``, ``uuid.UUID``,
``collections.OrderedDict/deque/Counter/defaultdict`` (``defaultdict`` only
with builtin factories), the safe builtins (``set``, ``frozenset``,
``bytearray``, ``complex``, ``slice``, ``range``, ``int``, ``float``,
``str``, ``bytes``, ``list``, ``dict``, ``tuple``, ``bool``, ``object`` as a
value only), ``copyreg._reconstructor`` (only for allowed classes and safe
builtin bases), ``codecs.encode``/``_codecs.encode`` (only
``(str, 'latin1')``), ``pathlib`` path classes, and
``zoneinfo.ZoneInfo._unpickle`` with a validated key. Pickle stores that
helper as ``builtins.getattr(ZoneInfo, '_unpickle')``, so ``builtins.getattr``
resolves to an inert marker. Its ``REDUCE`` accepts only that exact pair, or
``(AllowedEnum, member_name)``, and never calls the real ``getattr``.
Python 2 names (``__builtin__``, ``copy_reg``, ...) are mapped the same way
the stdlib does for protocols < 3.
"""

from __future__ import annotations

import _compat_pickle
import codecs
import collections
import copyreg
import dataclasses
import datetime
import decimal
import enum
import fractions
import importlib
import io
import os
import pathlib
import re
import struct
import sys
import uuid
from pickle import UnpicklingError
from typing import Any, Callable, Iterable

from pysafe_pickle.exceptions import UnsafeTypeError

try:  # pragma: no cover - zoneinfo is stdlib on 3.9+, but be defensive
    import zoneinfo as _zoneinfo
except ImportError:  # pragma: no cover
    _zoneinfo = None

__all__ = [
    "load_pickle",
    "loads_pickle",
    "convert_pickle",
    "SAFE_GLOBALS",
    "DEFAULT_MAX_DEPTH",
]

#: Maximum container nesting depth accepted by default.
DEFAULT_MAX_DEPTH = 1000

_HIGHEST_PROTOCOL = 5

# Budget for copying constructors: items copied may not exceed
# ``_BUDGET_FACTOR * len(data) + _BUDGET_BASE``.
_BUDGET_FACTOR = 8
_BUDGET_BASE = 1_000_000


class _Handler:
    """Validator + constructor for one allowlisted callable."""

    __slots__ = ("obj", "name", "reduce", "newobj")

    def __init__(self, obj, name, reduce=None, newobj=None):
        self.obj = obj
        self.name = name
        self.reduce = reduce  # f(vm, args) -> value
        self.newobj = newobj  # f(vm, args) -> value


class _GetattrMarker:
    """Inert stand-in for ``builtins.getattr``; never calls the real one."""

    __slots__ = ()

    def __repr__(self):
        return "<pysafe_pickle.compat restricted getattr>"


_GETATTR = _GetattrMarker()


def _qualname(obj: Any) -> str:
    mod = getattr(obj, "__module__", None)
    qn = getattr(obj, "__qualname__", None)
    if isinstance(mod, str) and isinstance(qn, str):
        return f"{mod}.{qn}"
    return f"object of type {type(obj).__module__}.{type(obj).__qualname__}"


def _bad_args(name: str, args: tuple) -> UnpicklingError:
    types = ", ".join(type(a).__name__ for a in args)
    return UnpicklingError(f"invalid arguments for {name}: ({types})")


# --------------------------------------------------------------------------
# Argument validators / constructors for the stdlib allowlist
# --------------------------------------------------------------------------

_SEQ_TYPES = (list, tuple, set, frozenset)
_BYTES_ENCODINGS = frozenset({"latin1", "latin-1", "iso-8859-1", "utf-8", "utf8", "ascii"})


def _arity(name, args, lo, hi):
    if not (lo <= len(args) <= hi):
        raise _bad_args(name, args)


def _mk_container(ctor, name, accepted=_SEQ_TYPES):
    def handler(vm, args):
        _arity(name, args, 0, 1)
        if not args:
            return ctor()
        src = args[0]
        if not isinstance(src, accepted):
            raise _bad_args(name, args)
        vm.charge(len(src))
        return ctor(src)

    return handler


def _h_dict(vm, args):
    _arity("builtins.dict", args, 0, 1)
    if not args:
        return {}
    src = args[0]
    if isinstance(src, dict):
        vm.charge(len(src))
        return dict(dict.items(src))
    if isinstance(src, (list, tuple)):
        vm.charge(len(src))
        out = {}
        for pair in src:
            if not isinstance(pair, (list, tuple)) or len(pair) != 2:
                raise _bad_args("builtins.dict", args)
            out[pair[0]] = pair[1]
        return out
    raise _bad_args("builtins.dict", args)


def _mk_bytes(ctor, name):
    def handler(vm, args):
        _arity(name, args, 0, 3)
        if not args:
            return ctor()
        src = args[0]
        if len(args) == 1 and isinstance(src, (bytes, bytearray)):
            vm.charge(len(src))
            return ctor(src)
        if isinstance(src, str) and len(args) >= 2:
            enc = args[1]
            err = args[2] if len(args) == 3 else "strict"
            if (
                not isinstance(enc, str)
                or enc.lower() not in _BYTES_ENCODINGS
                or err not in ("strict",)
            ):
                raise _bad_args(name, args)
            vm.charge(len(src))
            return ctor(src, enc, err)
        # Integer sizes (bytes(10**12)) and arbitrary iterables are refused.
        raise _bad_args(name, args)

    return handler


def _h_codecs_encode(vm, args):
    _arity("codecs.encode", args, 2, 3)
    text, enc = args[0], args[1]
    err = args[2] if len(args) == 3 else "strict"
    if (
        not isinstance(text, str)
        or not isinstance(enc, str)
        or enc.lower() not in ("latin1", "latin-1")
        or err != "strict"
    ):
        raise UnsafeTypeError(
            "codecs.encode is only allowed as codecs.encode(<str>, 'latin1')"
        )
    vm.charge(len(text))
    return codecs.encode(text, "latin1")


def _h_int(vm, args):
    _arity("builtins.int", args, 0, 2)
    if not args:
        return 0
    if len(args) == 1:
        if type(args[0]) in (int, bool, float, str, bytes):
            return int(args[0])
    elif type(args[0]) in (str, bytes) and type(args[1]) is int:
        return int(args[0], args[1])
    raise _bad_args("builtins.int", args)


def _h_float(vm, args):
    _arity("builtins.float", args, 0, 1)
    if not args:
        return 0.0
    if type(args[0]) in (int, bool, float, str):
        return float(args[0])
    raise _bad_args("builtins.float", args)


def _h_bool(vm, args):
    _arity("builtins.bool", args, 0, 1)
    if not args:
        return False
    if type(args[0]) in (int, bool):
        return bool(args[0])
    raise _bad_args("builtins.bool", args)


def _h_str(vm, args):
    _arity("builtins.str", args, 0, 3)
    if not args:
        return ""
    if len(args) == 1 and type(args[0]) is str:
        return args[0]
    if (
        len(args) >= 2
        and type(args[0]) in (bytes, bytearray)
        and type(args[1]) is str
        and args[1].lower() in _BYTES_ENCODINGS  # no data-driven codec lookups
        and (len(args) == 2 or args[2] in ("strict", "surrogatepass", "replace"))
    ):
        vm.charge(len(args[0]))
        return str(*args)
    raise _bad_args("builtins.str", args)


def _h_complex(vm, args):
    _arity("builtins.complex", args, 0, 2)
    if len(args) == 1 and type(args[0]) is str:
        return complex(args[0])
    if all(type(a) in (int, float, bool) for a in args):
        return complex(*args)
    raise _bad_args("builtins.complex", args)


def _h_slice(vm, args):
    _arity("builtins.slice", args, 1, 3)
    return slice(*args)


def _h_range(vm, args):
    _arity("builtins.range", args, 1, 3)
    if all(type(a) in (int, bool) for a in args):
        return range(*args)
    raise _bad_args("builtins.range", args)


def _mk_datetime(cls, name, maxargs):
    ok = (int, bool, bytes, type(None))

    def handler(vm, args):
        _arity(name, args, 0, maxargs)
        for a in args:
            if not (type(a) in ok or isinstance(a, datetime.tzinfo)):
                raise _bad_args(name, args)
        return cls(*args)

    return handler


def _h_timedelta(vm, args):
    _arity("datetime.timedelta", args, 0, 7)
    if all(type(a) in (int, float, bool) for a in args):
        return datetime.timedelta(*args)
    raise _bad_args("datetime.timedelta", args)


def _h_timezone(vm, args):
    _arity("datetime.timezone", args, 1, 2)
    if type(args[0]) is datetime.timedelta and (len(args) == 1 or type(args[1]) is str):
        return datetime.timezone(*args)
    raise _bad_args("datetime.timezone", args)


def _h_decimal(vm, args):
    _arity("decimal.Decimal", args, 0, 1)
    if not args:
        return decimal.Decimal()
    if type(args[0]) in (str, int, float):
        return decimal.Decimal(args[0])
    raise _bad_args("decimal.Decimal", args)


# Plain integers / ratios / decimals only. Exponent forms like '1e999999999'
# would force Fraction to compute 10**exp, a CPU/memory DoS.
_FRACTION_RE = re.compile(r"\s*[+-]?(?:\d+(?:/\d+)?|\d*\.\d+|\d+\.)\s*\Z")


def _h_fraction(vm, args):
    _arity("fractions.Fraction", args, 0, 2)
    if not args:
        return fractions.Fraction()
    if len(args) == 2:
        if type(args[0]) is int and type(args[1]) is int:
            return fractions.Fraction(args[0], args[1])
        raise _bad_args("fractions.Fraction", args)
    a = args[0]
    if type(a) is str:
        if len(a) > 10_000 or not _FRACTION_RE.match(a):
            raise _bad_args("fractions.Fraction", args)
        return fractions.Fraction(a)
    if type(a) in (int, float, decimal.Decimal, fractions.Fraction):
        return fractions.Fraction(a)
    raise _bad_args("fractions.Fraction", args)


def _h_uuid_reduce(vm, args):
    _arity("uuid.UUID", args, 1, 1)
    if type(args[0]) is str and len(args[0]) <= 64:
        return uuid.UUID(args[0])
    raise _bad_args("uuid.UUID", args)


def _h_uuid_newobj(vm, args):
    if args:
        raise _bad_args("uuid.UUID", args)
    return object.__new__(uuid.UUID)


def _h_ordereddict(vm, args):
    _arity("collections.OrderedDict", args, 0, 1)
    if not args:
        return collections.OrderedDict()
    src = args[0]
    if isinstance(src, (list, tuple)):
        vm.charge(len(src))
        od = collections.OrderedDict()
        for pair in src:
            if not isinstance(pair, (list, tuple)) or len(pair) != 2:
                raise _bad_args("collections.OrderedDict", args)
            od[pair[0]] = pair[1]
        return od
    if isinstance(src, dict):
        vm.charge(len(src))
        return collections.OrderedDict(dict.items(src))
    raise _bad_args("collections.OrderedDict", args)


def _h_counter(vm, args):
    _arity("collections.Counter", args, 0, 1)
    if not args:
        return collections.Counter()
    if isinstance(args[0], dict):
        vm.charge(len(args[0]))
        c = collections.Counter()
        for k, v in dict.items(args[0]):
            dict.__setitem__(c, k, v)
        return c
    raise _bad_args("collections.Counter", args)


def _h_deque(vm, args):
    _arity("collections.deque", args, 0, 2)
    src = args[0] if args else ()
    maxlen = args[1] if len(args) == 2 else None
    if not isinstance(src, (list, tuple)):
        raise _bad_args("collections.deque", args)
    if maxlen is not None and (type(maxlen) is not int or maxlen < 0):
        raise _bad_args("collections.deque", args)
    vm.charge(len(src))
    return collections.deque(src, maxlen)


_DEFAULTDICT_FACTORIES = (
    None, list, dict, set, frozenset, tuple, int, float, complex, str,
    bytes, bytearray, bool,
)


def _h_defaultdict(vm, args):
    _arity("collections.defaultdict", args, 0, 2)
    factory = args[0] if args else None
    if not any(factory is f for f in _DEFAULTDICT_FACTORIES):
        raise UnsafeTypeError(
            f"collections.defaultdict factory {_qualname(factory)} is not allowed "
            "(only builtin type factories are)"
        )
    dd = collections.defaultdict(factory)
    if len(args) == 2:
        src = args[1]
        if not isinstance(src, dict):
            raise _bad_args("collections.defaultdict", args)
        vm.charge(len(src))
        for k, v in dict.items(src):
            dict.__setitem__(dd, k, v)
    return dd


def _mk_path(cls, name):
    def handler(vm, args):
        if len(args) > 4096 or not all(type(a) is str for a in args):
            raise _bad_args(name, args)
        vm.charge(len(args))
        return cls(*args)

    return handler


_ZONE_KEY_RE = re.compile(r"[A-Za-z0-9_+\-]+(?:/[A-Za-z0-9_+\-]+)*\Z")


def _h_zoneinfo_unpickle(vm, args):
    _arity("zoneinfo.ZoneInfo._unpickle", args, 2, 2)
    key, from_cache = args
    if (
        type(key) is not str
        or len(key) > 255
        or not _ZONE_KEY_RE.match(key)
        or any(part in ("", ".", "..") for part in key.split("/"))
        or type(from_cache) not in (int, bool)
    ):
        shown = repr(key[:80]) if type(key) is str else type(key).__name__
        raise UnsafeTypeError(f"invalid zoneinfo key {shown}")
    return _zoneinfo.ZoneInfo._unpickle(key, from_cache)


_RECON_BASES = (object, tuple, list, dict, set, frozenset, int, float, str, bytes, complex)
_RECON_STATE = {
    tuple: (list, tuple), frozenset: (list, tuple, set, frozenset),
    list: (list, tuple), set: (list, tuple, set, frozenset), dict: (dict,),
    int: (int,), float: (float,), str: (str,), bytes: (bytes,), complex: (complex,),
}


def _h_reconstructor(vm, args):
    """copyreg._reconstructor(cls, base, state) without running user code."""
    _arity("copyreg._reconstructor", args, 3, 3)
    cls, base, state = args
    if not isinstance(cls, type) or not vm.is_reconstructible(cls):
        raise UnsafeTypeError(
            f"copyreg._reconstructor with class {_qualname(cls)} is not allowed"
        )
    if not any(base is b for b in _RECON_BASES) or not issubclass(cls, base):
        raise UnsafeTypeError(
            f"copyreg._reconstructor with base {_qualname(base)} is not allowed"
        )
    if base is object:
        if state is not None:
            raise _bad_args("copyreg._reconstructor", args)
        return object.__new__(cls)
    if cls is not base and not vm.is_user_class(cls):
        raise UnsafeTypeError(
            f"copyreg._reconstructor with class {_qualname(cls)} is not allowed"
        )
    if not isinstance(state, _RECON_STATE[base]):
        raise _bad_args("copyreg._reconstructor", args)
    if hasattr(state, "__len__"):
        vm.charge(len(state))
    obj = base.__new__(cls, state)
    if base in (list, dict, set):
        base.__init__(obj, state)  # builtin init, not the subclass's
    return obj


def _h_getattr(vm, args):
    """Restricted ``getattr``: only the attribute lookups pickle emits."""
    if len(args) == 2:
        target, attr = args
        if (
            _zoneinfo is not None
            and target is _zoneinfo.ZoneInfo
            and attr == "_unpickle"
        ):
            return _ZONEINFO_UNPICKLE
        if (
            isinstance(target, type)
            and issubclass(target, enum.Enum)
            and vm.is_user_class(target)
            and type(attr) is str
            and attr in target.__members__
        ):
            return target.__members__[attr]
    shown = ", ".join(
        _qualname(a) if isinstance(a, type)
        else repr(a[:60]) if type(a) is str
        else f"<{type(a).__name__}>"
        for a in args
    )
    raise UnsafeTypeError(f"builtins.getattr({shown}) is not allowed")


_ZONEINFO_UNPICKLE = _zoneinfo.ZoneInfo._unpickle if _zoneinfo is not None else None


def _build_safe_table() -> dict:
    """(module, name) -> _Handler for every default-allowed global."""
    t: dict = {}

    def add(module, name, obj, reduce=None, newobj=None):
        t[(module, name)] = _Handler(obj, f"{module}.{name}", reduce, newobj)

    # builtins
    add("builtins", "set", set, _mk_container(set, "builtins.set"))
    add("builtins", "frozenset", frozenset, _mk_container(frozenset, "builtins.frozenset"))
    add("builtins", "list", list, _mk_container(list, "builtins.list"))
    add("builtins", "tuple", tuple, _mk_container(tuple, "builtins.tuple"))
    add("builtins", "dict", dict, _h_dict)
    add("builtins", "bytearray", bytearray, _mk_bytes(bytearray, "builtins.bytearray"))
    add("builtins", "bytes", bytes, _mk_bytes(bytes, "builtins.bytes"))
    add("builtins", "int", int, _h_int)
    add("builtins", "float", float, _h_float)
    add("builtins", "bool", bool, _h_bool)
    add("builtins", "str", str, _h_str)
    add("builtins", "complex", complex, _h_complex)
    add("builtins", "slice", slice, _h_slice)
    add("builtins", "range", range, _h_range)
    add("builtins", "object", object)  # value only (copyreg base); not callable
    add("builtins", "getattr", _GETATTR, _h_getattr)  # inert marker, see docstring
    # codecs (protocol 0-2 encoding of bytes)
    add("_codecs", "encode", codecs.encode, _h_codecs_encode)
    add("codecs", "encode", codecs.encode, _h_codecs_encode)
    # copyreg
    add("copyreg", "_reconstructor", copyreg._reconstructor, _h_reconstructor)
    # datetime
    add("datetime", "datetime", datetime.datetime, _mk_datetime(datetime.datetime, "datetime.datetime", 9))
    add("datetime", "date", datetime.date, _mk_datetime(datetime.date, "datetime.date", 3))
    add("datetime", "time", datetime.time, _mk_datetime(datetime.time, "datetime.time", 6))
    add("datetime", "timedelta", datetime.timedelta, _h_timedelta)
    add("datetime", "timezone", datetime.timezone, _h_timezone)
    # numbers
    add("decimal", "Decimal", decimal.Decimal, _h_decimal)
    add("fractions", "Fraction", fractions.Fraction, _h_fraction)
    # uuid
    add("uuid", "UUID", uuid.UUID, _h_uuid_reduce, _h_uuid_newobj)
    # collections
    add("collections", "OrderedDict", collections.OrderedDict, _h_ordereddict)
    add("collections", "Counter", collections.Counter, _h_counter)
    add("collections", "deque", collections.deque, _h_deque)
    add("collections", "defaultdict", collections.defaultdict, _h_defaultdict)
    # pathlib
    for pname in ("PurePath", "PurePosixPath", "PureWindowsPath", "Path", "PosixPath", "WindowsPath"):
        cls = getattr(pathlib, pname, None)
        if cls is None:
            continue
        for mod in ("pathlib", "pathlib._local"):
            add(mod, pname, cls, _mk_path(cls, f"pathlib.{pname}"))
    # zoneinfo
    if _zoneinfo is not None:
        add("zoneinfo", "ZoneInfo", _zoneinfo.ZoneInfo)  # value only (getattr target)
        for mod in ("zoneinfo", "zoneinfo._zoneinfo", "_zoneinfo"):
            add(mod, "ZoneInfo._unpickle", _ZONEINFO_UNPICKLE, _h_zoneinfo_unpickle)
    return t


_SAFE_TABLE = _build_safe_table()

#: The exact default allowlist as a frozenset of ``(module, name)`` pairs.
SAFE_GLOBALS = frozenset(_SAFE_TABLE)


# --------------------------------------------------------------------------
# State setting for allowed user classes (no __setstate__)
# --------------------------------------------------------------------------

def _slot_names(cls: type) -> frozenset:
    names = set()
    for klass in cls.__mro__:
        slots = klass.__dict__.get("__slots__")
        if slots is None:
            continue
        if isinstance(slots, str):
            slots = (slots,)
        for s in slots:
            if s in ("__dict__", "__weakref__"):
                continue
            if s.startswith("__") and not s.endswith("__"):
                s = f"_{klass.__name__.lstrip('_')}{s}"
            names.add(s)
    return frozenset(names)


def _is_dataclass_list_state(cls: type) -> bool:
    setstate = getattr(cls, "__setstate__", None)
    return (
        dataclasses.is_dataclass(cls)
        and getattr(setstate, "__qualname__", None) == "_dataclass_setstate"
        and getattr(setstate, "__module__", None) == "dataclasses"
    )


def _set_state_direct(obj: Any, state: Any) -> None:
    cls = type(obj)
    if state is None:
        return
    slotstate = None
    if isinstance(state, (list, tuple)) and _is_dataclass_list_state(cls):
        fields = dataclasses.fields(cls)
        if len(state) != len(fields):
            raise UnpicklingError(f"bad dataclass state length for {_qualname(cls)}")
        for f, v in zip(fields, state):
            object.__setattr__(obj, f.name, v)
        return
    if type(state) is tuple and len(state) == 2:
        state, slotstate = state
    if state is not None:
        if type(state) is not dict:
            raise UnpicklingError(f"unsupported BUILD state for {_qualname(cls)}")
        try:
            inst_dict = object.__getattribute__(obj, "__dict__")
        except AttributeError:
            raise UnpicklingError(f"{_qualname(cls)} instances have no __dict__") from None
        for k, v in state.items():
            if type(k) is not str:
                raise UnpicklingError("instance attribute names must be str")
            inst_dict[sys.intern(k)] = v
    if slotstate is not None:
        if type(slotstate) is not dict:
            raise UnpicklingError(f"unsupported slot state for {_qualname(cls)}")
        allowed = _slot_names(cls)
        for k, v in slotstate.items():
            if type(k) is not str or k not in allowed:
                raise UnsafeTypeError(
                    f"BUILD may not set {k!r} on {_qualname(cls)} (not a declared slot)"
                )
            object.__setattr__(obj, k, v)


def _build_uuid(obj: uuid.UUID, state: Any) -> None:
    if type(state) is not dict or "int" not in state or set(state) - {"int", "is_safe"}:
        raise UnpicklingError("invalid uuid.UUID state")
    value = state["int"]
    if type(value) is not int or not 0 <= value < 1 << 128:
        raise UnpicklingError("invalid uuid.UUID state")
    safe = uuid.SafeUUID.unknown
    if "is_safe" in state:
        try:
            safe = uuid.SafeUUID(state["is_safe"])
        except (ValueError, TypeError):
            raise UnpicklingError("invalid uuid.UUID state") from None
    object.__setattr__(obj, "int", value)
    object.__setattr__(obj, "is_safe", safe)


# --------------------------------------------------------------------------
# Depth check on the finished object graph
# --------------------------------------------------------------------------

_DONE = object()


def _children(obj: Any, user_slots: Callable[[type], frozenset]):
    if isinstance(obj, (list, tuple)):
        return iter(list.__iter__(obj) if isinstance(obj, list) else tuple.__iter__(obj))
    if isinstance(obj, dict):
        return (x for kv in dict.items(obj) for x in kv)
    if isinstance(obj, (set, frozenset)):
        return iter(set.__iter__(obj) if isinstance(obj, set) else frozenset.__iter__(obj))
    if isinstance(obj, collections.deque):
        return collections.deque.__iter__(obj)
    if isinstance(obj, slice):
        return iter((obj.start, obj.stop, obj.step))
    slots = user_slots(type(obj))
    if slots is None:
        return None
    vals = []
    try:
        vals.extend(object.__getattribute__(obj, "__dict__").values())
    except AttributeError:
        pass
    for s in slots:
        try:
            vals.append(object.__getattribute__(obj, s))
        except AttributeError:
            pass
    return iter(vals)


def _check_depth(root: Any, limit: int, user_slots) -> None:
    it = _children(root, user_slots)
    if it is None:
        return
    done: dict = {}
    active = {id(root)}
    stack = [[root, it, 0]]
    while stack:
        frame = stack[-1]
        child = next(frame[1], _DONE)
        if child is _DONE:
            stack.pop()
            active.discard(id(frame[0]))
            d = frame[2] + 1
            if d > limit:
                raise UnpicklingError(f"maximum nesting depth {limit} exceeded")
            done[id(frame[0])] = d
            if stack and d > stack[-1][2]:
                stack[-1][2] = d
            continue
        cid = id(child)
        if cid in done:
            if done[cid] > frame[2]:
                frame[2] = done[cid]
            continue
        if cid in active:
            continue
        cit = _children(child, user_slots)
        if cit is None:
            continue
        if len(stack) >= limit:
            raise UnpicklingError(f"maximum nesting depth {limit} exceeded")
        active.add(cid)
        stack.append([child, cit, 0])


# --------------------------------------------------------------------------
# The VM
# --------------------------------------------------------------------------

_U8 = struct.Struct("<B")
_U16 = struct.Struct("<H")
_I32 = struct.Struct("<i")
_U32 = struct.Struct("<I")
_U64 = struct.Struct("<Q")
_F64 = struct.Struct(">d")


class _SafeVM:
    def __init__(
        self,
        data: bytes,
        *,
        classes: Iterable[type] | None,
        allow: Iterable[tuple[str, str]] | None,
        encoding: str,
        errors: str,
        fix_imports: bool,
        max_depth: int,
    ):
        self.data = data
        self.pos = 0
        self.encoding = encoding
        self.errors = errors
        self.fix_imports = fix_imports
        self.max_depth = max_depth
        self.proto = 0
        self.budget = _BUDGET_FACTOR * len(data) + _BUDGET_BASE

        self.user_classes: dict[tuple[str, str], type] = {}
        self.user_class_set: dict[int, type] = {}
        for cls in classes or ():
            if not isinstance(cls, type):
                raise TypeError(f"classes= entries must be classes, got {cls!r}")
            self.user_classes[(cls.__module__, cls.__qualname__)] = cls
            self.user_class_set[id(cls)] = cls

        self.allow_pairs: set[tuple[str, str]] = set()
        for pair in allow or ():
            if (
                not isinstance(pair, tuple)
                or len(pair) != 2
                or not all(isinstance(p, str) for p in pair)
            ):
                raise TypeError(f"allow= entries must be (module, name) tuples, got {pair!r}")
            self.allow_pairs.add(pair)
        self.extra_objs: dict[int, tuple[Any, str]] = {}

        # identity -> handler for every callable we may legitimately call
        self.handlers: dict[int, _Handler] = {id(h.obj): h for h in _SAFE_TABLE.values()}
        self._slot_cache: dict[type, frozenset] = {}
        # id -> (obj, depth) for hashable containers (tuple/frozenset/slice).
        self._imm_depth: dict[int, tuple[Any, int]] = {}

    # ---- policy helpers -------------------------------------------------

    def charge(self, n: int) -> None:
        self.budget -= n
        if self.budget < 0:
            raise UnpicklingError("pickle copies too much data (resource limit exceeded)")

    def _is_listed(self, obj: Any) -> bool:
        return self.user_class_set.get(id(obj)) is obj

    def track(self, obj: Any) -> Any:
        """Record the nesting depth of hashable containers as they are created.

        Tuples, frozensets and slices are immutable, so their depth is final
        at creation. Capping it here (not only on the finished graph) stops
        a deeply nested key from overflowing the C stack in ``hash()`` or
        ``==`` while the stream is still being loaded.
        """
        if isinstance(obj, (tuple, frozenset)):
            children = obj
        elif isinstance(obj, slice):
            children = (obj.start, obj.stop, obj.step)
        else:
            return obj
        table = self._imm_depth
        depth = 1
        for child in children:
            ent = table.get(id(child))
            if ent is not None and ent[0] is child and ent[1] >= depth:
                depth = ent[1] + 1
        if depth > self.max_depth:
            raise UnpicklingError(f"maximum nesting depth {self.max_depth} exceeded")
        if children:
            table[id(obj)] = (obj, depth)
        return obj

    def is_user_class(self, cls: Any) -> bool:
        return self._is_listed(cls) or self._is_extra(cls)

    def is_reconstructible(self, cls: type) -> bool:
        return cls is uuid.UUID or self.is_user_class(cls)

    def _is_extra(self, obj: Any) -> bool:
        ent = self.extra_objs.get(id(obj))
        return ent is not None and ent[0] is obj

    def user_slots(self, cls: type):
        if not self.is_user_class(cls):
            return None
        s = self._slot_cache.get(cls)
        if s is None:
            s = self._slot_cache[cls] = _slot_names(cls)
        return s

    def find_class(self, module: str, name: str) -> Any:
        if not isinstance(module, str) or not isinstance(name, str):
            raise UnpicklingError("global module and name must be str")
        if self.proto < 3 and self.fix_imports:
            if (module, name) in _compat_pickle.NAME_MAPPING:
                module, name = _compat_pickle.NAME_MAPPING[(module, name)]
            elif module in _compat_pickle.IMPORT_MAPPING:
                module = _compat_pickle.IMPORT_MAPPING[module]
        key = (module, name)
        if key in self.allow_pairs:
            obj = importlib.import_module(module)
            for part in name.split("."):
                obj = getattr(obj, part)
            self.extra_objs[id(obj)] = (obj, f"{module}.{name}")
            return obj
        cls = self.user_classes.get(key)
        if cls is not None:
            return cls
        h = _SAFE_TABLE.get(key)
        if h is not None:
            return h.obj
        raise UnsafeTypeError(
            f"global '{module}.{name}' is not allowed "
            "(pass the class via classes= or the pair via allow= if you trust it)"
        )

    def _name(self, obj: Any) -> str:
        h = self.handlers.get(id(obj))
        if h is not None and h.obj is obj:
            return h.name
        ent = self.extra_objs.get(id(obj))
        if ent is not None and ent[0] is obj:
            return ent[1]
        return _qualname(obj)

    def call_reduce(self, func: Any, args: Any) -> Any:
        if type(args) is not tuple:
            raise UnpicklingError("REDUCE arguments must be a tuple")
        if self._is_extra(func):
            return func(*args)  # explicitly trusted via allow=
        if self._is_listed(func):
            cls = func
            if issubclass(cls, enum.Enum):
                if len(args) != 1:
                    raise _bad_args(_qualname(cls), args)
                return cls(args[0])
            if issubclass(cls, BaseException):
                obj = cls.__new__(cls, *args)
                obj.args = args
                return obj
            raise UnsafeTypeError(
                f"REDUCE would call {_qualname(cls)}(*args), running its __init__; "
                "add it to allow= to permit this"
            )
        h = self.handlers.get(id(func))
        if h is None or h.obj is not func or h.reduce is None:
            raise UnsafeTypeError(f"calling {self._name(func)} is not allowed")
        return h.reduce(self, args)

    def call_newobj(self, cls: Any, args: Any, kwargs: Any = None) -> Any:
        if type(args) is not tuple or (kwargs is not None and type(kwargs) is not dict):
            raise UnpicklingError("NEWOBJ arguments must be a tuple (and dict)")
        if not isinstance(cls, type):
            raise UnsafeTypeError(f"NEWOBJ on {self._name(cls)} is not allowed")
        if self.is_user_class(cls):
            if kwargs:
                if not all(type(k) is str for k in kwargs):
                    raise UnpicklingError("NEWOBJ_EX keyword names must be str")
                return cls.__new__(cls, *args, **kwargs)
            return cls.__new__(cls, *args)
        h = self.handlers.get(id(cls))
        if h is not None and h.obj is cls and not kwargs:
            if h.newobj is not None:
                return h.newobj(self, args)
            if h.reduce is not None and cls in (set, frozenset, list, tuple, dict, bytearray,
                                                bytes, int, float, str, complex):
                return h.reduce(self, args)
        raise UnsafeTypeError(f"NEWOBJ on {self._name(cls)} is not allowed")

    def instantiate(self, cls: Any, args: list) -> Any:
        """INST / OBJ (protocol 0/1 instance creation)."""
        if not isinstance(cls, type) or not self.is_user_class(cls):
            raise UnsafeTypeError(f"INST/OBJ for {self._name(cls)} is not allowed")
        if args or hasattr(cls, "__getinitargs__"):
            if self._is_extra(cls):
                return cls(*args)
            raise UnsafeTypeError(
                f"INST/OBJ with arguments would call {_qualname(cls)}.__init__; not allowed"
            )
        return cls.__new__(cls)

    def build(self, inst: Any, state: Any) -> None:
        cls = type(inst)
        if self._is_extra(cls):
            setstate = getattr(inst, "__setstate__", None)
            if setstate is not None:
                setstate(state)  # explicitly trusted via allow=
                return
            _set_state_direct(inst, state)
            return
        if self._is_listed(cls):
            _set_state_direct(inst, state)
            return
        if cls is uuid.UUID:
            _build_uuid(inst, state)
            return
        raise UnsafeTypeError(f"BUILD on object of type {_qualname(cls)} is not allowed")

    # ---- container mutation (builtin base methods only) ---------------

    @staticmethod
    def _extend(target: Any, items: list) -> None:
        if isinstance(target, list):
            list.extend(target, items)
        elif isinstance(target, collections.deque):
            collections.deque.extend(target, items)
        else:
            raise UnsafeTypeError(
                f"APPEND to object of type {_qualname(type(target))} is not allowed"
            )

    @staticmethod
    def _setitems(target: Any, items: list) -> None:
        if len(items) % 2:
            raise UnpicklingError("odd number of items for SETITEMS")
        if isinstance(target, collections.OrderedDict):
            setter = collections.OrderedDict.__setitem__
        elif isinstance(target, dict):
            setter = dict.__setitem__
        else:
            raise UnsafeTypeError(
                f"SETITEM on object of type {_qualname(type(target))} is not allowed"
            )
        for i in range(0, len(items), 2):
            setter(target, items[i], items[i + 1])

    @staticmethod
    def _additems(target: Any, items: list) -> None:
        if isinstance(target, set):
            set.update(target, items)
        else:
            raise UnsafeTypeError(
                f"ADDITEMS on object of type {_qualname(type(target))} is not allowed"
            )

    # ---- input --------------------------------------------------------

    def read(self, n: int) -> bytes:
        if n < 0 or n > len(self.data) - self.pos:
            raise UnpicklingError(
                f"pickle data was truncated (need {n} bytes, "
                f"{len(self.data) - self.pos} remaining)"
            )
        start = self.pos
        self.pos += n
        return self.data[start:self.pos]

    def readline(self) -> bytes:
        i = self.data.find(b"\n", self.pos)
        if i < 0:
            raise UnpicklingError("pickle data was truncated (missing newline)")
        start = self.pos
        self.pos = i + 1
        return self.data[start:self.pos]

    def read_len(self, st: struct.Struct) -> int:
        (n,) = st.unpack(self.read(st.size))
        if n < 0:
            raise UnpicklingError("negative length in pickle data")
        if n > len(self.data) - self.pos:
            raise UnpicklingError(
                f"length {n} exceeds the remaining {len(self.data) - self.pos} bytes of input"
            )
        return n

    def decode_string(self, value: bytes) -> Any:
        if self.encoding == "bytes":
            return value
        return value.decode(self.encoding, self.errors)

    # ---- main loop ----------------------------------------------------

    def run(self) -> Any:
        if not self.data:
            raise EOFError("Ran out of input")
        try:
            return self._run()
        except (IndexError, KeyError, struct.error, RecursionError) as exc:
            if isinstance(exc, UnsafeTypeError):  # pragma: no cover - defensive
                raise
            raise UnpicklingError(f"malformed pickle data: {exc!r}") from None

    def _run(self) -> Any:  # noqa: C901 - one flat dispatch is clearest here
        data = self.data
        stack: list = []
        metastack: list = []
        memo: dict = {}
        max_depth = self.max_depth

        # NB: always call pop_mark() as its own statement; it rebinds ``stack``,
        # so ``stack.append(pop_mark())`` would append to the discarded list.
        def pop_mark() -> list:
            nonlocal stack
            if not metastack:
                raise UnpicklingError("could not find MARK")
            items = stack
            stack = metastack.pop()
            return items

        while True:
            if self.pos >= len(data):
                raise UnpicklingError("pickle data was truncated (no STOP opcode)")
            op = data[self.pos]
            self.pos += 1

            if op == 0x28:  # MARK (
                if len(metastack) >= max_depth:
                    raise UnpicklingError(f"maximum nesting depth {max_depth} exceeded")
                metastack.append(stack)
                stack = []
            elif op == 0x2E:  # STOP .
                if not stack:
                    raise UnpicklingError("STOP with empty stack")
                value = stack.pop()
                _check_depth(value, max_depth, self.user_slots)
                return value
            elif op == 0x30:  # POP 0
                if stack:
                    stack.pop()
                else:
                    pop_mark()
            elif op == 0x31:  # POP_MARK 1
                pop_mark()
            elif op == 0x32:  # DUP 2
                stack.append(stack[-1])
            elif op == 0x46:  # FLOAT F
                stack.append(float(self.readline()[:-1]))
            elif op == 0x49:  # INT I
                line = self.readline()
                if line == b"00\n":
                    stack.append(False)
                elif line == b"01\n":
                    stack.append(True)
                else:
                    stack.append(int(line, 0))
            elif op == 0x4A:  # BININT J
                stack.append(_I32.unpack(self.read(4))[0])
            elif op == 0x4B:  # BININT1 K
                stack.append(self.read(1)[0])
            elif op == 0x4C:  # LONG L
                val = self.readline()[:-1]
                if val and val[-1:] == b"L":
                    val = val[:-1]
                stack.append(int(val, 0))
            elif op == 0x4D:  # BININT2 M
                stack.append(_U16.unpack(self.read(2))[0])
            elif op == 0x4E:  # NONE N
                stack.append(None)
            elif op in (0x50, 0x51):  # PERSID P / BINPERSID Q
                raise UnpicklingError(
                    "persistent IDs are not supported by pysafe_pickle.compat"
                )
            elif op == 0x52:  # REDUCE R
                args = stack.pop()
                stack[-1] = self.track(self.call_reduce(stack[-1], args))
            elif op == 0x53:  # STRING S
                raw = self.readline()[:-1]
                if len(raw) >= 2 and raw[0] == raw[-1] and raw[:1] in (b'"', b"'"):
                    raw = raw[1:-1]
                else:
                    raise UnpicklingError("the STRING opcode argument must be quoted")
                stack.append(self.decode_string(codecs.escape_decode(raw)[0]))
            elif op == 0x54:  # BINSTRING T
                n = self.read_len(_I32)
                stack.append(self.decode_string(self.read(n)))
            elif op == 0x55:  # SHORT_BINSTRING U
                n = self.read_len(_U8)
                stack.append(self.decode_string(self.read(n)))
            elif op == 0x56:  # UNICODE V
                stack.append(str(self.readline()[:-1], "raw-unicode-escape"))
            elif op == 0x58:  # BINUNICODE X
                n = self.read_len(_U32)
                stack.append(str(self.read(n), "utf-8", "surrogatepass"))
            elif op == 0x61:  # APPEND a
                value = stack.pop()
                self._extend(stack[-1], [value])
            elif op == 0x62:  # BUILD b
                state = stack.pop()
                self.build(stack[-1], state)
            elif op == 0x63:  # GLOBAL c
                module = self.readline()[:-1].decode("utf-8")
                name = self.readline()[:-1].decode("utf-8")
                stack.append(self.find_class(module, name))
            elif op == 0x64:  # DICT d
                items = pop_mark()
                if len(items) % 2:
                    raise UnpicklingError("odd number of items for DICT")
                stack.append({items[i]: items[i + 1] for i in range(0, len(items), 2)})
            elif op == 0x7D:  # EMPTY_DICT }
                stack.append({})
            elif op == 0x65:  # APPENDS e
                items = pop_mark()
                self._extend(stack[-1], items)
            elif op == 0x67:  # GET g
                stack.append(memo[int(self.readline()[:-1])])
            elif op == 0x68:  # BINGET h
                stack.append(memo[self.read(1)[0]])
            elif op == 0x69:  # INST i
                module = self.readline()[:-1].decode("ascii")
                name = self.readline()[:-1].decode("ascii")
                cls = self.find_class(module, name)
                args = pop_mark()
                stack.append(self.track(self.instantiate(cls, args)))
            elif op == 0x6A:  # LONG_BINGET j
                stack.append(memo[_U32.unpack(self.read(4))[0]])
            elif op == 0x6C:  # LIST l
                items = pop_mark()
                stack.append(items)
            elif op == 0x5D:  # EMPTY_LIST ]
                stack.append([])
            elif op == 0x6F:  # OBJ o
                args = pop_mark()
                if not args:
                    raise UnpicklingError("OBJ without a class")
                cls = args.pop(0)
                stack.append(self.track(self.instantiate(cls, args)))
            elif op == 0x70:  # PUT p
                i = int(self.readline()[:-1])
                if i < 0:
                    raise UnpicklingError("negative PUT argument")
                memo[i] = stack[-1]
            elif op == 0x71:  # BINPUT q
                memo[self.read(1)[0]] = stack[-1]
            elif op == 0x72:  # LONG_BINPUT r
                memo[_U32.unpack(self.read(4))[0]] = stack[-1]
            elif op == 0x73:  # SETITEM s
                value = stack.pop()
                key = stack.pop()
                self._setitems(stack[-1], [key, value])
            elif op == 0x74:  # TUPLE t
                items = pop_mark()
                stack.append(self.track(tuple(items)))
            elif op == 0x29:  # EMPTY_TUPLE )
                stack.append(())
            elif op == 0x75:  # SETITEMS u
                items = pop_mark()
                self._setitems(stack[-1], items)
            elif op == 0x47:  # BINFLOAT G
                stack.append(_F64.unpack(self.read(8))[0])
            # ---- protocol 2 ----
            elif op == 0x80:  # PROTO
                proto = self.read(1)[0]
                if proto > _HIGHEST_PROTOCOL:
                    raise ValueError(f"unsupported pickle protocol: {proto}")
                self.proto = proto
            elif op == 0x81:  # NEWOBJ
                args = stack.pop()
                stack[-1] = self.track(self.call_newobj(stack[-1], args))
            elif op in (0x82, 0x83, 0x84):  # EXT1 / EXT2 / EXT4
                if op == 0x82:
                    code = self.read(1)[0]
                elif op == 0x83:
                    code = _U16.unpack(self.read(2))[0]
                else:
                    code = _I32.unpack(self.read(4))[0]
                target = copyreg._inverted_registry.get(code)
                where = f" -> {target[0]}.{target[1]}" if target else ""
                raise UnsafeTypeError(
                    f"EXT opcode (copyreg extension code {code}{where}) is not allowed"
                )
            elif op == 0x85:  # TUPLE1
                stack[-1] = self.track((stack[-1],))
            elif op == 0x86:  # TUPLE2
                b = stack.pop()
                stack[-1] = self.track((stack[-1], b))
            elif op == 0x87:  # TUPLE3
                c = stack.pop()
                b = stack.pop()
                stack[-1] = self.track((stack[-1], b, c))
            elif op == 0x88:  # NEWTRUE
                stack.append(True)
            elif op == 0x89:  # NEWFALSE
                stack.append(False)
            elif op == 0x8A:  # LONG1
                n = self.read_len(_U8)
                stack.append(int.from_bytes(self.read(n), "little", signed=True))
            elif op == 0x8B:  # LONG4
                n = self.read_len(_I32)
                stack.append(int.from_bytes(self.read(n), "little", signed=True))
            # ---- protocol 3 ----
            elif op == 0x42:  # BINBYTES B
                n = self.read_len(_U32)
                stack.append(self.read(n))
            elif op == 0x43:  # SHORT_BINBYTES C
                n = self.read_len(_U8)
                stack.append(self.read(n))
            # ---- protocol 4 ----
            elif op == 0x8C:  # SHORT_BINUNICODE
                n = self.read_len(_U8)
                stack.append(str(self.read(n), "utf-8", "surrogatepass"))
            elif op == 0x8D:  # BINUNICODE8
                n = self.read_len(_U64)
                stack.append(str(self.read(n), "utf-8", "surrogatepass"))
            elif op == 0x8E:  # BINBYTES8
                n = self.read_len(_U64)
                stack.append(self.read(n))
            elif op == 0x8F:  # EMPTY_SET
                stack.append(set())
            elif op == 0x90:  # ADDITEMS
                items = pop_mark()
                self._additems(stack[-1], items)
            elif op == 0x91:  # FROZENSET
                items = pop_mark()
                stack.append(self.track(frozenset(items)))
            elif op == 0x92:  # NEWOBJ_EX
                kwargs = stack.pop()
                args = stack.pop()
                stack[-1] = self.track(self.call_newobj(stack[-1], args, kwargs))
            elif op == 0x93:  # STACK_GLOBAL
                name = stack.pop()
                module = stack.pop()
                if type(name) is not str or type(module) is not str:
                    raise UnpicklingError("STACK_GLOBAL requires str")
                stack.append(self.find_class(module, name))
            elif op == 0x94:  # MEMOIZE
                memo[len(memo)] = stack[-1]
            elif op == 0x95:  # FRAME
                self.read_len(_U64)  # framing is an I/O hint; just validate it
            # ---- protocol 5 ----
            elif op == 0x96:  # BYTEARRAY8
                n = self.read_len(_U64)
                stack.append(bytearray(self.read(n)))
            elif op == 0x97:  # NEXT_BUFFER
                raise UnpicklingError(
                    "pickle stream refers to out-of-band data, which is not supported"
                )
            elif op == 0x98:  # READONLY_BUFFER
                top = stack[-1]
                if not isinstance(top, (bytes, bytearray, memoryview)):
                    raise UnpicklingError("READONLY_BUFFER on a non-buffer")
                with memoryview(top) as m:
                    if not m.readonly:
                        stack[-1] = m.toreadonly()
            else:
                raise UnpicklingError(f"invalid load key, {bytes([op])!r}.")


# --------------------------------------------------------------------------
# Public API
# --------------------------------------------------------------------------

def _run_vm(data, classes, allow, encoding, errors, fix_imports, max_depth):
    if isinstance(data, memoryview):
        data = data.tobytes()
    elif isinstance(data, bytearray):
        data = bytes(data)
    elif not isinstance(data, bytes):
        raise TypeError(f"a bytes-like object is required, not {type(data).__name__!r}")
    if not isinstance(max_depth, int) or max_depth < 1:
        raise ValueError("max_depth must be a positive int")
    vm = _SafeVM(
        data,
        classes=classes,
        allow=allow,
        encoding=encoding,
        errors=errors,
        fix_imports=fix_imports,
        max_depth=max_depth,
    )
    return vm.run(), vm.pos


def loads_pickle(
    data: bytes | bytearray | memoryview,
    *,
    classes: Iterable[type] | None = None,
    allow: Iterable[tuple[str, str]] | None = None,
    encoding: str = "ASCII",
    errors: str = "strict",
    fix_imports: bool = True,
    max_depth: int = DEFAULT_MAX_DEPTH,
) -> Any:
    """Safely load one standard pickle (protocols 0-5) from *data*.

    Args:
        data: The pickle bytes. Bytes after the ``STOP`` opcode are ignored,
            as with :func:`pickle.loads`.
        classes: Your own classes (dataclasses, plain classes, namedtuples,
            enums, exceptions) that may be rebuilt. They are created with
            ``cls.__new__``/``object.__new__`` and their state is written
            directly into ``__dict__``/slots. Their ``__init__``,
            ``__setstate__`` and ``__reduce__`` are never called.
        allow: Extra ``(module, name)`` globals that you fully trust. They
            are imported on demand and called with full pickle semantics
            (including ``__setstate__``), so only list things you would
            accept from an attacker.
        encoding, errors: How to decode Python 2 ``str`` (``STRING``
            opcodes), as in :func:`pickle.loads`. ``encoding='bytes'`` keeps
            them as bytes.
        fix_imports: Map Python 2 module names for protocols < 3.
        max_depth: Maximum container nesting depth.

    Raises:
        UnsafeTypeError: The stream references a global or operation that
            is not allowed. Raised before anything is called.
        pickle.UnpicklingError, ValueError, TypeError, EOFError: For
            malformed or over-limit input.
    """
    value, _ = _run_vm(data, classes, allow, encoding, errors, fix_imports, max_depth)
    return value


def load_pickle(
    data_or_file: Any,
    *,
    classes: Iterable[type] | None = None,
    allow: Iterable[tuple[str, str]] | None = None,
    encoding: str = "ASCII",
    errors: str = "strict",
    fix_imports: bool = True,
    max_depth: int = DEFAULT_MAX_DEPTH,
) -> Any:
    """Safely load one standard pickle from bytes, a path or a binary file.

    *data_or_file* may be bytes-like, a ``str``/``os.PathLike`` path, or a
    binary file object with ``read()``. For seekable files the position is
    left right after the pickle's ``STOP`` opcode, so several pickles written
    back to back can be read with repeated calls, as with :func:`pickle.load`.
    Non-seekable streams are read to EOF.

    See :func:`loads_pickle` for the keyword arguments and the exceptions
    raised.
    """
    kw = dict(classes=classes, allow=allow, encoding=encoding, errors=errors,
              fix_imports=fix_imports, max_depth=max_depth)
    if isinstance(data_or_file, (bytes, bytearray, memoryview)):
        return loads_pickle(data_or_file, **kw)
    if isinstance(data_or_file, (str, os.PathLike)):
        with open(data_or_file, "rb") as fh:
            return loads_pickle(fh.read(), **kw)
    read = getattr(data_or_file, "read", None)
    if read is None:
        raise TypeError("load_pickle() expects bytes, a path or a binary file object")
    start = None
    try:
        if data_or_file.seekable():
            start = data_or_file.tell()
    except (AttributeError, OSError, io.UnsupportedOperation):
        start = None
    data = read()
    if not isinstance(data, (bytes, bytearray)):
        raise TypeError("file must be opened in binary mode")
    value, consumed = _run_vm(data, classes, allow, encoding, errors, fix_imports, max_depth)
    if start is not None:
        data_or_file.seek(start + consumed)
    return value


def convert_pickle(
    data: Any,
    *,
    classes: Iterable[type] | None = None,
    allow: Iterable[tuple[str, str]] | None = None,
    encoding: str = "ASCII",
    errors: str = "strict",
    fix_imports: bool = True,
    max_depth: int = DEFAULT_MAX_DEPTH,
    **dumps_kwargs: Any,
) -> bytes:
    """Safely load a standard pickle and re-encode it with :func:`pysafe_pickle.dumps`.

    *data* is anything :func:`load_pickle` accepts. Extra keyword arguments
    (for example ``protocol=`` and ``schema_version=``) go to ``dumps``.

    The output can only hold types that pysafe-pickle can encode: the
    builtin scalars and containers, ``datetime`` types, ``timezone`` and
    ``ZoneInfo``, ``Decimal``, ``Fraction``, ``UUID``, ``OrderedDict``,
    ``Counter``, ``defaultdict`` (builtin factories), ``deque``, ``range``,
    ``slice``, paths, enums, namedtuples and dataclass instances. Plain
    (non-dataclass) classes that this reader can load make ``dumps`` raise
    ``UnsafeTypeError``. To read the result back as your own classes, pass
    the same ``classes=`` to :func:`pysafe_pickle.loads`.
    """
    from pysafe_pickle._pysafe_pickle import dumps

    obj = load_pickle(
        data, classes=classes, allow=allow, encoding=encoding, errors=errors,
        fix_imports=fix_imports, max_depth=max_depth,
    )
    return dumps(obj, **dumps_kwargs)
