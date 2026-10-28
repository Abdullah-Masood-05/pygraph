class PySafePickleError(Exception):
    """Base exception for all pysafe-pickle errors."""

# Backward compatibility alias
PyGraphError = PySafePickleError

class UnsafeTypeError(PySafePickleError, TypeError):
    """Raised when a type cannot be serialized, or is rejected by the allowlist
    during deserialization. Also a ``TypeError``."""

class SchemaVersionError(PySafePickleError, ValueError):
    """Raised when schema version mismatch cannot be resolved (no migration
    path). Also a ``ValueError``."""

class HMACError(PySafePickleError, ValueError):
    """Raised when HMAC verification fails (data tampering detected, wrong key,
    a signed payload loaded without a key, or a key given for an unsigned
    payload). Also a ``ValueError``."""

class DecodeError(PySafePickleError, ValueError):
    """Raised when a payload is corrupt, truncated or otherwise invalid.
    Also a ``ValueError``."""
