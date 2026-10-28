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
        hmac_key: bytes | None = None,
        classes: Any = None,
    ):
        self._file = file
        self._fix_imports = fix_imports
        self._encoding = encoding
        self._errors = errors
        self._buffers = buffers
        self._allowlist = allowlist
        self._hmac_key = hmac_key
        self._classes = classes
        # Bytes read from the file but not yet decoded: self._buf[self._pos:].
        self._buf = b""
        self._pos = 0
        self._eof = False

    def load(self) -> Any:
        """Read and return the next record of the stream.

        Repeated calls return successive records written by repeated
        ``Pickler.dump()`` / ``dump()`` calls. The file is read in growing
        chunks and any bytes read past a record are kept in an internal buffer
        for the next call, so the file position may be ahead of the last
        record returned. Raises ``EOFError`` at a clean end of stream and
        ``DecodeError`` if the stream ends inside a record.
        """
        from pysafe_pickle._pysafe_pickle import _loads_prefix
        from pysafe_pickle.exceptions import DecodeError, HMACError

        chunk_size = _CHUNK_SIZE
        while True:
            if self._pos < len(self._buf):
                try:
                    obj, used = _loads_prefix(
                        self._buf,
                        allowlist=self._allowlist,
                        hmac_key=self._hmac_key,
                        offset=self._pos,
                        classes=self._classes,
                    )
                except (DecodeError, HMACError):
                    # An incomplete record looks corrupt (or, when signed,
                    # unverifiable) until the rest of it has been read.
                    if self._eof:
                        raise
                else:
                    self._pos += used
                    return obj
            elif self._eof:
                raise EOFError("Ran out of input")

            chunk = self._file.read(chunk_size)
            if not chunk:
                self._eof = True
                continue
            self._buf = self._buf[self._pos:] + bytes(chunk)
            self._pos = 0
            # Grow geometrically so a large record is re-scanned O(log n) times.
            chunk_size = max(chunk_size, len(self._buf))


_CHUNK_SIZE = 64 * 1024
