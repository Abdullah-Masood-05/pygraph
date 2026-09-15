"""Pickle-compatible Pickler and Unpickler classes for pysafe-pickle."""
from typing import Any, BinaryIO


class PickleBuffer:
    """A buffer of bytes that can be passed out-of-band during pickling (PEP 574)."""

    def __init__(self, buf: bytes, *, readonly: bool = False):
        self._buf = buf
        self._readonly = readonly

    def raw(self) -> memoryview:
        return memoryview(self._buf)

    @property
    def buf(self) -> memoryview:
        return self.raw()

    @property
    def readonly(self) -> bool:
        return self._readonly


class Pickler:
    """A pickle-compatible pickler using pysafe-pickle's safe serialization."""

    def __init__(
        self,
        file: BinaryIO,
        *,
        protocol: int = 5,
        fix_imports: bool = True,
        buffer_callback: Any = None,
        schema_version: int | None = None,
        hmac_key: bytes | None = None,
    ):
        self._file = file
        self._protocol = protocol
        self._fix_imports = fix_imports
        self._buffer_callback = buffer_callback
        self._schema_version = schema_version
        self._hmac_key = hmac_key

    def dump(self, obj: Any) -> None:
        from pysafe_pickle._pysafe_pickle import dumps

        data = dumps(
            obj,
            protocol=self._protocol,
            schema_version=self._schema_version,
            hmac_key=self._hmac_key,
        )
        self._file.write(data)

    @property
    def proto(self) -> int:
        return self._protocol


class Unpickler:
    """A pickle-compatible unpickler using pysafe-pickle's safe deserialization."""

    def __init__(
        self,
        file: BinaryIO,
        *,
        fix_imports: bool = True,
        encoding: str = "ASCII",
        errors: str = "strict",
        buffers: Any = None,
        allowlist: set[str] | None = None,
    ):
        self._file = file
        self._fix_imports = fix_imports
        self._encoding = encoding
        self._errors = errors
        self._buffers = buffers
        self._allowlist = allowlist

    def load(self) -> Any:
        from pysafe_pickle._pysafe_pickle import loads

        data = self._file.read()
        return loads(data, allowlist=self._allowlist)
