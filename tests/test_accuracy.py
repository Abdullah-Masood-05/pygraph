"""Accuracy, fidelity and robustness tests for pysafe_pickle.

These complement ``test_roundtrip.py`` (basic happy paths) by checking exact
type/value fidelity, identity preservation, edge-case scalars, dataclass
corner cases, and malformed-payload handling.
"""
import enum
import io
import math
import struct
import sys
from collections import OrderedDict, defaultdict
from dataclasses import InitVar, dataclass, field
from typing import ClassVar

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

import pysafe_pickle as psp

# Built with chr() so editors/tools never "helpfully" merge them into one astral char.
HIGH_SURROGATE = chr(0xD83D)
LOW_SURROGATE = chr(0xDE00)
SURROGATE_PAIR_AS_TWO = HIGH_SURROGATE + LOW_SURROGATE  # len 2, NOT U+1F600
EMOJI = chr(0x1F600)  # len 1


def roundtrip(obj, **kwargs):
    return psp.loads(psp.dumps(obj), **kwargs)


def float_bits(x):
    return struct.pack("<d", x)


def assert_same_structure(expected, actual, path="root"):
    """Recursively compare values *and* exact types.

    Plain ``==`` hides bugs such as ``1 == True == 1.0`` or ``0.0 == -0.0``,
    so this checks ``type(...) is`` at every level and compares floats
    bit-for-bit.
    """
    assert type(actual) is type(expected), (
        f"{path}: type {type(actual).__name__} != {type(expected).__name__}"
    )
    if isinstance(expected, float):
        if math.isnan(expected):
            assert math.isnan(actual), f"{path}: expected nan, got {actual!r}"
        else:
            assert float_bits(actual) == float_bits(expected), (
                f"{path}: {actual!r} != {expected!r} (bitwise)"
            )
    elif isinstance(expected, complex):
        assert_same_structure(expected.real, actual.real, f"{path}.real")
        assert_same_structure(expected.imag, actual.imag, f"{path}.imag")
    elif isinstance(expected, (list, tuple)):
        assert len(actual) == len(expected), f"{path}: length mismatch"
        for i, (e, a) in enumerate(zip(expected, actual)):
            assert_same_structure(e, a, f"{path}[{i}]")
    elif isinstance(expected, dict):
        assert len(actual) == len(expected), f"{path}: length mismatch"
        # Insertion order must be preserved.
        for i, ((ek, ev), (ak, av)) in enumerate(zip(expected.items(), actual.items())):
            assert_same_structure(ek, ak, f"{path}.keys()[{i}]")
            assert_same_structure(ev, av, f"{path}[{ek!r}]")
    elif isinstance(expected, (set, frozenset)):
        assert len(actual) == len(expected), f"{path}: length mismatch"
        actual_by_value = {a: a for a in actual}
        for e in expected:
            assert e in actual_by_value, f"{path}: missing element {e!r}"
            assert_same_structure(e, actual_by_value[e], f"{path}{{{e!r}}}")
    else:
        assert actual == expected, f"{path}: {actual!r} != {expected!r}"


def header_prefix():
    """Magic + format version + schema version + flags, taken from a real payload
    so hand-crafted payloads keep working if the header constants change."""
    return psp.dumps(None)[:11]


def crafted_payload(records, obj_count, strings=(), types_blob=b"", type_count=0):
    out = bytearray(header_prefix())
    out += struct.pack("<I", len(strings))
    for s in strings:
        encoded = s.encode("utf-8")
        out += struct.pack("<I", len(encoded)) + encoded
    out += struct.pack("<I", type_count) + types_blob
    out += struct.pack("<I", obj_count) + records
    return bytes(out)


# Wire tags (see src/format/mod.rs)
TAG_NONE = 0x01
TAG_INT = 0x04
TAG_STRING = 0x06
TAG_LIST = 0x08
TAG_TUPLE = 0x09
TAG_DICT = 0x0A
TAG_SET = 0x0B
TAG_FROZENSET = 0x0C
TAG_DATACLASS = 0x0D
TAG_REFERENCE = 0x0E


