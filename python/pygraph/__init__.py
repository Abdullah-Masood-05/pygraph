import warnings
import sys as _sys
import pysafe_pickle

warnings.warn(
    "'pygraph' is deprecated and will be removed in pysafe-pickle 2.0.0. "
    "Please use 'import pysafe_pickle as psp' instead.",
    FutureWarning,
    stacklevel=2,
)

from pysafe_pickle import *  # noqa: F401, F403
from pysafe_pickle import __version__
from pysafe_pickle import exceptions, migrations, pickler, _reconstruct

# Register submodule aliases so `from pygraph.migrations import migrate` works seamlessly
_sys.modules["pygraph.exceptions"] = exceptions
_sys.modules["pygraph.migrations"] = migrations
_sys.modules["pygraph.pickler"] = pickler
_sys.modules["pygraph._reconstruct"] = _reconstruct
