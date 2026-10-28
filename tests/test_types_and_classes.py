"""Stdlib value types, enums / namedtuples, and real classes via ``classes=``."""
import collections
import dataclasses
import datetime as dt
import decimal
import enum
import fractions
import io
import os
import pathlib
import sys
import typing
import uuid
import zoneinfo
from collections import Counter, OrderedDict, defaultdict, deque, namedtuple
from dataclasses import dataclass, field

import pytest
from hypothesis import given, settings, strategies as st

import pysafe_pickle as psp
from pysafe_pickle.migrations import migrate


def rt(obj, **kw):
    return psp.loads(psp.dumps(obj), **kw)


def same(a, b):
    """Deep equality that also requires identical types at every level."""
    if type(a) is not type(b):
        return False
    if isinstance(a, decimal.Decimal):
        return str(a) == str(b)
    if isinstance(a, dt.datetime) or isinstance(a, dt.time):
        return a == b and a.fold == b.fold and a.tzinfo == b.tzinfo and str(a) == str(b)
    if isinstance(a, (list, tuple, deque)):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        return list(a.keys()) == list(b.keys()) and all(same(a[k], b[k]) for k in a)
    return a == b


try:
    NY = zoneinfo.ZoneInfo("America/New_York")
except zoneinfo.ZoneInfoNotFoundError:  # e.g. Windows without the tzdata package
    pytest.skip("no IANA time zone database; pip install tzdata", allow_module_level=True)

STD_VALUES = [
    dt.datetime(2024, 3, 10, 1, 2, 3, 456789),
    dt.datetime(1, 1, 1),
    dt.datetime(9999, 12, 31, 23, 59, 59, 999999),
    dt.datetime(2024, 3, 10, 1, 2, 3, tzinfo=dt.timezone.utc),
    dt.datetime(2024, 1, 1, tzinfo=dt.timezone(dt.timedelta(hours=5, minutes=30), "IST")),
    dt.datetime(2024, 7, 1, 12, tzinfo=NY),
    dt.datetime(2024, 11, 3, 1, 30, tzinfo=NY, fold=1),
    dt.date(1999, 12, 31),
    dt.time(23, 59, 1, 5),
    dt.time(1, 2, tzinfo=dt.timezone(dt.timedelta(hours=-3), "BRT"), fold=1),
    dt.timedelta(days=-3, seconds=5, microseconds=7),
    dt.timedelta.max,
    dt.timedelta.min,
    dt.timezone.utc,
    dt.timezone(dt.timedelta(minutes=-90)),
    dt.timezone(dt.timedelta(0), "Zulu"),
    NY,
    zoneinfo.ZoneInfo("UTC"),
    decimal.Decimal("1.2300E+5"),
    decimal.Decimal("-0"),
    decimal.Decimal("0E-30"),
    decimal.Decimal("NaN"),
    decimal.Decimal("-sNaN"),
    decimal.Decimal("Infinity"),
    decimal.Decimal("-Infinity"),
    decimal.Decimal("3.14159265358979323846264338327950288419716939937510"),
    uuid.UUID("12345678-1234-5678-1234-567812345678"),
    uuid.uuid4(),
    fractions.Fraction(-3, 7),
    fractions.Fraction(10**40, 3),
    range(10),
    range(1, 10**30, 3),
    range(5, -5, -2),
    slice(1, "a", None),
    slice(None),
    ...,
    NotImplemented,
    pathlib.PurePosixPath("/a/b/c.txt"),
    pathlib.PureWindowsPath(r"C:\x\y"),
    pathlib.PureWindowsPath(r"\\server\share\f"),
    OrderedDict(a=1, b=[2]),
    OrderedDict(),
    defaultdict(list, x=[1]),
    defaultdict(None, x=1),
    defaultdict(int),
    Counter("hello world"),
    deque([1, "2", 3.0]),
    deque([1, 2, 3], maxlen=5),
    deque(maxlen=0),
]


