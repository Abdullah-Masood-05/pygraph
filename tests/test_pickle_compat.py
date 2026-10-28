"""Tests for pysafe_pickle.compat: the safe reader for standard pickle streams."""
import collections
import copyreg
import datetime
import decimal
import enum
import fractions
import functools
import io
import ipaddress
import math
import operator
import os
import pathlib
import pickle
import struct
import subprocess
import sys
import textwrap
import uuid
from collections import namedtuple
from dataclasses import dataclass, field

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

import pysafe_pickle as psp
from pysafe_pickle import compat
from pysafe_pickle.compat import convert_pickle, load_pickle, loads_pickle
from pysafe_pickle.exceptions import PySafePickleError, UnsafeTypeError

PROTOCOLS = list(range(0, pickle.HIGHEST_PROTOCOL + 1))
MOD = __name__


# ---------------------------------------------------------------------------
# Allowed user classes (module level so pickle can reference them)
# ---------------------------------------------------------------------------

@dataclass
class Point:
    x: int
    y: int
    tags: list = field(default_factory=list)


@dataclass(frozen=True, slots=True)
class FrozenSlots:
    a: int
    b: str = "b"


class Plain:
    def __init__(self, name, value):
        self.name = name
        self.value = value

    def __eq__(self, other):
        return type(other) is Plain and vars(self) == vars(other)


class Slotted:
    __slots__ = ("a", "__b")

    def __init__(self, a, b):
        self.a = a
        self.__b = b

    def get_b(self):
        return self.__b


NT = namedtuple("NT", "x y")


class Color(enum.Enum):
    RED = 1
    GREEN = "green"


class Perm(enum.IntFlag):
    R = 4
    W = 2


class MyError(Exception):
    pass


ALLOWED = [Point, FrozenSlots, Plain, Slotted, NT, Color, Perm, MyError]


# ---------------------------------------------------------------------------
# Sentinels: if any of these ever run, a test fails
# ---------------------------------------------------------------------------

SENTINEL = {"hit": False}


def sentinel_hit(*args, **kwargs):
    SENTINEL["hit"] = True
    return "pwned"


class HookClass:
    """Allowed class whose hooks must never be called by the safe loader."""

    def __init__(self, *args):
        SENTINEL["hit"] = True
        self.v = args

    def __setstate__(self, state):
        SENTINEL["hit"] = True
        self.__dict__.update(state)


class EvilInit:
    """Never allowed. Every hook flips the sentinel."""

    def __new__(cls, *args):
        SENTINEL["hit"] = True
        return object.__new__(cls)

    def __init__(self, *args):
        SENTINEL["hit"] = True

    def __setstate__(self, state):
        SENTINEL["hit"] = True


class Reducer:
    """Pickles as an arbitrary (callable, args) pair."""

    def __init__(self, func, args):
        self.func = func
        self.args = args

    def __reduce__(self):
        return (self.func, self.args)


class ReduceExGame:
    def __init__(self, func, args):
        self.func = func
        self.args = args

    def __reduce_ex__(self, protocol):
        return (self.func, self.args)


@pytest.fixture(autouse=True)
def _reset_sentinel():
    SENTINEL["hit"] = False
    yield
    assert SENTINEL["hit"] is False, "a sentinel hook ran during the test"


def _flag_code(flag):
    return f"open({str(flag)!r}, 'w').close()"


def _flag_cmd(flag):
    return f'"{sys.executable}" -c "{_flag_code(flag)}"'


def _same(a, b, _seen=None):
    """Deep equality that also checks exact types, nan, -0.0 and handles cycles."""
    if _seen is None:
        _seen = set()
    if type(a) is not type(b):
        return False
    key = (id(a), id(b))
    if key in _seen:
        return True
    if isinstance(a, float):
        if math.isnan(a):
            return math.isnan(b)
        return a == b and math.copysign(1, a) == math.copysign(1, b)
    if isinstance(a, complex):
        return _same(a.real, b.real) and _same(a.imag, b.imag)
    if isinstance(a, decimal.Decimal):
        return str(a) == str(b)
    if isinstance(a, (list, tuple, collections.deque)):
        _seen.add(key)
        if isinstance(a, collections.deque) and a.maxlen != b.maxlen:
            return False
        return len(a) == len(b) and all(_same(x, y, _seen) for x, y in zip(a, b))
    if isinstance(a, dict):
        _seen.add(key)
        if getattr(a, "default_factory", None) is not getattr(b, "default_factory", None):
            return False
        return list(a) == list(b) and all(_same(a[k], b[k], _seen) for k in a)
    if isinstance(a, type):
        return a is b
    return a == b


