import pygraph


def test_version():
    assert pygraph.__version__ == "0.1.0"


def test_import():
    assert hasattr(pygraph, "__version__")
    assert hasattr(pygraph, "PyGraphError")
    assert hasattr(pygraph, "UnsafeTypeError")
    assert hasattr(pygraph, "SchemaVersionError")
    assert hasattr(pygraph, "HMACError")