# --- Module-level dataclasses (unique names to avoid registry collisions) ---


@dataclass
class AccNode:
    value: int
    nxt: object = None


@dataclass
class AccHolder:
    items: list


@dataclass
class AccPair:
    left: object
    right: object


@dataclass
class AccWithClassVar:
    x: int
    counter: ClassVar[int] = 99


@dataclass
class AccWithInitFalse:
    x: int
    computed: int = field(init=False)

    def __post_init__(self):
        self.computed = self.x * 10


@dataclass
class AccWithFactory:
    tags: list = field(default_factory=list)
    meta: dict = field(default_factory=dict)


@dataclass(frozen=True)
class AccFrozen:
    a: int
    b: str


@dataclass
class AccEmpty:
    pass


if sys.version_info >= (3, 10):

    @dataclass(slots=True)
    class AccSlots:
        a: int
        b: list

    @dataclass(frozen=True, slots=True)
    class AccFrozenSlots:
        a: int
        b: tuple


@dataclass
class AccInitVarDefault:
    x: int
    seed: InitVar[int] = 0

    def __post_init__(self, seed):
        self.x += seed


@dataclass
class AccInitVarRequired:
    x: int
    seed: InitVar[int]

    def __post_init__(self, seed):
        self.x += seed


@dataclass
class AccSelfField:
    self: int


# --- Arbitrary-precision ints ---


class TestBigInts:
    @pytest.mark.parametrize(
        "value",
        [
            2**63 - 1,
            -(2**63),
            2**63,
            -(2**63) - 1,
            2**64,
            -(2**64),
            2**100,
            -(2**70),
            10**1000,
            -(10**1000),
        ],
        ids=[
            "i64_max",
            "i64_min",
            "i64_max_plus_1",
            "i64_min_minus_1",
            "2pow64",
            "neg_2pow64",
            "2pow100",
            "neg_2pow70",
            "10pow1000",
            "neg_10pow1000",
        ],
    )
    def test_int_roundtrip_exact(self, value):
        result = roundtrip(value)
        assert type(result) is int
        assert result == value

    def test_big_ints_in_containers(self):
        obj = {"big": [2**200, -(2**65), (2**64, -1)], 2**70: "key", (2**80,): {2**90}}
        result = roundtrip(obj)
        assert_same_structure(obj, result)

    def test_mixed_small_and_big(self):
        obj = [0, 1, -1, 2**63 - 1, 2**63, -(2**63), -(2**63) - 1]
        assert_same_structure(obj, roundtrip(obj))


# --- bytearray ---


class TestBytearray:
    def test_roundtrip_type_preserved(self):
        result = roundtrip(bytearray(b"\x00abc\xff"))
        assert type(result) is bytearray
        assert result == bytearray(b"\x00abc\xff")

    def test_empty(self):
        result = roundtrip(bytearray())
        assert type(result) is bytearray
        assert result == bytearray()

    def test_shared_identity(self):
        b = bytearray(b"x")
        result = roundtrip([b, b])
        assert result[0] is result[1]
        result[0].append(ord("y"))
        assert result[1] == bytearray(b"xy")

    def test_bytes_and_bytearray_distinct(self):
        obj = [b"same", bytearray(b"same")]
        result = roundtrip(obj)
        assert type(result[0]) is bytes
        assert type(result[1]) is bytearray

    def test_large(self):
        data = bytearray(range(256)) * 4096
        result = roundtrip(data)
        assert type(result) is bytearray
        assert result == data


# --- complex ---


