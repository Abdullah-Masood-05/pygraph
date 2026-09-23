import io
import sys
from dataclasses import dataclass

import pygraph
import pytest


class TestVersion:
    def test_version(self):
        assert pygraph.__version__ == "1.2.0"

    def test_imports(self):
        assert hasattr(pygraph, "dumps")
        assert hasattr(pygraph, "loads")
        assert hasattr(pygraph, "dump")
        assert hasattr(pygraph, "load")


class TestPrimitives:
    def test_none(self):
        data = pygraph.dumps(None)
        assert pygraph.loads(data) is None

    def test_bool_true(self):
        data = pygraph.dumps(True)
        assert pygraph.loads(data) is True

    def test_bool_false(self):
        data = pygraph.dumps(False)
        assert pygraph.loads(data) is False

    def test_int_positive(self):
        data = pygraph.dumps(42)
        assert pygraph.loads(data) == 42

    def test_int_negative(self):
        data = pygraph.dumps(-42)
        assert pygraph.loads(data) == -42

    def test_int_zero(self):
        data = pygraph.dumps(0)
        assert pygraph.loads(data) == 0

    def test_int_large(self):
        data = pygraph.dumps(2**62)
        assert pygraph.loads(data) == 2**62

    def test_int_large_negative(self):
        data = pygraph.dumps(-(2**62))
        assert pygraph.loads(data) == -(2**62)

    def test_float_positive(self):
        data = pygraph.dumps(3.14)
        assert pygraph.loads(data) == pytest.approx(3.14)

    def test_float_negative(self):
        data = pygraph.dumps(-2.5)
        assert pygraph.loads(data) == pytest.approx(-2.5)

    def test_float_zero(self):
        data = pygraph.dumps(0.0)
        assert pygraph.loads(data) == 0.0

    def test_float_nan(self):
        data = pygraph.dumps(float("nan"))
        result = pygraph.loads(data)
        assert result != result  # NaN != NaN

    def test_float_inf(self):
        data = pygraph.dumps(float("inf"))
        assert pygraph.loads(data) == float("inf")

    def test_float_neg_inf(self):
        data = pygraph.dumps(float("-inf"))
        assert pygraph.loads(data) == float("-inf")

    def test_string_empty(self):
        data = pygraph.dumps("")
        assert pygraph.loads(data) == ""

    def test_string_simple(self):
        data = pygraph.dumps("hello")
        assert pygraph.loads(data) == "hello"

    def test_string_unicode(self):
        data = pygraph.dumps("hello world")
        assert pygraph.loads(data) == "hello world"

    def test_bytes_empty(self):
        data = pygraph.dumps(b"")
        assert pygraph.loads(data) == b""

    def test_bytes_simple(self):
        data = pygraph.dumps(b"\x00\x01\x02")
        assert pygraph.loads(data) == b"\x00\x01\x02"


class TestContainers:
    def test_list_empty(self):
        data = pygraph.dumps([])
        assert pygraph.loads(data) == []

    def test_list_primitives(self):
        data = pygraph.dumps([1, 2, 3])
        assert pygraph.loads(data) == [1, 2, 3]

    def test_list_mixed(self):
        data = pygraph.dumps([1, "two", 3.0, None, True])
        assert pygraph.loads(data) == [1, "two", 3.0, None, True]

    def test_list_nested(self):
        data = pygraph.dumps([[1, 2], [3, 4]])
        assert pygraph.loads(data) == [[1, 2], [3, 4]]

    def test_tuple_empty(self):
        data = pygraph.dumps(())
        assert pygraph.loads(data) == ()

    def test_tuple_primitives(self):
        data = pygraph.dumps((1, 2, 3))
        assert pygraph.loads(data) == (1, 2, 3)

    def test_dict_empty(self):
        data = pygraph.dumps({})
        assert pygraph.loads(data) == {}

    def test_dict_simple(self):
        data = pygraph.dumps({"a": 1, "b": 2})
        assert pygraph.loads(data) == {"a": 1, "b": 2}

    def test_dict_nested(self):
        data = pygraph.dumps({"a": {"b": {"c": 1}}})
        assert pygraph.loads(data) == {"a": {"b": {"c": 1}}}

    def test_set_empty(self):
        data = pygraph.dumps(set())
        assert pygraph.loads(data) == set()

    def test_set_values(self):
        data = pygraph.dumps({1, 2, 3})
        assert pygraph.loads(data) == {1, 2, 3}

    def test_frozenset_empty(self):
        data = pygraph.dumps(frozenset())
        assert pygraph.loads(data) == frozenset()

    def test_frozenset_values(self):
        data = pygraph.dumps(frozenset({1, 2, 3}))
        assert pygraph.loads(data) == frozenset({1, 2, 3})


