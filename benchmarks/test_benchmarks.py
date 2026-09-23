"""Pysafe-pickle vs original pygraph vs standard pickle benchmark suite.

Usage:
    pytest benchmarks/ -v --benchmark-only
    pytest benchmarks/ -v --benchmark-compare
"""
import pickle
import pytest
import pysafe_pickle as psp
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

    def test_pysafe_pickle_dumps(self, data, benchmark):
        benchmark(psp.dumps, data)

    def test_original_pygraph_dumps(self, data, benchmark):
        benchmark(pygraph.dumps, data)


class TestLoads:
    def test_pickle_loads(self, data, benchmark):
        pickled = pickle.dumps(data, protocol=5)
        benchmark(pickle.loads, pickled)

    def test_pysafe_pickle_loads(self, data, benchmark):
        encoded = psp.dumps(data)
        benchmark(psp.loads, encoded)

    def test_original_pygraph_loads(self, data, benchmark):
        encoded = pygraph.dumps(data)
        benchmark(pygraph.loads, encoded)


class TestRoundtrip:
    def test_pickle_roundtrip(self, data, benchmark):
        def roundtrip():
            pickled = pickle.dumps(data, protocol=5)
            return pickle.loads(pickled)
        benchmark(roundtrip)

    def test_pysafe_pickle_roundtrip(self, data, benchmark):
        def roundtrip():
            encoded = psp.dumps(data)
            return psp.loads(encoded)
        benchmark(roundtrip)

    def test_original_pygraph_roundtrip(self, data, benchmark):
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
        psp_size = len(psp.dumps(data))
        pygraph_size = len(pygraph.dumps(data))

        ratio = psp_size / pickle_size if pickle_size > 0 else float("inf")
        print(f"\n{name}: pickle={pickle_size}B, pysafe_pickle={psp_size}B, original_pygraph={pygraph_size}B, ratio={ratio:.2f}x")

        # payload should be within 3x of pickle size
        assert ratio < 3.0, f"pysafe-pickle payload too large: {ratio:.2f}x pickle size"
        assert psp_size == pygraph_size, "pysafe-pickle and pygraph shim should produce identical payload size"
