from typing import Any, BinaryIO

__version__: str

def __version() -> str: ...
def dumps(
    obj: Any,
    *,
    protocol: int = 5,
    schema_version: int | None = None,
    hmac_key: bytes | None = None,
) -> bytes: ...
def loads(
    data: bytes | bytearray | memoryview,
    *,
    allowlist: set[str] | None = None,
    hmac_key: bytes | None = None,
) -> Any: ...
def _loads_prefix(
    data: bytes | bytearray | memoryview,
    *,
    allowlist: set[str] | None = None,
    hmac_key: bytes | None = None,
    offset: int = 0,
) -> tuple[Any, int]: ...
def dump(
    obj: Any, file: BinaryIO, **kwargs: Any
) -> None: ...
def load(
    file: BinaryIO,
    *,
    allowlist: set[str] | None = None,
    hmac_key: bytes | None = None,
) -> Any: ...
