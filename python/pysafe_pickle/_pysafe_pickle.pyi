from typing import Any, Callable, BinaryIO

def __version() -> str: ...
def dumps(
    obj: Any,
    *,
    protocol: int = 5,
    schema_version: int | None = None,
    hmac_key: bytes | None = None,
) -> bytes: ...
def loads(
    data: bytes, *, allowlist: set[str] | None = None
) -> Any: ...
def dump(
    obj: Any, file: BinaryIO, **kwargs: Any
) -> None: ...
def load(
    file: BinaryIO, **kwargs: Any
) -> Any: ...

class PickleBuffer:
    def __init__(self, buf: bytes, *, readonly: bool = False) -> None: ...
    def raw(self) -> memoryview: ...
    @property
    def buf(self) -> memoryview: ...
    @property
    def readonly(self) -> bool: ...

class Pickler:
    def __init__(
        self,
        file: BinaryIO,
        *,
        protocol: int = 5,
        fix_imports: bool = True,
        buffer_callback: Any = None,
        schema_version: int | None = None,
        hmac_key: bytes | None = None,
    ) -> None: ...
    def dump(self, obj: Any) -> None: ...
    @property
    def proto(self) -> int: ...

class Unpickler:
    def __init__(
        self,
        file: BinaryIO,
        *,
        fix_imports: bool = True,
        encoding: str = "ASCII",
        errors: str = "strict",
        buffers: Any = None,
        allowlist: set[str] | None = None,
    ) -> None: ...
    def load(self) -> Any: ...

def migrate(*, from_version: int, to_version: int, type_name: str | None = None) -> Callable: ...