def sbu(s):
    """SHORT_BINUNICODE opcode."""
    raw = s.encode()
    return b"\x8c" + bytes([len(raw)]) + raw


# ---------------------------------------------------------------------------
# Round trips
# ---------------------------------------------------------------------------

def _sample():
    shared = [1, 2, 3]
    return {
        "none": None,
        "bools": [True, False],
        "ints": [0, 1, -1, 255, 256, 65535, 65536, 2**31 - 1, -2**31, 2**31, 2**63, -2**63 - 1,
                 2**1000, -(2**1000)],
        "floats": [0.0, -0.0, 1.5, -2.25, 1e308, 5e-324, float("inf"), float("-inf"), float("nan")],
        "complex": [1j, complex(-1.5, 2.5), complex(float("inf"), -0.0)],
        "strs": ["", "ascii", "line\nbreak", "back\\slash", "quote'\"", "\u00e9\u4e2d\U0001F600",
                 "\x00\x7f\x80\xff", "\ud800", "a\udfffb"],
        "bytes": [b"", b"abc", bytes(range(256))],
        "bytearray": bytearray(b"\x00\xffxyz"),
        "tuple": (1, ("nested", (2.5, None)), ()),
        "set": {1, 2, "three"},
        "frozenset": frozenset({4, 5}),
        "empty": [[], {}, (), set(), frozenset()],
        "nested": {"a": [{"b": ({"c": [1, [2, [3, [4]]]]},)}]},
        "shared": [shared, shared, {"again": shared}],
        "range": range(1, 20, 3),
        "slice": slice(1, None, -1),
        "datetime": [
            datetime.datetime(2024, 2, 29, 23, 59, 59, 999999),
            datetime.datetime(2020, 1, 1, tzinfo=datetime.timezone.utc),
            datetime.datetime(2020, 6, 1, 12, tzinfo=datetime.timezone(datetime.timedelta(hours=5, minutes=30), "IST")),
            datetime.date(1999, 12, 31),
            datetime.time(13, 37, 1, 2),
            datetime.time(1, 2, tzinfo=datetime.timezone(datetime.timedelta(hours=-3))),
            datetime.timedelta(days=-3, seconds=5, microseconds=7),
            datetime.timezone.utc,
        ],
        "decimal": [decimal.Decimal("3.14159265358979323846"), decimal.Decimal("-0"),
                    decimal.Decimal("NaN"), decimal.Decimal("Infinity"), decimal.Decimal("1E+999")],
        "fraction": [fractions.Fraction(1, 3), fractions.Fraction(-22, 7)],
        "uuid": [uuid.UUID("12345678-1234-5678-1234-567812345678"), uuid.uuid4()],
        "ordereddict": collections.OrderedDict([("z", 1), ("a", [2])]),
        "deque": collections.deque([1, 2, 3]),
        "deque_maxlen": collections.deque([1, 2], maxlen=5),
        "counter": collections.Counter("mississippi"),
        "defaultdict": collections.defaultdict(list, {"k": [1, 2]}),
        "defaultdict_int": collections.defaultdict(int, {"n": 3}),
        "paths": [pathlib.PurePosixPath("/etc/x"), pathlib.PureWindowsPath(r"C:\a\b"), pathlib.Path("rel/p")],
        "types": [int, set, collections.OrderedDict, datetime.date],
    }


def _user_sample():
    return [
        Point(1, 2, ["t"]),
        FrozenSlots(5, "x"),
        Plain("n", [1, {"v": 2}]),
        NT(1, "two"),
        Color.RED,
        Color.GREEN,
        Perm.R | Perm.W,
        MyError("boom", 2),
    ]