class TestComplex:
    @pytest.mark.parametrize(
        "value",
        [0j, 1 + 2j, -1.5 - 0.25j, complex(1e308, -1e-308), complex(0.0, -0.0), complex(-0.0, 0.0)],
        ids=["zero", "simple", "negative", "extreme", "neg_zero_imag", "neg_zero_real"],
    )
    def test_roundtrip(self, value):
        result = roundtrip(value)
        assert type(result) is complex
        assert_same_structure(value, result)

    def test_inf_and_negative_zero(self):
        value = complex(float("inf"), -0.0)
        result = roundtrip(value)
        assert type(result) is complex
        assert result.real == float("inf")
        assert result.imag == 0.0
        assert math.copysign(1.0, result.imag) == -1.0

    def test_nan_component(self):
        result = roundtrip(complex(float("nan"), 1.0))
        assert type(result) is complex
        assert math.isnan(result.real)
        assert result.imag == 1.0

    def test_complex_in_containers(self):
        obj = {"c": [1j, (2 + 3j,)], 4j: frozenset({5j})}
        assert_same_structure(obj, roundtrip(obj))


# --- Strings ---


class TestStrings:
    @pytest.mark.parametrize(
        "value",
        [
            chr(0xD800),
            "a" + chr(0xDFFF) + "b",
            chr(0xDC80) + chr(0xD800),
            HIGH_SURROGATE,
            "x" + SURROGATE_PAIR_AS_TWO + "y",
        ],
        ids=["lone_high", "lone_low_inner", "reversed_pair", "high_only", "pair_as_two_surrogates"],
    )
    def test_lone_surrogates(self, value):
        result = roundtrip(value)
        assert type(result) is str
        assert result == value
        assert len(result) == len(value)

    def test_surrogate_as_dict_key(self):
        obj = {chr(0xD800): chr(0xDFFF), "normal": chr(0xD801) + "x"}
        assert_same_structure(obj, roundtrip(obj))

    def test_surrogate_and_real_char_not_conflated(self):
        # A real astral char and its surrogate-pair spelling are different strings.
        obj = [EMOJI, SURROGATE_PAIR_AS_TWO]
        result = roundtrip(obj)
        assert result[0] == EMOJI and len(result[0]) == 1
        assert result[1] == SURROGATE_PAIR_AS_TWO and len(result[1]) == 2

    @pytest.mark.parametrize(
        "value",
        ["\x00", "a\x00b", chr(0x10FFFF), chr(0xE9) + chr(0x4E2D) + EMOJI, "\r\n\t", "x" * 70000],
        ids=["nul", "inner_nul", "max_codepoint", "multibyte", "whitespace", "long"],
    )
    def test_valid_unicode(self, value):
        result = roundtrip(value)
        assert result == value


# --- Floats ---


class TestFloatEdgeCases:
    @pytest.mark.parametrize(
        "value",
        [
            -0.0,
            0.0,
            5e-324,
            -5e-324,
            2.2250738585072014e-308,
            1.7976931348623157e308,
            -1.7976931348623157e308,
            float("inf"),
            float("-inf"),
            0.1,
            1 / 3,
        ],
    )
    def test_exact_bits(self, value):
        result = roundtrip(value)
        assert type(result) is float
        assert float_bits(result) == float_bits(value)

    def test_negative_zero_sign(self):
        result = roundtrip(-0.0)
        assert math.copysign(1.0, result) == -1.0

    def test_nan_in_container(self):
        result = roundtrip([float("nan"), {"k": float("nan")}])
        assert math.isnan(result[0])
        assert math.isnan(result[1]["k"])


# --- Type fidelity ---


