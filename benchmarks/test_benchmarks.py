"""Pygraph vs pickle benchmark suite.

Usage:
    pytest benchmarks/ -v --benchmark-only
    pytest benchmarks/ -v --benchmark-compare
"""
import pickle
import pytest
import pygraph
from benchmarks.bench_data import BENCHMARK_DATA


def _get_data(name):
    return BENCHMARK_DATA[name]()


# --- Dumps benchmarks ---

@pytest.fixture(params=list(BENCHMARK_DATA.keys()))
def data(request):
    return _get_data(request.param)


class TestDumps:
    def test_pickle_dumps(self, data, benchmark):
        benchmark(pickle.dumps, data, 5)

    def test_pygraph_dumps(self, data, benchmark):
        benchmark(pygraph.dumps, data)


class TestLoads:
    def test_pickle_loads(self, data, benchmark):
        pickled = pickle.dumps(data, protocol=5)
        benchmark(pickle.loads, pickled)

    def test_pygraph_loads(self, data, benchmark):
        encoded = pygraph.dumps(data)
        benchmark(pygraph.loads, encoded)


class TestRoundtrip:
    def test_pickle_roundtrip(self, data, benchmark):
        def roundtrip():
            pickled = pickle.dumps(data, protocol=5)
            return pickle.loads(pickled)
        benchmark(roundtrip)

    def test_pygraph_roundtrip(self, data, benchmark):
        def roundtrip():
            encoded = pygraph.dumps(data)
            return pygraph.loads(encoded)
        benchmark(roundtrip)


# --- Size comparison ---

class TestSize:
    @pytest.mark.parametrize("name", list(BENCHMARK_DATA.keys()))
    def test_size_comparison(self, name):
        data = _get_data(name)
        pickle_size = len(pickle.dumps(data, protocol=5))
        pygraph_size = len(pygraph.dumps(data))

        ratio = pygraph_size / pickle_size if pickle_size > 0 else float("inf")
        print(f"\n{name}: pickle={pickle_size}B, pygraph={pygraph_size}B, ratio={ratio:.2f}x")

        # pygraph should be within 3x of pickle size
        assert ratio < 3.0, f"pygraph payload too large: {ratio:.2f}x pickle size"
