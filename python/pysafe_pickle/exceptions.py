class PySafePickleError(Exception):
    """Base exception for all pysafe-pickle errors."""

# Backward compatibility alias
PyGraphError = PySafePickleError

class UnsafeTypeError(PySafePickleError):
    """Raised when an unallowlisted type is encountered during deserialization."""

class SchemaVersionError(PySafePickleError):
    """Raised when schema version mismatch cannot be resolved."""

class HMACError(PySafePickleError):
    """Raised when HMAC verification fails (data tampering detected)."""