class TestTypeFidelity:
    def test_bool_int_float_not_conflated(self):
        obj = [1, True, 1.0, 0, False, 0.0, None]
        assert_same_structure(obj, roundtrip(obj))

    def test_list_vs_tuple(self):
        obj = ([1, (2, [3])], [(), []])
        assert_same_structure(obj, roundtrip(obj))

    def test_set_vs_frozenset(self):
        obj = [{1, 2}, frozenset({1, 2}), {frozenset({3})}]
        assert_same_structure(obj, roundtrip(obj))

    def test_dict_insertion_order(self):
        obj = {k: i for i, k in enumerate(["z", "a", "m", 3, (1,), "b"])}
        result = roundtrip(obj)
        assert list(result.keys()) == list(obj.keys())

    def test_mixed_key_types(self):
        obj = {1: "int", "1": "str", 1.5: "float", (1,): "tuple", None: "none", b"1": "bytes"}
        assert_same_structure(obj, roundtrip(obj))

    @pytest.mark.parametrize(
        "factory",
        [
            lambda: enum.IntEnum("Color", "RED GREEN").RED,
            lambda: type("MyStr", (str,), {})("x"),
            lambda: type("MyInt", (int,), {})(1),
            lambda: type("MyList", (list,), {})([1]),
            lambda: OrderedDict(a=1),
            lambda: defaultdict(list),
            lambda: object(),
            lambda: int,
        ],
        ids=["IntEnum", "str_subclass", "int_subclass", "list_subclass", "OrderedDict", "defaultdict", "object", "type"],
    )
    def test_subclasses_and_foreign_types_rejected(self, factory):
        with pytest.raises(TypeError, match="Unsupported type"):
            psp.dumps(factory())

    def test_unsupported_nested_deep_inside(self):
        with pytest.raises(TypeError, match="Unsupported type"):
            psp.dumps({"a": [1, (2, {"b": object()})]})


# --- Shared references / identity ---


class TestSharedReferences:
    def test_shared_str(self):
        s = "shared-" + str(12345)
        result = roundtrip([s, s, {"k": s}])
        assert result == [s, s, {"k": s}]

    def test_shared_list_identity_across_containers(self):
        shared = [1, 2]
        obj = {"a": shared, "b": (shared,), "c": [[shared]]}
        result = roundtrip(obj)
        assert result["a"] is result["b"][0]
        assert result["a"] is result["c"][0][0]
        result["a"].append(3)
        assert result["c"][0][0] == [1, 2, 3]

    def test_dict_containing_itself_via_list(self):
        d = {"name": "root"}
        d["children"] = [d, {"parent": d}]
        result = roundtrip(d)
        assert result["children"][0] is result
        assert result["children"][1]["parent"] is result

    def test_set_inside_list_referenced_twice(self):
        s = {1, 2, 3}
        result = roundtrip([s, s])
        assert type(result[0]) is set
        assert result[0] is result[1]
        assert result[0] == {1, 2, 3}

    def test_shared_tuple_with_mutable_inside(self):
        inner = [1]
        t = (inner, "x")
        result = roundtrip([t, t, inner])
        assert result[0] == result[1] == ([1], "x")
        assert result[0][0] is result[1][0]
        assert result[0][0] is result[2]

    def test_shared_frozenset_and_bytes(self):
        fs = frozenset({1, 2})
        b = b"payload"
        result = roundtrip([fs, fs, b, b])
        assert result[0] == result[1] == fs
        assert result[2] == result[3] == b

    def test_list_cycle_through_tuple_in_mutable(self):
        # list -> tuple -> list: the cycle passes through a tuple but is anchored
        # by a mutable list, so it is representable; must roundtrip or raise
        # ValueError, never crash.
        lst = []
        lst.append((lst,))
        try:
            result = roundtrip(lst)
        except ValueError:
            return
        assert result[0][0] is result

    def test_distinct_equal_lists_stay_distinct(self):
        obj = [[1], [1]]
        result = roundtrip(obj)
        assert result[0] == result[1]
        assert result[0] is not result[1]


# --- Dataclass cycles ---


class TestDataclassCycles:
    def test_self_cycle(self):
        n = AccNode(1)
        n.nxt = n
        result = roundtrip(n)
        assert type(result).__name__ == "AccNode"
        assert result.value == 1
        assert result.nxt is result

    def test_mutual_cycle(self):
        a = AccPair("a", None)
        b = AccPair("b", a)
        a.right = b
        result = roundtrip(a)
        assert result.left == "a"
        assert result.right.left == "b"
        assert result.right.right is result

    def test_cycle_through_list(self):
        h = AccHolder([])
        h.items.append(h)
        h.items.append(1)
        result = roundtrip(h)
        assert result.items[0] is result
        assert result.items[1] == 1

    def test_cycle_through_dict(self):
        h = AccHolder([])
        h.items.append({"owner": h})
        result = roundtrip(h)
        assert result.items[0]["owner"] is result

    def test_root_is_list_containing_cyclic_dataclass(self):
        h = AccHolder([])
        h.items.append(h)
        result = roundtrip([h, h])
        assert result[0] is result[1]
        assert result[0].items[0] is result[0]

    def test_shared_dataclass_identity(self):
        n = AccNode(5)
        result = roundtrip([n, {"n": n}, AccPair(n, n)])
        assert result[0] is result[1]["n"]
        assert result[2].left is result[0]
        assert result[2].right is result[0]

    def test_linked_list_ring(self):
        nodes = [AccNode(i) for i in range(50)]
        for i, n in enumerate(nodes):
            n.nxt = nodes[(i + 1) % len(nodes)]
        result = roundtrip(nodes[0])
        cur = result
        for i in range(50):
            assert cur.value == i
            cur = cur.nxt
        assert cur is result


