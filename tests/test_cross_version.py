import warnings
from pathlib import Path
from dataclasses import dataclass
import pytest

LEGACY_BLOB_PATH = Path(__file__).parent / "fixtures" / "v101_user_v2.bin"


def _reset_pygraph():
    import sys
    for k in list(sys.modules.keys()):
        if k == "pygraph" or k.startswith("pygraph."):
            sys.modules.pop(k, None)


def test_v101_bytes_load_under_new_name_no_warning():
    """Verify legacy v1.0.1 blobs with PYGR magic load under pysafe_pickle with 0 warnings."""
    import pysafe_pickle as psp

    raw = LEGACY_BLOB_PATH.read_bytes()
    assert raw[:4] == b"PYGR"

    with warnings.catch_warnings(record=True) as rec:
        warnings.simplefilter("always")
        obj = psp.loads(raw)

    assert getattr(obj, "name") == "Alice"
    assert getattr(obj, "age") == 30
    assert getattr(obj, "email") == "alice@example.com"
    future_warnings = [w for w in rec if issubclass(w.category, FutureWarning)]
    assert not future_warnings, f"Expected no FutureWarning on happy path, got: {future_warnings}"


def test_v101_bytes_load_under_legacy_shim():
    """Verify legacy v1.0.1 blobs load through pygraph shim and emit FutureWarning."""
    _reset_pygraph()
    with pytest.warns(FutureWarning, match="deprecated"):
        import pygraph

    raw = LEGACY_BLOB_PATH.read_bytes()
    obj = pygraph.loads(raw)
    assert getattr(obj, "name") == "Alice"
    assert getattr(obj, "age") == 30
    assert getattr(obj, "email") == "alice@example.com"


def test_new_blob_uses_new_magic():
    """Verify new serialization uses b'PSPK' magic bytes."""
    import pysafe_pickle as psp

    blob = psp.dumps({"hello": "world"})
    assert blob[:4] == b"PSPK"


def test_legacy_dotted_decorator_still_works():
    """Verify from pygraph.migrations import migrate works seamlessly."""
    _reset_pygraph()
    with pytest.warns(FutureWarning, match="deprecated"):
        from pygraph.migrations import migrate

    @migrate(from_version=1, to_version=2, type_name="Widget")
    def m(state: dict) -> dict:
        state["migrated"] = True
        return state

    assert callable(m)


def test_migration_cross_registered_legacy_applied_new():
    """Verify migrations registered via pygraph apply when queried via pysafe_pickle."""
    _reset_pygraph()
    with pytest.warns(FutureWarning, match="deprecated"):
        import pygraph
    import pysafe_pickle as psp

    @pygraph.migrate(from_version=1, to_version=2, type_name="CrossWidget")
    def migrate_widget(state: dict) -> dict:
        state["tag"] = "migrated_from_v1"
        return state

    initial_state = {"name": "gear", "__pygraph_version__": 1}
    migrated_state = psp.apply_migrations("CrossWidget", initial_state, 1, 2)
    assert migrated_state["tag"] == "migrated_from_v1"
    assert migrated_state["__pysafe_pickle_version__"] == 2
    assert migrated_state["__pygraph_version__"] == 2


def test_exceptions_aliasing():
    """Verify PyGraphError is an alias of PySafePickleError."""
    import pysafe_pickle as psp
    from pygraph.exceptions import PyGraphError, UnsafeTypeError

    assert issubclass(UnsafeTypeError, psp.PySafePickleError)
    assert psp.PyGraphError is psp.PySafePickleError