class TestStdlibTypes:
    @pytest.mark.parametrize("value", STD_VALUES, ids=lambda v: type(v).__name__)
    def test_roundtrip_exact_type(self, value):
        result = rt(value)
        assert type(result) is type(value)
        if isinstance(value, decimal.Decimal):
            assert str(result) == str(value)
        else:
            assert same(result, value)

    def test_singletons_identity(self):
        a, b = rt([..., NotImplemented])
        assert a is ... and b is NotImplemented

    def test_aware_datetime_tzinfo(self):
        d = dt.datetime(2024, 11, 3, 1, 30, tzinfo=NY, fold=1)
        r = rt(d)
        assert r.tzinfo is NY  # ZoneInfo cache: same object
        assert r.fold == 1 and r.utcoffset() == d.utcoffset()
        assert rt(dt.datetime(2024, 1, 1, tzinfo=dt.timezone.utc)).tzinfo is dt.timezone.utc

    def test_shared_tzinfo_one_record(self):
        tz = dt.timezone(dt.timedelta(hours=2), "X")
        items = [dt.datetime(2020, 1, d, tzinfo=tz) for d in range(1, 20)]
        r = rt(items)
        assert all(x.tzinfo is r[0].tzinfo for x in r)
        assert r == items

    def test_timezone_name_preserved(self):
        assert rt(dt.timezone(dt.timedelta(hours=1), "CET")).tzname(None) == "CET"
        assert rt(dt.timezone(dt.timedelta(hours=1))).tzname(None) == "UTC+01:00"

    def test_decimal_specials(self):
        for s in ["NaN", "-NaN", "sNaN", "Infinity", "-Infinity", "-0", "-0.000", "1E+999999"]:
            r = rt(decimal.Decimal(s))
            assert type(r) is decimal.Decimal and str(r) == str(decimal.Decimal(s))
        assert rt(decimal.Decimal("-0")).is_signed()

    def test_concrete_path_matches_os(self):
        p = pathlib.Path("some") / "file.txt"
        r = rt(p)
        assert type(r) is type(p) and r == p

    def test_foreign_concrete_path_loads_as_pure(self):
        # A WindowsPath payload loads as PureWindowsPath on POSIX and a
        # PosixPath payload as PurePosixPath on Windows.
        data = bytearray(psp.dumps(pathlib.PurePosixPath("/etc/x")))
        flavor_at = data.index(b"/etc/x") - 5
        assert data[flavor_at] == 0
        data[flavor_at] = 2  # PosixPath
        r = psp.loads(bytes(data))
        expected = pathlib.PurePosixPath if os.name == "nt" else pathlib.PosixPath
        assert type(r) is expected and str(r) == "/etc/x"

    def test_ordereddict_order_and_move_to_end(self):
        od = OrderedDict(a=1, b=2, c=3)
        od.move_to_end("a")
        od.move_to_end("c", last=False)
        r = rt(od)
        assert type(r) is OrderedDict
        assert list(r.items()) == list(od.items()) == [("c", 3), ("b", 2), ("a", 1)]
        r.move_to_end("c")
        assert list(r) == ["b", "a", "c"]

    def test_defaultdict_factory_preserved(self):
        for factory in [list, dict, set, int, float, str, tuple, bytes, frozenset, bool, complex, None]:
            d = defaultdict(factory, k=1)
            r = rt(d)
            assert type(r) is defaultdict and r.default_factory is factory and r == d
        r = rt(defaultdict(list))
        r["new"].append(1)
        assert r == {"new": [1]}

    def test_counter(self):
        c = Counter(a=3, b=-1, c=0)
        r = rt(c)
        assert type(r) is Counter and r == c and r["zzz"] == 0
        assert r.most_common(1) == [("a", 3)]

    def test_deque_maxlen(self):
        d = deque(range(10), maxlen=4)
        r = rt(d)
        assert type(r) is deque and r.maxlen == 4 and list(r) == [6, 7, 8, 9]
        assert rt(deque([1])).maxlen is None

    def test_shared_and_cyclic_deque(self):
        d = deque([1])
        d.append(d)
        r = rt(d)
        assert r[1] is r
        inner = deque([1, 2])
        r = rt([inner, inner])
        assert r[0] is r[1]

    def test_shared_and_cyclic_ordereddict(self):
        od = OrderedDict(x=1)
        od["self"] = od
        r = rt(od)
        assert type(r) is OrderedDict and r["self"] is r and list(r) == ["x", "self"]
        shared = OrderedDict(a=[1])
        r = rt({"p": shared, "q": shared})
        assert r["p"] is r["q"]

    def test_cyclic_defaultdict_and_counter(self):
        d = defaultdict(list)
        d["me"] = d
        r = rt(d)
        assert r["me"] is r and r.default_factory is list
        c = Counter(a=1)
        r = rt([c, c])
        assert r[0] is r[1] and type(r[0]) is Counter

    def test_shared_immutables(self):
        u = uuid.uuid4()
        f = fractions.Fraction(1, 3)
        r = rt([u, u, f, f])
        assert r[0] is r[1] and r[2] is r[3]

    def test_slice_with_containers(self):
        s = slice([1], {"a": 2}, (3,))
        assert rt(s) == s

    def test_nested_in_dataclass(self):
        @dataclass
        class Event:
            when: dt.datetime
            amount: decimal.Decimal
            tags: deque

        e = Event(dt.datetime(2024, 1, 1, tzinfo=NY), decimal.Decimal("9.99"), deque(["a"], maxlen=3))
        r = rt(e)
        assert r.when == e.when and r.amount == e.amount and r.tags.maxlen == 3

    def test_new_types_as_dict_keys(self):
        d = {dt.date(2020, 1, 1): 1, uuid.UUID(int=5): 2, decimal.Decimal("1.5"): 3,
             fractions.Fraction(1, 2): 4, pathlib.PurePosixPath("a"): 5, range(3): 6}
        assert same(rt(d), d)