# --- Dataclass field kinds ---


class TestDataclassFieldKinds:
    def test_classvar_not_serialized(self):
        result = roundtrip(AccWithClassVar(7))
        assert result.x == 7
        assert "counter" not in getattr(result, "__dict__", {})
        assert "counter" not in type(result).__dataclass_fields__
        assert list(type(result).__dataclass_fields__) == ["x"]

    def test_init_false_field_preserved(self):
        obj = AccWithInitFalse(4)
        obj.computed = 123  # differs from what __post_init__ would compute
        result = roundtrip(obj)
        assert result.x == 4
        assert result.computed == 123

    def test_default_factory_fields(self):
        obj = AccWithFactory()
        obj.tags.append("t")
        result = roundtrip(obj)
        assert result.tags == ["t"]
        assert result.meta == {}

    def test_frozen(self):
        result = roundtrip(AccFrozen(1, "b"))
        assert type(result).__name__ == "AccFrozen"
        assert (result.a, result.b) == (1, "b")

    def test_frozen_as_dict_key_and_set_member(self):
        f = AccFrozen(1, "k")
        result = roundtrip({"d": {f: 1}, "s": {f}})
        (key,) = result["d"].keys()
        (member,) = result["s"]
        assert (key.a, key.b) == (1, "k")
        assert (member.a, member.b) == (1, "k")

    @pytest.mark.skipif(sys.version_info < (3, 10), reason="slots=True requires 3.10")
    def test_slots(self):
        result = roundtrip(AccSlots(1, [2]))
        assert type(result).__name__ == "AccSlots"
        assert (result.a, result.b) == (1, [2])

    @pytest.mark.skipif(sys.version_info < (3, 10), reason="slots=True requires 3.10")
    def test_frozen_slots(self):
        result = roundtrip(AccFrozenSlots(1, (2, 3)))
        assert (result.a, result.b) == (1, (2, 3))

    def test_empty_dataclass(self):
        result = roundtrip(AccEmpty())
        assert type(result).__name__ == "AccEmpty"

    def test_field_values_keep_exact_types(self):
        obj = AccPair(True, 1.0)
        result = roundtrip(obj)
        assert result.left is True
        assert type(result.right) is float

    def test_allowlist_accepts_listed_nested_types(self):
        obj = AccPair(AccNode(1), AccFrozen(2, "x"))
        result = roundtrip(obj, allowlist={"AccPair", "AccNode", "AccFrozen"})
        assert result.left.value == 1
        assert result.right.b == "x"

    def test_allowlist_rejects_unlisted_nested_type(self):
        obj = AccPair(AccNode(1), None)
        with pytest.raises(TypeError, match="not in the allowlist"):
            roundtrip(obj, allowlist={"AccPair"})


