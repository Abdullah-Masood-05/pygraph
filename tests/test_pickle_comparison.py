"""Tests comparing pygraph against pickle for supported types."""
import io
import pickle
from dataclasses import dataclass

import pygraph
import pytest


class TestPrimitivesComparison:
    """Compare pygraph and pickle round-trips for primitives."""

    @pytest.mark.parametrize("obj", [None, True, False, 0, 42, -42, 2**62, 3.14, float("nan"), float("inf")])
    def test_primitives(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        pk_result = pickle.loads(pk_data)

        if isinstance(obj, float) and obj != obj:  # NaN
            assert pg_result != pg_result
            assert pk_result != pk_result
        else:
            assert pg_result == pk_result

    @pytest.mark.parametrize("obj", ["", "hello", "hello world", "\x00\x01\x02"])
    def test_strings(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    @pytest.mark.parametrize("obj", [b"", b"\x00\x01\x02", b"hello"])
    def test_bytes(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)


class TestContainersComparison:
    """Compare pygraph and pickle round-trips for containers."""

    @pytest.mark.parametrize("obj", [[], [1, 2, 3], [1, "two", 3.0, None], [[1, 2], [3, 4]]])
    def test_lists(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    @pytest.mark.parametrize("obj", [(), (1, 2, 3), (1, "two", 3.0)])
    def test_tuples(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    @pytest.mark.parametrize("obj", [{}, {"a": 1}, {"a": {"b": {"c": 1}}}])
    def test_dicts(self, obj):
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    def test_set(self):
        obj = {1, 2, 3}
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    def test_frozenset(self):
        obj = frozenset({1, 2, 3})
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)


class TestComplexStructuresComparison:
    """Compare pygraph and pickle for complex nested structures."""

    def test_deeply_nested(self):
        obj = [[[[[42]]]]]
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        assert pygraph.loads(pg_data) == pickle.loads(pk_data)

    def test_mixed_containers(self):
        obj = {"list": [1, 2], "tuple": (3, 4), "set": {5}}
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        pk_result = pickle.loads(pk_data)
        assert pg_result["list"] == pk_result["list"]
        assert pg_result["tuple"] == pk_result["tuple"]
        assert pg_result["set"] == pk_result["set"]


class TestCyclesComparison:
    """Compare pygraph and pickle for cyclic references."""

    def test_list_self_ref(self):
        obj = []
        obj.append(obj)
        pg_data = pygraph.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        assert pg_result[0] is pg_result

    def test_dict_self_ref(self):
        obj = {}
        obj["self"] = obj
        pg_data = pygraph.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        assert pg_result["self"] is pg_result

    def test_shared_ref(self):
        shared = [1, 2, 3]
        obj = [shared, shared]
        pg_data = pygraph.dumps(obj)
        pk_data = pickle.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        pk_result = pickle.loads(pk_data)
        assert pg_result[0] == pk_result[0]
        assert pg_result[1] == pk_result[1]
        assert pg_result[0] is pg_result[1]


class TestFileIOComparison:
    """Compare dump/load against pickle's dump/load."""

    def test_dump_load(self):
        obj = {"key": "value", "numbers": [1, 2, 3]}
        f = io.BytesIO()
        pygraph.dump(obj, f)
        f.seek(0)
        pg_result = pygraph.load(f)

        f2 = io.BytesIO()
        pickle.dump(obj, f2)
        f2.seek(0)
        pk_result = pickle.load(f2)

        assert pg_result == pk_result

    def test_pickler_unpickler(self):
        obj = [1, "two", 3.0, None, {"a": [True]}]
        f = io.BytesIO()
        pickler = pygraph.Pickler(f)
        pickler.dump(obj)
        f.seek(0)
        unpickler = pygraph.Unpickler(f)
        pg_result = unpickler.load()

        f2 = io.BytesIO()
        pickle.Pickler(f2).dump(obj)
        f2.seek(0)
        pk_result = pickle.Unpickler(f2).load()

        assert pg_result == pk_result


class TestDataclassesComparison:
    """Compare pygraph and pickle for dataclasses."""

    def test_simple_dataclass(self):
        @dataclass
        class Point:
            x: int
            y: int

        obj = Point(10, 20)
        pg_data = pygraph.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        assert pg_result.x == 10
        assert pg_result.y == 20

    def test_nested_dataclass(self):
        @dataclass
        class Inner:
            value: int

        @dataclass
        class Outer:
            inner: Inner
            name: str

        obj = Outer(inner=Inner(value=42), name="test")
        pg_data = pygraph.dumps(obj)
        pg_result = pygraph.loads(pg_data)
        assert pg_result.name == "test"
        assert pg_result.inner.value == 42


class TestSecurityAdvantage:
    """Test that pygraph rejects unsafe types that pickle allows."""

    def test_rejects_lambda(self):
        obj = lambda x: x
        with pytest.raises(TypeError, match="Unsupported type"):
            pygraph.dumps(obj)

    def test_rejects_exec(self):
        with pytest.raises(TypeError, match="Unsupported type"):
            pygraph.dumps(exec)

    def test_rejects_eval(self):
        with pytest.raises(TypeError, match="Unsupported type"):
            pygraph.dumps(eval)