class TestRejections:
    def test_defaultdict_lambda(self):
        with pytest.raises(TypeError, match="Unsupported type: defaultdict with factory"):
            psp.dumps(defaultdict(lambda: 0))

    def test_defaultdict_custom_class_factory(self):
        class Box(list):
            pass

        with pytest.raises(TypeError, match="Unsupported type: defaultdict with factory .*Box"):
            psp.dumps(defaultdict(Box))

    def test_subclasses_rejected(self):
        class MyDate(dt.date):
            pass

        class MyDecimal(decimal.Decimal):
            pass

        class MyOD(OrderedDict):
            pass

        for obj in [MyDate(2020, 1, 1), MyDecimal(1), MyOD(), type("MyDeque", (deque,), {})()]:
            with pytest.raises(TypeError, match="Unsupported type"):
                psp.dumps(obj)

    def test_custom_tzinfo_rejected(self):
        class MyTZ(dt.tzinfo):
            def utcoffset(self, d):
                return dt.timedelta(0)

        with pytest.raises(TypeError, match="Unsupported type: tzinfo MyTZ"):
            psp.dumps(dt.datetime(2020, 1, 1, tzinfo=MyTZ()))

    def _zoneinfo_payload(self, key: bytes) -> bytes:
        orig = b"Europe/Paris"
        data = psp.dumps(zoneinfo.ZoneInfo(orig.decode()))
        blob = len(orig).to_bytes(4, "little") + orig
        assert data.count(blob) == 1
        return data.replace(blob, len(key).to_bytes(4, "little") + key)

    @pytest.mark.parametrize("key", [b"../x", b"/etc/passwd", b"a/../../b", b"C:\\x", b"..", b"a//b", b""])
    def test_zoneinfo_bad_key(self, key):
        with pytest.raises(ValueError, match="Invalid ZoneInfo key"):
            psp.loads(self._zoneinfo_payload(key))

    def test_zoneinfo_payload_helper_sane(self):
        assert psp.loads(self._zoneinfo_payload(b"Asia/Tokyo")) is zoneinfo.ZoneInfo("Asia/Tokyo")

    def test_enum_without_classes(self):
        class Color(enum.Enum):
            RED = 1

        data = psp.dumps({"c": Color.RED})
        with pytest.raises(TypeError, match=r"Unsupported type: enum Color \(pass it via classes=\)"):
            psp.loads(data)

    def test_tampered_tzinfo_ref(self):
        # Point a datetime's tzinfo id at a non-tz record.
        d = dt.datetime(2020, 1, 1, tzinfo=dt.timezone.utc)
        data = bytearray(psp.dumps([d, 5]))
        # record layout: list(0) -> datetime(1), int(2); tz is record 3
        idx = data.index(bytes([0x13]))
        data[idx + 13: idx + 17] = (2).to_bytes(4, "little")
        with pytest.raises(ValueError, match="tzinfo"):
            psp.loads(bytes(data))

    def test_truncated_new_records(self):
        for v in [dt.datetime(2020, 1, 1), uuid.uuid4(), deque([1, 2]), OrderedDict(a=1)]:
            data = psp.dumps(v)
            for cut in range(1, 6):
                with pytest.raises(ValueError):
                    psp.loads(data[:-cut])