class TestDataclassBugsBeyondSpec:
    """Genuine bugs found while writing these tests; not part of the current spec.

    Marked xfail (non-strict) so they document the issue without blocking the suite.
    """

    @pytest.mark.xfail(reason="InitVar pseudo-field is serialized as instance state", strict=False)
    def test_initvar_with_default_not_serialized(self):
        result = roundtrip(AccInitVarDefault(1, seed=5))
        assert result.x == 6
        assert "seed" not in type(result).__dataclass_fields__

    @pytest.mark.xfail(reason="dumps getattr()s the InitVar name -> AttributeError", strict=False)
    def test_initvar_without_default_serializes(self):
        result = roundtrip(AccInitVarRequired(1, 5))
        assert result.x == 6

    @pytest.mark.xfail(reason="make_class __init__(self, **kwargs) clashes with a field named 'self'", strict=False)
    def test_field_named_self(self):
        result = roundtrip(AccSelfField(3))
        assert result.self == 3

    @pytest.mark.xfail(
        reason="TypeRegistry dedupes by __name__: same-named dataclasses with different fields are silently corrupted",
        strict=False,
    )
    def test_same_name_different_fields_in_one_payload(self):
        def make_a():
            @dataclass
            class Dup:
                x: int
                y: int

            return Dup

        def make_b():
            @dataclass
            class Dup:
                a: str
                b: str
                c: str

            return Dup

        A, B = make_a(), make_b()
        result = roundtrip([A(1, 2), B("p", "q", "r")])
        assert (result[0].x, result[0].y) == (1, 2)
        assert (result[1].a, result[1].b, result[1].c) == ("p", "q", "r")


# --- Input types for loads/load ---


class TestLoadInputs:
    OBJ = {"k": [1, 2.5, "s", b"b", None, (True,)]}

    def test_bytes(self):
        assert psp.loads(psp.dumps(self.OBJ)) == self.OBJ

    def test_bytearray(self):
        assert psp.loads(bytearray(psp.dumps(self.OBJ))) == self.OBJ

    def test_memoryview(self):
        assert psp.loads(memoryview(psp.dumps(self.OBJ))) == self.OBJ

    def test_memoryview_slice_with_offset(self):
        payload = psp.dumps(self.OBJ)
        buf = b"JUNK" + payload + b"TAIL"
        view = memoryview(buf)[4 : 4 + len(payload)]
        assert psp.loads(view) == self.OBJ

    def test_load_from_bytesio(self):
        f = io.BytesIO(psp.dumps(self.OBJ))
        assert psp.load(f) == self.OBJ

    def test_load_with_allowlist(self):
        f = io.BytesIO(psp.dumps(AccNode(1)))
        result = psp.load(f, allowlist={"AccNode"})
        assert result.value == 1

    def test_dump_then_load_separate_files(self):
        buf1, buf2 = io.BytesIO(), io.BytesIO()
        psp.dump([1], buf1)
        psp.dump({"two": 2}, buf2)
        buf1.seek(0)
        buf2.seek(0)
        assert psp.load(buf1) == [1]
        assert psp.load(buf2) == {"two": 2}


# --- Determinism / API errors ---


class TestDeterminism:
    @pytest.mark.parametrize(
        "factory",
        [
            lambda: {"a": [1, 2.5, "x", b"y", None], "b": (True, False), "c": {1, 2, 3}},
            lambda: AccPair(AccNode(1), [AccFrozen(2, "z")]),
            lambda: [frozenset({"a", "b", "c"}), {"k": {"nested": [1]}}],
        ],
        ids=["containers", "dataclasses", "sets"],
    )
    def test_same_object_same_bytes(self, factory):
        obj = factory()
        assert psp.dumps(obj) == psp.dumps(obj)

    def test_cyclic_object_same_bytes(self):
        d = {}
        d["self"] = d
        assert psp.dumps(d) == psp.dumps(d)

    def test_equal_fresh_objects_same_bytes(self):
        def build():
            return {"name": "n", "vals": [1, 2, 3], "nested": {"t": (1, "two")}}

        assert psp.dumps(build()) == psp.dumps(build())

    def test_new_types_same_bytes(self):
        obj = [2**100, bytearray(b"x"), 1 + 2j, chr(0xD800)]
        assert psp.dumps(obj) == psp.dumps(obj)


