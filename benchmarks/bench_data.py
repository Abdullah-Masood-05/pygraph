"""Benchmarks comparing pygraph vs pickle performance."""
import pickle
import pysafe_pickle as psp
from dataclasses import dataclass


@dataclass
class Point:
    x: float
    y: float
    z: float


@dataclass
class Address:
    street: str
    city: str
    zip_code: str


@dataclass
class Person:
    name: str
    age: int
    address: Address


# --- Test data generators ---

def make_primitives():
    return [None, True, False, 0, 42, -42, 3.14, "hello", b"\x00\x01\x02"]


def make_nested_list(depth=10):
    result = [1]
    for _ in range(depth):
        result = [result]
    return result


def make_wide_dict(n=1000):
    return {f"key_{i}": i for i in range(n)}


def make_dataclass():
    return Point(1.0, 2.0, 3.0)


def make_nested_dataclass():
    return Person("Alice", 30, Address("123 Main St", "Springfield", "12345"))


def make_complex_graph():
    shared = {"shared_data": [1, 2, 3, 4, 5]}
    return {
        "list1": shared,
        "list2": shared,
        "nested": {"a": shared, "b": [shared, shared]},
    }


def make_large_string(n=100_000):
    return "x" * n


def make_mixed_container():
    return {
        "primitives": [1, 2.5, "hello", True, None],
        "nested": {"deep": {"value": [1, 2, [3, 4, {"key": "val"}]]}},
        "bytes": b"\x00\x01\x02\x03",
        "set_data": [1, 2, 3, 4, 5],
    }


BENCHMARK_DATA = {
    "primitives": make_primitives,
    "nested_list_depth5": lambda: make_nested_list(5),
    "nested_list_depth20": lambda: make_nested_list(20),
    "wide_dict_100": lambda: make_wide_dict(100),
    "wide_dict_10000": lambda: make_wide_dict(10000),
    "simple_dataclass": make_dataclass,
    "nested_dataclass": make_nested_dataclass,
    "complex_graph": make_complex_graph,
    "large_string_100k": lambda: make_large_string(100_000),
    "mixed_container": make_mixed_container,
}