# --- enums / namedtuples ------------------------------------------------------


class Color(enum.Enum):
    RED = 1
    GREEN = "g"


class Level(enum.IntEnum):
    LOW = 1
    HIGH = 2


class Perm(enum.Flag):
    R = 1
    W = 2
    X = 4


Point = namedtuple("Point", "x y")


class Pair(typing.NamedTuple):
    left: int
    right: str = "r"


class TestEnums:
    def test_roundtrip_identity(self):
        vals = [Color.RED, Color.GREEN, Level.HIGH, Perm.R, Perm.R | Perm.X, Perm(0)]
        r = rt(vals, classes=[Color, Level, Perm])
        assert all(a is b for a, b in zip(r, vals))

    def test_classes_as_dict_and_allowlist(self):
        data = psp.dumps([Color.RED])
        assert psp.loads(data, classes={"Color": Color})[0] is Color.RED
        assert psp.loads(data, classes={"Color": Color}, allowlist={"Color"})[0] is Color.RED
        with pytest.raises(TypeError, match="not in the allowlist"):
            psp.loads(data, classes=[Color], allowlist={"Other"})

    def test_wrong_kind_of_class(self):
        data = psp.dumps(Color.RED)
        with pytest.raises(TypeError, match="must be an Enum"):
            psp.loads(data, classes={"Color": Point})

    def test_unknown_member(self):
        class Color(enum.Enum):  # same name, different members
            BLUE = 3

        with pytest.raises(ValueError, match="no member named 'RED'"):
            psp.loads(psp.dumps(globals()["Color"].RED), classes=[Color])

    @pytest.mark.skipif(sys.version_info < (3, 11), reason="StrEnum is 3.11+")
    def test_strenum(self):
        class S(enum.StrEnum):
            A = "a"

        assert rt(S.A, classes=[S]) is S.A


class TestNamedTuples:
    def test_dynamic_roundtrip(self):
        p = Point(1, [2])
        r = rt(p)
        assert r == p and r._fields == ("x", "y") and isinstance(r, tuple)
        assert type(r).__name__ == "Point" and r.y == [2]

    def test_with_classes(self):
        r = rt([Point(1, 2), Pair(3)], classes=[Point, Pair])
        assert type(r[0]) is Point and type(r[1]) is Pair
        assert r == [Point(1, 2), Pair(3, "r")]

    def test_field_mapping_and_defaults(self):
        Old = namedtuple("Pair", "left")
        r = rt(Old(9), classes=[Pair])
        assert type(r) is Pair and r == Pair(9, "r")

    def test_extra_field_rejected(self):
        Old = namedtuple("Pair", "left right extra")
        with pytest.raises(TypeError, match="'extra'"):
            rt(Old(1, "x", 2), classes=[Pair])

    def test_shared_namedtuple(self):
        p = Point(1, 2)
        r = rt([p, p], classes=[Point])
        assert r[0] is r[1]

    def test_bad_names_in_data_rejected(self):
        data = psp.dumps(Point(1, 2)).replace(b"Point", b"Po nt")
        with pytest.raises(ValueError, match="Invalid namedtuple"):
            psp.loads(data)