class TestApiErrors:
    @pytest.mark.parametrize("protocol", [0, 1, 2, 3, 4, 6])
    def test_unsupported_protocol(self, protocol):
        with pytest.raises(ValueError):
            psp.dumps([1], protocol=protocol)

    def test_protocol_5_accepted(self):
        assert psp.loads(psp.dumps([1], protocol=5)) == [1]

    def test_hmac_key_not_implemented(self):
        with pytest.raises(NotImplementedError):
            psp.dumps([1], hmac_key=b"secret")


# --- Malformed / hostile payloads ---


def _rich_legacy_payload():
    shared = [1, 2]
    obj = {
        "ints": [0, -1, 2**40, -(2**62)],
        "floats": [0.5, -0.0, float("inf")],
        "str": "h" + chr(0xE9) + "llo",
        "bytes": b"\x00\xff",
        "tuple": (None, True, False),
        "set": {1, 2},
        "frozenset": frozenset({"a"}),
        "shared": [shared, shared],
        "dc": AccPair(AccNode(1), AccFrozen(2, "x")),
    }
    obj["cycle"] = obj
    return psp.dumps(obj)


def _rich_new_types_payload():
    b = bytearray(b"ba")
    n = AccNode(1)
    n.nxt = n
    return psp.dumps([2**100, -(10**30), 1 + 2j, b, b, chr(0xD800) + "x", n])


class TestMalformedPayloads:
    @pytest.mark.parametrize(
        "make_payload",
        [_rich_legacy_payload, _rich_new_types_payload],
        ids=["legacy_types", "new_types"],
    )
    def test_every_truncated_prefix_raises_value_error(self, make_payload):
        payload = make_payload()
        for i in range(len(payload)):
            with pytest.raises(ValueError):
                psp.loads(payload[:i])

    @pytest.mark.parametrize(
        "data",
        [b"", b"P", b"PSP", b"XXXX" + b"\x00" * 40, b"\x80\x05\x95" + b"\x00" * 40, b"pspk" + b"\x00" * 40],
        ids=["empty", "one_byte", "short_magic", "garbage_magic", "pickle_magic", "lowercase_magic"],
    )
    def test_bad_magic_or_too_short(self, data):
        with pytest.raises(ValueError):
            psp.loads(data)

    def test_garbage_magic_on_valid_body(self):
        payload = bytearray(psp.dumps([1, 2, 3]))
        payload[0:4] = b"EVIL"
        with pytest.raises(ValueError):
            psp.loads(bytes(payload))

    def test_valid_crafted_payload_sanity(self):
        # Guards the hand-crafted format helper: a well-formed [None] loads fine.
        records = bytes([TAG_LIST]) + struct.pack("<II", 1, 1) + bytes([TAG_NONE])
        assert psp.loads(crafted_payload(records, 2)) == [None]

    @pytest.mark.parametrize(
        "records,count",
        [
            (bytes([TAG_LIST]) + struct.pack("<II", 1, 7), 1),
            (bytes([TAG_LIST]) + struct.pack("<II", 1, 0xFFFFFFFF), 1),
            (bytes([TAG_TUPLE]) + struct.pack("<II", 1, 2), 1),
            (bytes([TAG_SET]) + struct.pack("<II", 1, 3), 1),
            (bytes([TAG_FROZENSET]) + struct.pack("<II", 1, 3), 1),
            (bytes([TAG_DICT]) + struct.pack("<III", 1, 9, 1) + bytes([TAG_NONE]), 2),
            (bytes([TAG_DICT]) + struct.pack("<III", 1, 1, 9) + bytes([TAG_NONE]), 2),
            (bytes([TAG_REFERENCE]) + struct.pack("<I", 5), 1),
            (bytes([TAG_LIST]) + struct.pack("<II", 1, 1) + bytes([TAG_REFERENCE]) + struct.pack("<I", 99), 2),
        ],
        ids=[
            "list_child",
            "list_child_u32max",
            "tuple_child",
            "set_child",
            "frozenset_child",
            "dict_key",
            "dict_value",
            "reference_root",
            "reference_child",
        ],
    )
    def test_child_id_out_of_range(self, records, count):
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(records, count))

    def test_string_index_out_of_range(self):
        records = bytes([TAG_STRING]) + struct.pack("<I", 3)
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(records, 1, strings=["only"]))

    def test_dataclass_type_id_out_of_range(self):
        records = bytes([TAG_DATACLASS]) + struct.pack("<HH", 4, 0)
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(records, 1))

    def test_zero_records(self):
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(b"", 0))

    @pytest.mark.parametrize("tag", [0x00, 0x7F, 0xFF])
    def test_unknown_tag(self, tag):
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(bytes([tag]), 1))

    def test_invalid_utf8_in_string_table(self):
        out = bytearray(header_prefix())
        out += struct.pack("<I", 1) + struct.pack("<I", 2) + b"\xc3\x28"
        out += struct.pack("<I", 0)
        out += struct.pack("<I", 1) + bytes([TAG_STRING]) + struct.pack("<I", 0)
        with pytest.raises(ValueError):
            psp.loads(bytes(out))

    def test_overlong_int_varint(self):
        # 11 continuation bytes: more than any valid encoding of a 64-bit varint.
        records = bytes([TAG_INT]) + b"\xff" * 11 + b"\x01"
        with pytest.raises(ValueError):
            psp.loads(crafted_payload(records, 1))

    def test_single_byte_corruption_never_crashes(self):
        """Flipping any byte must yield a value or a regular Python exception,
        never a Rust panic (PanicException is a BaseException) or a hang."""
        payload = _rich_legacy_payload()
        for i in range(len(payload)):
            for mask in (0x01, 0x80, 0xFF):
                corrupted = bytearray(payload)
                corrupted[i] ^= mask
                try:
                    psp.loads(bytes(corrupted))
                except (ValueError, TypeError, RecursionError, AttributeError):
                    pass