class TestRoundTrip:
    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_broad_sample(self, protocol):
        obj = _sample()
        data = pickle.dumps(obj, protocol=protocol)
        result = loads_pickle(data)
        assert _same(result, obj)
        assert math.copysign(1.0, result["floats"][1]) == -1.0
        assert math.isnan(result["floats"][-1])
        assert result["strs"][7] == "\ud800"

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    @pytest.mark.parametrize("key", sorted(_sample()))
    def test_each_value(self, protocol, key):
        obj = _sample()[key]
        assert _same(loads_pickle(pickle.dumps(obj, protocol=protocol)), obj)

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_shared_references(self, protocol):
        inner = {"x": [1]}
        obj = [inner, inner, (inner,)]
        result = loads_pickle(pickle.dumps(obj, protocol=protocol))
        assert result[0] is result[1] is result[2][0]
        assert result == obj

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_cycles(self, protocol):
        a = []
        a.append(a)
        d = {}
        d["self"] = d
        obj = [a, d]
        result = loads_pickle(pickle.dumps(obj, protocol=protocol))
        assert result[0][0] is result[0]
        assert result[1]["self"] is result[1]

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_user_classes(self, protocol):
        objs = _user_sample()
        if protocol < 2:
            objs.append(None)  # Slotted needs protocol >= 2
        else:
            objs.append(Slotted(1, [2]))
        result = loads_pickle(pickle.dumps(objs, protocol=protocol), classes=ALLOWED)
        for got, want in zip(result, objs):
            assert type(got) is type(want)
        assert result[0] == objs[0]
        assert result[1] == objs[1]
        assert result[2] == objs[2]
        assert result[3] == objs[3] and result[3].x == 1
        assert result[4] is Color.RED and result[5] is Color.GREEN
        assert result[6] == Perm.R | Perm.W
        assert result[7].args == ("boom", 2)
        if protocol >= 2:
            assert result[8].a == 1 and result[8].get_b() == [2]

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_user_class_hooks_not_called(self, protocol):
        obj = HookClass.__new__(HookClass)
        obj.__dict__["v"] = 42
        data = pickle.dumps(obj, protocol=protocol)
        result = loads_pickle(data, classes=[HookClass])
        assert type(result) is HookClass and result.v == 42
        # _reset_sentinel asserts neither __init__ nor __setstate__ ran

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_user_class_cycle(self, protocol):
        p = Point(1, 2)
        p.tags.append(p)
        result = loads_pickle(pickle.dumps(p, protocol=protocol), classes=[Point])
        assert result.tags[0] is result

    def test_zoneinfo(self):
        zoneinfo = pytest.importorskip("zoneinfo")
        try:
            tz = zoneinfo.ZoneInfo("Europe/Paris")
        except zoneinfo.ZoneInfoNotFoundError:
            pytest.skip("no tz database available")
        obj = datetime.datetime(2024, 7, 1, 12, tzinfo=tz)
        for protocol in PROTOCOLS:
            assert loads_pickle(pickle.dumps(obj, protocol=protocol)) == obj

    def test_python2_style_strings(self):
        # protocol 2 with SHORT_BINSTRING (py2 str) and a py2 module name
        data = b"\x80\x02c__builtin__\nset\nq\x00]q\x01U\x03abcq\x02a\x85Rq\x03."
        assert loads_pickle(data) == {"abc"}
        assert loads_pickle(data, encoding="bytes") == {b"abc"}
        assert loads_pickle(b"S'a\\nb'\np0\n.") == "a\nb"
        assert loads_pickle(b"I01\n.") is True
        assert loads_pickle(b"L123456789012345678901234567890L\n.") == 123456789012345678901234567890

    def test_trailing_data_ignored(self):
        assert loads_pickle(pickle.dumps([1]) + b"garbage") == [1]

    def test_readonly_buffer_opcodes_ok(self):
        assert loads_pickle(pickle.dumps(bytearray(b"ab"), protocol=5)) == bytearray(b"ab")


_leaf = st.one_of(
    st.none(), st.booleans(), st.integers(), st.floats(allow_nan=False),
    st.text(), st.binary(), st.complex_numbers(allow_nan=False),
)
_values = st.recursive(
    _leaf,
    lambda c: st.one_of(
        st.lists(c, max_size=4),
        st.tuples(c, c),
        st.dictionaries(st.text(max_size=5), c, max_size=4),
        st.frozensets(st.integers(), max_size=4),
    ),
    max_leaves=12,
)