# --- dataclasses via classes= ------------------------------------------------

CALLS = []


@dataclass
class Node:
    value: int
    children: list = field(default_factory=list)
    parent: "Node | None" = None

    def __post_init__(self):
        CALLS.append("post_init")


@dataclass(frozen=True)
class Frozen:
    a: int
    b: tuple = ()


@dataclass(slots=True)
class Slotted:
    a: int
    b: str = "x"


@dataclass(kw_only=True)
class KwOnly:
    a: int
    b: int = 2


@dataclass(frozen=True, slots=True)
class FrozenSlotted:
    a: int
    items: list = field(default_factory=list)


@dataclass
class NoInit:
    a: int

    def __init__(self, *args, **kwargs):
        CALLS.append("init")
        raise AssertionError("__init__ must not run on load")


class TestClassesDataclasses:
    def setup_method(self):
        CALLS.clear()

    def test_real_class_and_no_init(self):
        n = Node(1)
        CALLS.clear()
        r = rt(n, classes=[Node])
        assert type(r) is Node and isinstance(r, Node) and r == n
        assert CALLS == []

    def test_init_not_called(self):
        obj = object.__new__(NoInit)
        obj.a = 5
        r = rt(obj, classes=[NoInit])
        assert type(r) is NoInit and r.a == 5 and CALLS == []

    def test_without_classes_is_dynamic(self):
        r = rt(Node(1))
        assert type(r) is not Node and r.value == 1

    def test_cycles(self):
        root = Node(0)
        child = Node(1, parent=root)
        root.children.append(child)
        root.children.append(root)
        r = rt(root, classes=[Node])
        assert type(r) is Node and r.children[1] is r and r.children[0].parent is r
        assert type(r.children[0]) is Node

    @pytest.mark.parametrize(
        "obj",
        [Frozen(1, (2,)), Slotted(3, "y"), KwOnly(a=1, b=5), FrozenSlotted(1, [2])],
        ids=["frozen", "slots", "kw_only", "frozen_slots"],
    )
    def test_dataclass_variants(self, obj):
        cls = type(obj)
        r = rt(obj, classes=[cls])
        assert type(r) is cls and r == obj

    def test_frozen_slots_cycle(self):
        x = FrozenSlotted(1)
        x.items.append(x)
        r = rt(x, classes=[FrozenSlotted])
        assert r.items[0] is r

    def test_missing_field_gets_default(self):
        @dataclass
        class Node:  # old shape: only `value`
            value: int

        r = rt(Node(7), classes={"Node": globals()["Node"]})
        real = globals()["Node"]
        assert type(r) is real and r.value == 7 and r.children == [] and r.parent is None
        r2 = rt(Node(8), classes=[real])
        assert r2.children is not r.children  # default_factory called per instance

    def test_missing_field_without_default(self):
        @dataclass
        class Slotted:
            b: str

        with pytest.raises(TypeError, match="no value for field 'a'"):
            rt(Slotted("q"), classes={"Slotted": globals()["Slotted"]})

    def test_extra_unknown_field_rejected(self):
        @dataclass
        class Frozen:
            a: int
            surprise: int
            other: int = 0

        with pytest.raises(TypeError, match="'surprise', 'other'"):
            rt(Frozen(1, 2), classes={"Frozen": globals()["Frozen"]})

    def test_not_a_dataclass(self):
        with pytest.raises(TypeError, match="must be a dataclass"):
            rt(Frozen(1), classes={"Frozen": Point})

    def test_only_listed_classes_are_real(self):
        r = rt([Frozen(1), Slotted(2)], classes=[Frozen])
        assert type(r[0]) is Frozen and type(r[1]) is not Slotted

    def test_classes_validation(self):
        data = psp.dumps(1)
        with pytest.raises(TypeError):
            psp.loads(data, classes=[1])
        with pytest.raises(TypeError):
            psp.loads(data, classes="Frozen")
        A1 = type("A", (), {})
        A2 = type("A", (), {})
        with pytest.raises(ValueError, match="two different classes named 'A'"):
            psp.loads(data, classes=[A1, A2])

    def test_load_and_unpickler(self):
        buf = io.BytesIO()
        psp.dump(Frozen(4), buf)
        buf.seek(0)
        assert type(psp.load(buf, classes=[Frozen])) is Frozen
        buf.seek(0)
        assert type(psp.Unpickler(buf, classes=[Frozen]).load()) is Frozen

    def test_allowlist_checked_first(self):
        data = psp.dumps(Frozen(1))
        with pytest.raises(TypeError, match="not in the allowlist"):
            psp.loads(data, classes=[Frozen], allowlist={"Other"})