# --- Property-based roundtrip ---


def _text(surrogates):
    if not surrogates:
        return st.text()
    # Default character strategies almost never produce Cs (surrogates); mix them
    # in explicitly, plus a branch guaranteeing at least one surrogate.
    any_char = st.one_of(st.characters(exclude_categories=()), st.characters(categories=["Cs"]))
    return st.one_of(
        st.text(alphabet=any_char),
        st.builds(
            lambda pre, s, post: pre + s + post,
            st.text(alphabet=any_char, max_size=3),
            st.characters(categories=["Cs"]),
            st.text(alphabet=any_char, max_size=3),
        ),
    )


def _values(full):
    ints = st.integers() if full else st.integers(min_value=-(2**63), max_value=2**63 - 1)
    text = _text(surrogates=full)
    scalars = [
        st.none(),
        st.booleans(),
        ints,
        st.floats(allow_nan=False),
        text,
        st.binary(),
    ]
    if full:
        scalars += [
            st.binary().map(bytearray),
            st.complex_numbers(allow_nan=False),
        ]
    keys = st.recursive(
        st.one_of(text, ints),
        lambda inner: st.tuples(inner, inner) | st.lists(inner, max_size=3).map(tuple),
        max_leaves=5,
    )
    return st.recursive(
        st.one_of(*scalars),
        lambda children: st.one_of(
            st.lists(children, max_size=6),
            st.lists(children, max_size=6).map(tuple),
            st.dictionaries(keys, children, max_size=6),
            st.frozensets(keys, max_size=6),
        ),
        max_leaves=30,
    )


_PROPERTY_SETTINGS = settings(
    max_examples=200,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow],
)


class TestPropertyRoundtrip:
    @_PROPERTY_SETTINGS
    @given(_values(full=True))
    def test_roundtrip_full_spec(self, obj):
        result = roundtrip(obj)
        assert result == obj
        assert_same_structure(obj, result)

    @_PROPERTY_SETTINGS
    @given(_values(full=False))
    def test_roundtrip_i64_no_surrogates(self, obj):
        result = roundtrip(obj)
        assert result == obj
        assert_same_structure(obj, result)

    @_PROPERTY_SETTINGS
    @given(_values(full=False))
    def test_dumps_deterministic(self, obj):
        assert psp.dumps(obj) == psp.dumps(obj)
