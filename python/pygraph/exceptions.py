class PyGraphError(Exception):
    """Base exception for all pygraph errors."""

class UnsafeTypeError(PyGraphError):
    """Raised when an unallowlisted type is encountered during deserialization."""

class SchemaVersionError(PyGraphError):
    """Raised when schema version mismatch cannot be resolved."""

class HMACError(PyGraphError):
    """Raised when HMAC verification fails (data tampering detected)."""