# --- automatic migrations with classes= ------------------------------------------


def _make_account_v1():
    @dataclass
    class Account:
        __pysafe_pickle_version__ = 1
        owner: str
        cents: int

    return Account


@dataclass
class Account:
    __pysafe_pickle_version__ = 2
    owner: str
    balance: decimal.Decimal
    currency: str = "USD"


@migrate(from_version=1, to_version=2, type_name="Account")
def _account_v1_to_v2(state: dict) -> dict:
    cents = state.pop("cents")
    return {**state, "balance": decimal.Decimal(cents) / 100}


class TestMigrationWithClasses:
    def test_v1_to_v2_runs_automatically(self):
        old = _make_account_v1()("ann", 1234)
        data = psp.dumps([old, old])
        r = psp.loads(data, classes=[Account])
        assert type(r[0]) is Account and r[0] is r[1]
        assert r[0] == Account("ann", decimal.Decimal("12.34"), "USD")
        assert not hasattr(r[0], "cents")

    def test_same_version_no_migration(self):
        r = rt(Account("bob", decimal.Decimal("1")), classes=[Account])
        assert r == Account("bob", decimal.Decimal("1"))

    def test_without_classes_unchanged(self):
        r = rt(_make_account_v1()("ann", 1))
        assert r.cents == 1 and not hasattr(r, "balance")

    def test_migration_leaving_extra_field_rejected(self):
        @dataclass
        class Gadget:
            __pysafe_pickle_version__ = 1
            name: str
            legacy: int

        @dataclass
        class GadgetV2:
            __pysafe_pickle_version__ = 2
            name: str

        @migrate(from_version=1, to_version=2, type_name="Gadget")
        def _noop(state):
            return state

        with pytest.raises(TypeError, match="'legacy'"):
            rt(Gadget("g", 1), classes={"Gadget": GadgetV2})


# --- wire compatibility ------------------------------------------------------------


_GOLDEN_MIXED = (
    "5053504b01000000000000090000000a0000007072696d697469766573060000006e6573746564050000"
    "006279746573080000007365745f646174610500000068656c6c6f04000000646565700500000076616c"
    "7565030000006b65790300000076616c000000001a0000000a0400000001000000020000000300000004"
    "0000000500000006000000070000000800000006000000000805000000090000000a0000000b0000000c"
    "0000000d00000006010000000a010000000e0000000f0000000602000000070400000000010203060300"
    "00000805000000090000001000000011000000120000001300000004020500000000000004400604000000"
    "020106050000000a010000001400000015000000040404060408040a0606000000080300000009000000"
    "100000001600000008030000001100000012000000170000000a01000000180000001900000006070000"
    "000608000000"
)
_GOLDEN_PERSON = (
    "5053504b010000000000000c00000006000000506572736f6e040000006e616d65030000006167650700"
    "00006164647265737305000000416c6963650700000041646472657373060000007374726565740400"
    "000063697479080000007a69705f636f64650b000000313233204d61696e2053740b000000537072696e"
    "676669656c64050000003132333435020000000000000000000000000003000100000002000000030000"
    "00010005000000000000000300060000000700000008000000070000000d000003000100000002000000"
    "030000000604000000043c0d010003000400000005000000060000000609000000060a000000060b000000"
)