@settings(max_examples=150, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(_values, st.sampled_from(PROTOCOLS))
def test_property_roundtrip(value, protocol):
    assert _same(loads_pickle(pickle.dumps(value, protocol=protocol)), value)


# ---------------------------------------------------------------------------
# File API and conversion
# ---------------------------------------------------------------------------

class TestFileAPI:
    def test_load_from_file_object_sequential(self):
        buf = io.BytesIO()
        pickle.dump([1, 2], buf, protocol=0)
        pickle.dump({"a": 1}, buf, protocol=5)
        buf.seek(0)
        assert load_pickle(buf) == [1, 2]
        assert load_pickle(buf) == {"a": 1}
        assert buf.read() == b""

    def test_load_from_path(self, tmp_path):
        p = tmp_path / "data.pkl"
        p.write_bytes(pickle.dumps({"x": (1, 2)}))
        assert load_pickle(p) == {"x": (1, 2)}
        assert load_pickle(str(p)) == {"x": (1, 2)}
        with open(p, "rb") as fh:
            assert load_pickle(fh) == {"x": (1, 2)}

    def test_load_from_bytes_like(self):
        data = pickle.dumps([1, "a"])
        assert load_pickle(data) == [1, "a"]
        assert load_pickle(bytearray(data)) == [1, "a"]
        assert load_pickle(memoryview(data)) == [1, "a"]

    def test_exported_from_package(self):
        assert psp.load_pickle is load_pickle
        assert psp.loads_pickle is loads_pickle
        assert psp.convert_pickle is convert_pickle

    def test_empty_input(self):
        with pytest.raises(EOFError):
            loads_pickle(b"")

    def test_bad_arguments(self):
        with pytest.raises(TypeError):
            loads_pickle(b"N.", classes=["not a class"])
        with pytest.raises(TypeError):
            loads_pickle(b"N.", allow=["os.system"])
        with pytest.raises(TypeError):
            load_pickle(12345)


class TestConvert:
    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_convert_builtins(self, protocol):
        obj = {"a": [1, 2.5, "s", b"b", None, True], "t": (1, (2,)), "s": {1, 2}, "big": 2**200}
        out = convert_pickle(pickle.dumps(obj, protocol=protocol))
        assert out[:4] == b"PSPK"
        assert psp.loads(out) == obj

    def test_convert_dataclass(self):
        out = convert_pickle(pickle.dumps(Point(1, 2, ["z"])), classes=[Point])
        back = psp.loads(out)
        assert (back.x, back.y, back.tags) == (1, 2, ["z"])

    def test_convert_still_safe(self, tmp_path):
        flag = tmp_path / "flag"
        with pytest.raises(UnsafeTypeError):
            convert_pickle(pickle.dumps(Reducer(os.system, (_flag_cmd(flag),))))
        assert not flag.exists()


# ---------------------------------------------------------------------------
# Malicious corpus: every payload must raise UnsafeTypeError and run nothing
# ---------------------------------------------------------------------------

def _malicious_reducers(flag):
    code = _flag_code(flag)
    cmd = _flag_cmd(flag)
    return {
        "os.system": Reducer(os.system, (cmd,)),
        "eval": Reducer(eval, (code,)),
        "exec": Reducer(exec, (code,)),
        "subprocess.Popen": Reducer(subprocess.Popen, ([sys.executable, "-c", code],)),
        "subprocess.call": Reducer(subprocess.call, ([sys.executable, "-c", code],)),
        "builtins.__import__": Reducer(__import__, ("os",)),
        "builtins.open": Reducer(open, (str(flag), "w")),
        "functools.partial(os.system)": Reducer(functools.partial(os.system, cmd), ()),
        "operator.attrgetter": Reducer(operator.attrgetter, ("system",)),
        "operator.methodcaller": Reducer(operator.methodcaller, ("system", cmd)),
        "sentinel function": Reducer(sentinel_hit, (str(flag),)),
        "__reduce_ex__ game": ReduceExGame(os.system, (cmd,)),
        "nested in container": [1, {"k": Reducer(os.system, (cmd,))}],
        "sys.exit": Reducer(sys.exit, (3,)),
    }


class TestMalicious:
    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_reduce_payloads(self, tmp_path, protocol):
        flag = tmp_path / "flag"
        for label, evil in _malicious_reducers(flag).items():
            data = pickle.dumps(evil, protocol=protocol)
            with pytest.raises(UnsafeTypeError) as info:
                loads_pickle(data)
            assert "not allowed" in str(info.value), label
            assert not flag.exists(), label

    @pytest.mark.parametrize("protocol", PROTOCOLS)
    def test_reduce_payloads_even_with_classes(self, tmp_path, protocol):
        # Allowing unrelated classes must not open any door.
        flag = tmp_path / "flag"
        for label, evil in _malicious_reducers(flag).items():
            with pytest.raises(UnsafeTypeError):
                loads_pickle(pickle.dumps(evil, protocol=protocol), classes=ALLOWED)
        assert not flag.exists()

    def test_message_names_global(self, tmp_path):
        data = pickle.dumps(Reducer(os.system, ("echo",)), protocol=0)
        with pytest.raises(UnsafeTypeError, match=r"(nt|posix)\.system"):
            loads_pickle(data)
        with pytest.raises(UnsafeTypeError, match=r"builtins\.eval"):
            loads_pickle(pickle.dumps(Reducer(eval, ("1",)), protocol=4))
        with pytest.raises(UnsafeTypeError, match=r"subprocess\.Popen"):
            loads_pickle(pickle.dumps(Reducer(subprocess.Popen, (["x"],)), protocol=2))

    def test_is_pysafe_error(self):
        with pytest.raises(PySafePickleError):
            loads_pickle(pickle.dumps(Reducer(sentinel_hit, ())))

    def test_getattr_chains(self, tmp_path):
        flag = tmp_path / "flag"
        # getattr(__import__('os'), 'system')(cmd)
        p0 = (b"cbuiltins\ngetattr\n(cbuiltins\n__import__\n(S'os'\ntRS'system'\ntR(V"
              + _flag_cmd(flag).encode() + b"\ntR.")
        with pytest.raises(UnsafeTypeError, match="__import__"):
            loads_pickle(p0)
        # getattr(pathlib.Path, 'write_text') -- allowed class, disallowed attribute
        p1 = b"cbuiltins\ngetattr\n(cpathlib\nPath\nS'write_text'\ntR."
        with pytest.raises(UnsafeTypeError, match="getattr"):
            loads_pickle(p1)
        # getattr(datetime.datetime, '__reduce__') via protocol 4
        p2 = (b"\x80\x04" + sbu("builtins") + sbu("getattr") + b"\x93"
              + sbu("datetime") + sbu("datetime") + b"\x93" + sbu("__reduce__") + b"\x86R.")
        with pytest.raises(UnsafeTypeError, match="getattr"):
            loads_pickle(p2)
        # getattr on an allowed Enum by a non-member name
        p3 = (b"\x80\x04" + sbu("builtins") + sbu("getattr") + b"\x93"
              + sbu(MOD) + sbu("Color") + b"\x93" + sbu("__class__") + b"\x86R.")
        with pytest.raises(UnsafeTypeError, match="getattr"):
            loads_pickle(p3, classes=[Color])
        assert not flag.exists()

    def test_getattr_enum_member_by_name_allowed(self):
        p = (b"\x80\x04" + sbu("builtins") + sbu("getattr") + b"\x93"
             + sbu(MOD) + sbu("Color") + b"\x93" + sbu("RED") + b"\x86R.")
        assert loads_pickle(p, classes=[Color]) is Color.RED

    def test_reconstructor_disallowed_class(self):
        # class not allowed at all -> lookup fails
        p = (f"ccopyreg\n_reconstructor\n(c{MOD}\nEvilInit\ncbuiltins\nobject\nNtR.").encode()
        with pytest.raises(UnsafeTypeError, match="EvilInit"):
            loads_pickle(p)
        # class is an allowed *global* but not reconstructible
        p = b"ccopyreg\n_reconstructor\n(cdatetime\ndatetime\ncbuiltins\nobject\nNtR."
        with pytest.raises(UnsafeTypeError, match="_reconstructor"):
            loads_pickle(p)
        # allowed class, but base is not a safe builtin
        p = (f"ccopyreg\n_reconstructor\n(c{MOD}\nPlain\nc{MOD}\nPlain\nNtR.").encode()
        with pytest.raises(UnsafeTypeError, match="base"):
            loads_pickle(p, classes=[Plain])

    def test_inst_and_obj_opcodes(self):
        inst = f"(i{MOD}\nEvilInit\n.".encode()
        obj = f"(c{MOD}\nEvilInit\no.".encode()
        for p in (inst, obj):
            with pytest.raises(UnsafeTypeError, match="EvilInit"):
                loads_pickle(p)
        # INST/OBJ for disallowed stdlib-allowed callables (would call them)
        with pytest.raises(UnsafeTypeError):
            loads_pickle(b"(cdatetime\ntimedelta\nI1\no.")
        # allowed user class with args would need __init__ -> refused
        with pytest.raises(UnsafeTypeError, match="__init__"):
            loads_pickle(f"(I1\ni{MOD}\nHookClass\n.".encode(), classes=[HookClass])
        # allowed user class with no args: created without __init__
        got = loads_pickle(f"(i{MOD}\nHookClass\n(dS'v'\nI7\nsb.".encode(), classes=[HookClass])
        assert type(got) is HookClass and got.v == 7

    def test_build_on_disallowed_objects(self):
        # BUILD on a timedelta / list / dict
        for p in (
            b"cdatetime\ntimedelta\n(I1\ntR(dS'x'\nI1\nsb.",
            b"\x80\x02]}b.",
            b"\x80\x02}}b.",
        ):
            with pytest.raises(UnsafeTypeError, match="BUILD"):
                loads_pickle(p)

    def test_build_cannot_set_class_via_slots(self):
        # BUILD with slotstate {'__class__': EvilInit} on an allowed slotted class
        payload = (b"\x80\x04" + sbu(MOD) + sbu("Slotted") + b"\x93)\x81"
                   + b"N}" + sbu("__class__") + sbu(MOD) + sbu("Plain") + b"\x93s\x86b.")
        with pytest.raises(UnsafeTypeError, match="__class__"):
            loads_pickle(payload, classes=[Slotted, Plain])

    def test_ext_opcodes(self, tmp_path):
        code = 0x12345678
        copyreg.add_extension(MOD, "sentinel_hit", 240)
        try:
            for protocol in (2, 3, 4, 5):
                data = pickle.dumps(sentinel_hit, protocol=protocol)
                assert b"\x82\xf0" in data
                with pytest.raises(UnsafeTypeError, match="EXT"):
                    loads_pickle(data, classes=ALLOWED)
                full = data[:-1] + b")R."  # and call it
                with pytest.raises(UnsafeTypeError):
                    loads_pickle(full)
        finally:
            copyreg.remove_extension(MOD, "sentinel_hit", 240)
        for raw in (b"\x80\x02\x82\x01.", b"\x80\x02\x83\x01\x01.", b"\x80\x02\x84" + struct.pack("<i", code) + b"."):
            with pytest.raises(UnsafeTypeError, match="EXT"):
                loads_pickle(raw)

    def test_no_data_driven_imports(self, tmp_path, monkeypatch):
        flag = tmp_path / "imported.flag"
        modname = "pysafe_evil_import_probe"
        (tmp_path / f"{modname}.py").write_text(f"{_flag_code(flag)}\ndef f(): pass\n")
        monkeypatch.syspath_prepend(str(tmp_path))
        with pytest.raises(UnsafeTypeError, match=modname):
            loads_pickle(f"c{modname}\nf\n(tR.".encode())
        assert modname not in sys.modules
        assert not flag.exists()

    def test_numpy_reducer_rejected_without_import(self):
        p = b"cnumpy.core.multiarray\n_reconstruct\n(tR."
        with pytest.raises(UnsafeTypeError, match="numpy.core.multiarray._reconstruct"):
            loads_pickle(p)

    @pytest.mark.parametrize("name", [
        "posix\nsystem", "nt\nsystem", "os\npopen", "builtins\nexec", "builtins\ncompile",
        "builtins\nglobals", "builtins\nsetattr", "builtins\ntype", "importlib\nimport_module",
        "sys\nmodules", "types\nFunctionType", "types\nCodeType", "pickle\nloads",
        "copyreg\n__newobj__", "shutil\nrmtree", "runpy\nrun_path", "ctypes\nCDLL",
        "__builtin__\neval", "pathlib\nPath.write_text", "uuid\n_uuid",
        "collections\nnamedtuple", "datetime\nsys",
    ])
    def test_denied_globals(self, name):
        with pytest.raises(UnsafeTypeError):
            loads_pickle(f"c{name}\n.".encode())
        module, attr = name.split("\n")
        with pytest.raises(UnsafeTypeError):
            loads_pickle(b"\x80\x04" + sbu(module) + sbu(attr) + b"\x93.")

    def test_allowed_value_globals_are_not_callable(self):
        with pytest.raises(UnsafeTypeError, match="builtins.object"):
            loads_pickle(b"cbuiltins\nobject\n)R.")

    def test_reduce_on_plain_user_class_refused(self):
        # REDUCE(Plain, args) would run Plain.__init__
        p = f"c{MOD}\nHookClass\n(I1\ntR.".encode()
        with pytest.raises(UnsafeTypeError, match="__init__"):
            loads_pickle(p, classes=[HookClass])

    def test_defaultdict_factory_restricted(self):
        data = pickle.dumps(collections.defaultdict(datetime.date), protocol=2)
        with pytest.raises(UnsafeTypeError, match="factory"):
            loads_pickle(data)

    def test_codecs_encode_only_latin1(self):
        with pytest.raises(UnsafeTypeError, match="latin1"):
            loads_pickle(b"c_codecs\nencode\n(V\x61\nVrot13\ntR.")
        with pytest.raises(UnsafeTypeError, match="latin1"):
            loads_pickle(b"c_codecs\nencode\n(V\x61\nVutf-8\ntR.")

    def test_zoneinfo_key_validation(self):
        for key in ("../../etc/passwd", "/abs", "a//b", "a/../b", "x" * 300, "C:\\Windows"):
            payload = (b"\x80\x04" + sbu("builtins") + sbu("getattr") + b"\x93"
                       + sbu("zoneinfo") + sbu("ZoneInfo") + b"\x93" + sbu("_unpickle")
                       + b"\x86R" + b"X" + struct.pack("<I", len(key)) + key.encode()
                       + b"K\x01\x86R.")
            with pytest.raises(UnsafeTypeError, match="zoneinfo key"):
                loads_pickle(payload)

    def test_allow_opt_in(self):
        data = pickle.dumps(ipaddress.IPv4Address("10.0.0.1"), protocol=4)
        with pytest.raises(UnsafeTypeError, match="ipaddress.IPv4Address"):
            loads_pickle(data)
        assert loads_pickle(data, allow=[("ipaddress", "IPv4Address")]) == ipaddress.IPv4Address("10.0.0.1")

    def test_safe_globals_listing(self):
        assert ("os", "system") not in compat.SAFE_GLOBALS
        assert ("datetime", "datetime") in compat.SAFE_GLOBALS
        assert all(m != "os" and m != "subprocess" for m, _ in compat.SAFE_GLOBALS)


# ---------------------------------------------------------------------------
# Resource bounds
# ---------------------------------------------------------------------------

class TestResourceBounds:
    @pytest.mark.parametrize("payload", [
        b"\x80\x04\x8d" + struct.pack("<Q", 2**62) + b"x.",        # BINUNICODE8
        b"\x80\x04\x8e" + struct.pack("<Q", 2**63 - 1) + b"x.",    # BINBYTES8
        b"\x80\x05\x96" + struct.pack("<Q", 2**40) + b".",          # BYTEARRAY8
        b"\x80\x03X" + struct.pack("<I", 2**32 - 1) + b"abc.",      # BINUNICODE
        b"\x80\x03B" + struct.pack("<I", 2**31) + b".",             # BINBYTES
        b"T" + struct.pack("<i", 2**31 - 1) + b".",                 # BINSTRING
        b"T" + struct.pack("<i", -5) + b".",                        # negative BINSTRING
        b"\x80\x02\x8b" + struct.pack("<i", 2**31 - 1) + b".",      # LONG4
        b"\x80\x02\x8b" + struct.pack("<i", -1) + b".",             # negative LONG4
        b"\x80\x04\x95" + struct.pack("<Q", 2**60) + b"N.",         # FRAME
    ])
    def test_absurd_lengths_rejected_before_allocation(self, payload):
        with pytest.raises(pickle.UnpicklingError, match="length|truncated"):
            loads_pickle(payload)

    def test_mark_depth_limit(self):
        with pytest.raises(pickle.UnpicklingError, match="depth"):
            loads_pickle(b"(" * 100_000 + b".")

    def test_deep_hashable_keys_do_not_overflow_c_stack(self):
        # A 10**6-deep tuple / frozenset / slice used as a set item makes
        # hash() recurse in C. The stdlib unpickler crashes the interpreter
        # with a stack overflow on the tuple case. We must raise first. Run it
        # in a subprocess so a regression can't take down the test run.
        script = textwrap.dedent(
            """
            from pysafe_pickle.compat import loads_pickle
            payloads = [
                b"\\x80\\x04\\x8f(N" + b"\\x85" * 1_000_000 + b"\\x90.",
                b"\\x80\\x04\\x8f(" + b"(" * 900 + b"\\x91" * 900 + b"\\x90.",
            ]
            inner = b"N"
            for _ in range(5000):
                inner = b"\\x8c\\x08builtins\\x8c\\x05slice\\x93" + inner + b"\\x85R"
            payloads.append(b"\\x80\\x04\\x8f(" + inner + b"\\x90.")
            for p in payloads:
                try:
                    loads_pickle(p, max_depth=500)
                except Exception as exc:
                    print("RAISED", type(exc).__name__, exc)
                else:
                    print("LOADED")
            """
        )
        proc = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True, timeout=120)
        assert proc.returncode == 0, proc.stdout + proc.stderr
        assert proc.stdout.count("RAISED UnpicklingError maximum nesting depth") == 3, proc.stdout

    def test_object_depth_limit_without_marks(self):
        n = 5000
        payload = b"\x80\x02" + b"]" * n + b"a" * (n - 1) + b"."
        with pytest.raises(pickle.UnpicklingError, match="depth"):
            loads_pickle(payload)

    def test_depth_limit_through_memo_bypass(self):
        # Attach each child to its parent *before* deepening the child.
        n = 3000
        out = bytearray(b"\x80\x04]\x94")  # root list, memo 0
        for i in range(n):
            # push memo[i], new empty list (memo i+1), append it to memo[i], pop parent
            out += b"j" + struct.pack("<I", i) + b"]\x94a0"
        out += b"."
        with pytest.raises(pickle.UnpicklingError, match="depth"):
            loads_pickle(bytes(out))
        root = loads_pickle(bytes(out), max_depth=n + 5)
        depth = 0
        while root:
            root = root[0]
            depth += 1
        assert depth == n

    def test_depth_limit_configurable(self):
        assert loads_pickle(pickle.dumps([[[[1]]]])) == [[[[1]]]]
        with pytest.raises(pickle.UnpicklingError, match="depth"):
            loads_pickle(pickle.dumps([[[[1]]]]), max_depth=3)
        nested = Point(1, 2, [Point(3, 4, [[[]]])])
        with pytest.raises(pickle.UnpicklingError, match="depth"):
            loads_pickle(pickle.dumps(nested), classes=[Point], max_depth=4)

    def test_copy_amplification_budget(self):
        # list(L) repeated through memo references: quadratic blow-up attempt
        out = bytearray(b"\x80\x04(" + b"N" * 20000 + b"l\x94")  # memo 0 = 20000-item list
        out += b"("
        for _ in range(20000):
            out += b"\x8c\x08builtins\x8c\x04list\x93h\x00\x85R0"
        out += b"1N."
        with pytest.raises(pickle.UnpicklingError, match="resource limit"):
            loads_pickle(bytes(out))

    @pytest.mark.parametrize("payload", [
        b"cbuiltins\nbytes\n(I1000000000000\ntR.",
        b"cbuiltins\nbytearray\n(I1000000000000\ntR.",
        b"cbuiltins\nlist\n(cbuiltins\nrange\n(I1000000000000\ntRtR.",
        b"cfractions\nFraction\n(V1e999999999\ntR.",
    ])
    def test_allocating_constructor_forms_rejected(self, payload):
        with pytest.raises(pickle.UnpicklingError, match="invalid arguments"):
            loads_pickle(payload)

    @pytest.mark.parametrize("payload", [
        b"\x80\x06N.", b"Z.", b"N", b"(t", b"0.", b"1.", b"\x80\x02h\x05.", b"g9\n.",
        b"S'unquoted\n.", b"\x80\x02P", b"\x80\x05\x97.", b"\x80\x02}N\x85u.",
        b"\x80\x04K\x01K\x02\x93.",
    ])
    def test_malformed_raise_python_errors(self, payload):
        with pytest.raises(Exception) as info:
            loads_pickle(payload)
        assert not isinstance(info.value, UnsafeTypeError) or "not allowed" in str(info.value)