class TestNestedStructures:
    def test_deeply_nested_list(self):
        obj = [[[[[42]]]]]
        data = pygraph.dumps(obj)
        assert pygraph.loads(data) == obj

    def test_mixed_containers(self):
        obj = {"list": [1, 2], "tuple": (3, 4), "set": {5}}
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result["list"] == [1, 2]
        assert result["tuple"] == (3, 4)
        assert result["set"] == {5}

    def test_complex_structure(self):
        obj = {
            "users": [
                {"name": "Alice", "scores": [95, 87, 92]},
                {"name": "Bob", "scores": [78, 85, 90]},
            ],
            "metadata": {"version": 2, "tags": ["alpha", "beta"]},
        }
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result == obj


class TestCycles:
    def test_list_self_reference(self):
        obj = []
        obj.append(obj)
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result[0] is result

    def test_dict_self_reference(self):
        obj = {}
        obj["self"] = obj
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result["self"] is result

    def test_shared_reference(self):
        shared = [1, 2, 3]
        obj = [shared, shared]
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result[0] == result[1]
        assert result[0] is result[1]

    def test_indirect_cycle(self):
        a = {"name": "a"}
        b = {"name": "b"}
        a["ref"] = b
        b["ref"] = a
        data = pygraph.dumps(a)
        result = pygraph.loads(data)
        assert result["name"] == "a"
        assert result["ref"]["name"] == "b"
        assert result["ref"]["ref"] is result


class TestDumpLoad:
    def test_dump_to_file(self):
        obj = {"key": "value"}
        f = io.BytesIO()
        pygraph.dump(obj, f)
        f.seek(0)
        result = pygraph.load(f)
        assert result == obj

    def test_dump_load_roundtrip(self):
        obj = [1, "two", 3.0, None, {"a": [True]}]
        f = io.BytesIO()
        pygraph.dump(obj, f)
        f.seek(0)
        result = pygraph.load(f)
        assert result == obj


class TestDataclasses:
    def test_simple_dataclass(self):
        @dataclass
        class Point:
            x: int
            y: int

        obj = Point(10, 20)
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result.x == 10
        assert result.y == 20
        assert repr(result) == "Point(x=10, y=20)"

    def test_nested_dataclass(self):
        @dataclass
        class Inner:
            value: int

        @dataclass
        class Outer:
            inner: Inner
            name: str

        obj = Outer(inner=Inner(value=42), name="test")
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result.name == "test"
        assert result.inner.value == 42
        assert repr(result) == "Outer(inner=Inner(value=42), name='test')"

    def test_dataclass_with_containers(self):
        @dataclass
        class Config:
            name: str
            values: list
            mapping: dict

        obj = Config(name="cfg", values=[1, 2, 3], mapping={"a": 1})
        data = pygraph.dumps(obj)
        result = pygraph.loads(data)
        assert result.name == "cfg"
        assert result.values == [1, 2, 3]
        assert result.mapping == {"a": 1}


class TestAllowlist:
    def test_unsupported_type_raises(self):
        with pytest.raises(TypeError):
            pygraph.dumps(lambda x: x)

    def test_allowlist_restricts(self):
        @dataclass
        class Allowed:
            x: int

        obj = Allowed(x=1)
        data = pygraph.dumps(obj)

        with pytest.raises(TypeError, match="not in the allowlist"):
            pygraph.loads(data, allowlist={"OtherClass"})
