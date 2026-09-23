from importlib.metadata import version, PackageNotFoundError

try:
    __version__ = version("pysafe-pickle")
except PackageNotFoundError:
    try:
        from pysafe_pickle._pysafe_pickle import __version__
    except ImportError:
        __version__ = "1.2.0"

from pysafe_pickle._pysafe_pickle import dumps, loads, dump, load
from pysafe_pickle.exceptions import (
    PySafePickleError,
    PyGraphError,
    UnsafeTypeError,
    SchemaVersionError,
    HMACError,
)
from pysafe_pickle.pickler import Pickler, Unpickler, PickleBuffer
from pysafe_pickle.migrations import (
    migrate,
    get_migrations,
    get_migration_chain,
    apply_migrations,
    get_schema_version,
    set_schema_version,
)

__all__ = [
    "__version__",
    "dumps",
    "loads",
    "dump",
    "load",
    "Pickler",
    "Unpickler",
    "PickleBuffer",
    "migrate",
    "get_migrations",
    "get_migration_chain",
    "apply_migrations",
    "get_schema_version",
    "set_schema_version",
    "PySafePickleError",
    "PyGraphError",
    "UnsafeTypeError",
    "SchemaVersionError",
    "HMACError",
]