# ---------------------------------------------------------------------------
# Fuzz: byte mutations of valid pickles, in a subprocess with a timeout
# ---------------------------------------------------------------------------

_FUZZ_SCRIPT = textwrap.dedent(
    """
    import pickle, random, sys, datetime, decimal, collections, uuid, fractions
    from pysafe_pickle.compat import loads_pickle

    rng = random.Random(1234)
    seeds = []
    sample = [
        {"a": [1, 2.5, "x\\u20ac", b"\\x00\\xff", None, True], "b": (1, (2, 3)), "s": {1, 2}},
        [2**100, -1, 0.0, float("nan"), 1j, bytearray(b"ab"), frozenset({"q"})],
        datetime.datetime(2020, 1, 2, 3, tzinfo=datetime.timezone.utc),
        decimal.Decimal("1.25"), fractions.Fraction(3, 7), uuid.UUID(int=99),
        collections.OrderedDict(a=1), collections.deque([1, 2], 4),
        collections.Counter("abca"), collections.defaultdict(list, k=[1]),
        range(5), slice(1, 2), "\\ud800",
    ]
    shared = [1]
    sample.append([shared, shared])
    for p in range(pickle.HIGHEST_PROTOCOL + 1):
        for obj in sample:
            seeds.append(pickle.dumps(obj, protocol=p))

    n_ok = n_err = 0
    for i in range(int(sys.argv[1])):
        data = bytearray(rng.choice(seeds))
        for _ in range(rng.randint(1, 4)):
            if not data:
                data = bytearray(b"N")
            kind = rng.random()
            pos = rng.randrange(len(data))
            if kind < 0.5:
                data[pos] = rng.randrange(256)
            elif kind < 0.7:
                data.insert(pos, rng.randrange(256))
            elif kind < 0.85:
                del data[pos]
            else:
                data = data[:pos] or bytearray(b"N")
        try:
            loads_pickle(bytes(data))
            n_ok += 1
        except Exception:
            n_err += 1
        except BaseException as exc:
            print("NON-EXCEPTION", type(exc).__name__, bytes(data))
            sys.exit(2)
    print("FUZZ-DONE", n_ok, n_err)
    """
)


def test_fuzz_mutations_no_crash_no_hang():
    proc = subprocess.run(
        [sys.executable, "-c", _FUZZ_SCRIPT, "100000"],
        capture_output=True,
        text=True,
        timeout=240,
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "FUZZ-DONE" in proc.stdout
