import struct
import sys
import pytest
import pysafe_pickle as psp

def test_untrusted_string_count_bomb():
    # PSPK header + version(1) + schema(0) + flags(0) + string_count(0xFFFFFFFF)
    payload = b"PSPK" + struct.pack("<HI B I", 1, 0, 0, 0xFFFFFFFF)
    with pytest.raises(ValueError, match="exceeds maximum possible elements"):
        psp.loads(payload)

def test_untrusted_object_count_bomb():
    # PSPK header + version(1) + schema(0) + flags(0) + string_count(0) + type_count(0) + obj_count(0xFFFFFFFF)
    payload = b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, 0xFFFFFFFF)
    with pytest.raises(ValueError, match="exceeds maximum possible elements"):
        psp.loads(payload)

def test_untrusted_list_count_bomb():
    # 0 strings, 0 types, 1 record: Tag::List (0x08) with count 0xFFFFFFFF
    header = b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, 1)
    record = bytes([0x08]) + struct.pack("<I", 0xFFFFFFFF)
    payload = header + record
    with pytest.raises(ValueError, match="exceeds maximum possible elements"):
        psp.loads(payload)

def test_untrusted_dict_count_bomb():
    # 0 strings, 0 types, 1 record: Tag::Dict (0x0A) with count 0xFFFFFFFF
    header = b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, 1)
    record = bytes([0x0A]) + struct.pack("<I", 0xFFFFFFFF)
    payload = header + record
    with pytest.raises(ValueError, match="exceeds maximum possible elements"):
        psp.loads(payload)

def test_recursion_depth_limit_deserialization():
    # Construct a payload describing a deeply nested list exceeding recursion limit
    limit = sys.getrecursionlimit()
    depth = limit + 50
    # records: record 0 is [1], record 1 is [2], ..., record depth-1 is []
    # Header: 0 strings, 0 types, depth records
    records_bytes = bytearray()
    for i in range(depth - 1):
        # Tag::List (0x08) + count=1 + ref=i+1
        records_bytes.extend(bytes([0x08]) + struct.pack("<I I", 1, i + 1))
    # Last record is empty list: Tag::List (0x08) + count=0
    records_bytes.extend(bytes([0x08]) + struct.pack("<I", 0))

    header = b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, depth)
    payload = header + bytes(records_bytes)

    with pytest.raises(RecursionError):
        psp.loads(payload)

def test_recursion_depth_limit_serialization():
    # Deeply nested list exceeding safe recursion limit
    curr = []
    root = curr
    for _ in range(300):
        nested = []
        curr.append(nested)
        curr = nested

    with pytest.raises(RecursionError):
        psp.dumps(root)

def test_address_reuse_prevention():
    # Object where a property generates a fresh object every time.
    # Without pinning, the fresh object can be freed and a new object allocated
    # at the exact same address, causing memo aliasing.
    from dataclasses import dataclass

    @dataclass
    class Inner:
        val: int

    class Container:
        def __init__(self):
            self.items = [Inner(i) for i in range(10)]

    c = Container()
    # In pysafe_pickle, custom classes must be dataclasses or builtins
    @dataclass
    class Node:
        a: Inner
        b: Inner
        c: Inner

    # Create distinct objects with distinct contents
    n = Node(Inner(1), Inner(2), Inner(3))
    data = psp.dumps(n)
    res = psp.loads(data)
    assert res.a.val == 1
    assert res.b.val == 2
    assert res.c.val == 3
    assert res.a is not res.b

def test_cyclic_tuple_rejected():
    # Craft a payload where Record 0 is a Tuple containing ref 0 (cycle to self)
    # Header: 0 strings, 0 types, 1 record
    header = b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, 1)
    # Tag::Tuple (0x09) + count=1 + ref=0
    record = bytes([0x09]) + struct.pack("<I I", 1, 0)
    payload = header + record

    with pytest.raises(ValueError, match="Cyclic reference involving immutable container"):
        psp.loads(payload)