def test_plain_data_encoding_unchanged():
    """Data without the new types encodes byte-for-byte as v1.3.4 did."""
    sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
    try:
        from benchmarks.bench_data import make_mixed_container, make_nested_dataclass
    finally:
        sys.path.pop(0)
    assert psp.dumps(make_mixed_container()).hex() == _GOLDEN_MIXED
    assert psp.dumps(make_nested_dataclass()).hex() == _GOLDEN_PERSON
    assert psp.loads(bytes.fromhex(_GOLDEN_MIXED)) == make_mixed_container()


# --- property-based ------------------------------------------------------------------

_tz = st.one_of(
    st.none(),
    st.just(dt.timezone.utc),
    st.builds(
        dt.timezone,
        st.integers(-23 * 60 + 1, 23 * 60 - 1).map(lambda m: dt.timedelta(minutes=m)),
    ),
    st.sampled_from([NY, zoneinfo.ZoneInfo("Europe/London"), zoneinfo.ZoneInfo("Asia/Kolkata")]),
)
_seg = st.text("abcxyz_-.0123", min_size=1, max_size=6).filter(lambda s: s not in (".", ".."))

_new_scalars = st.one_of(
    st.datetimes(timezones=_tz),
    st.dates(),
    st.times(timezones=_tz),
    st.timedeltas(),
    st.decimals(allow_nan=False),
    st.uuids(),
    st.fractions(),
    st.builds(range, st.integers(-50, 50), st.integers(-50, 50), st.integers(1, 5)),
    st.just(...),
    st.just(NotImplemented),
    st.lists(_seg, min_size=1, max_size=3).map(lambda p: pathlib.PurePosixPath(*p)),
    st.lists(_seg, min_size=1, max_size=3).map(lambda p: pathlib.PureWindowsPath(*p)),
    st.sampled_from([Color.RED, Level.LOW, Perm.R | Perm.W]),
    st.builds(Point, st.integers(), st.text(max_size=5)),
)
_base = st.one_of(st.none(), st.booleans(), st.integers(), st.text(max_size=8), st.binary(max_size=8), _new_scalars)
_keys = st.one_of(st.integers(), st.text(max_size=5), st.dates(), st.uuids())


def _containers(children):
    return st.one_of(
        st.lists(children, max_size=4),
        st.tuples(children, children),
        st.dictionaries(_keys, children, max_size=4),
        st.dictionaries(_keys, children, max_size=4).map(OrderedDict),
        st.dictionaries(_keys, children, max_size=4).map(lambda d: defaultdict(list, d)),
        st.dictionaries(_keys, st.integers(), max_size=4).map(Counter),
        st.builds(deque, st.lists(children, max_size=4), st.one_of(st.none(), st.integers(4, 6))),
        st.builds(slice, children, children, children),
    )


@settings(max_examples=300, deadline=None)
@given(st.recursive(_base, _containers, max_leaves=12))
def test_hypothesis_roundtrip(value):
    assert same(rt(value, classes=[Color, Level, Perm, Point]), value)


def test_enum_without_classes_raises_unsafe_type_error():
    import enum
    from pysafe_pickle.exceptions import UnsafeTypeError

    class Shade(enum.Enum):
        DARK = 1

    blob = psp.dumps(Shade.DARK)
    with pytest.raises(UnsafeTypeError, match="pass it via classes="):
        psp.loads(blob)
