"""HMAC integrity, the exception hierarchy, and multi-record streams."""
import hashlib
import hmac
import io
import struct
import sys
from dataclasses import asdict, dataclass, is_dataclass

import pytest

import pysafe_pickle as psp
from pysafe_pickle import (
    DecodeError,
    HMACError,
    PySafePickleError,
    SchemaVersionError,
    UnsafeTypeError,
)
from pysafe_pickle._pysafe_pickle import _loads_prefix

KEY = b"super-secret-key"
FLAGS_OFFSET = 10
TAG_LEN = 32


@dataclass
class IntegrityPoint:
    x: int
    y: int


SAMPLES = [
    None,
    0,
    -(2**80),
    3.5,
    "text",
    b"\x00\xff",
    [1, "two", 3.0, None],
    {"a": [1, 2], "b": {"c": (1, 2)}},
    {1, 2, 3},
    IntegrityPoint(1, 2),
]


def norm(x):
    """Comparable form: dataclass instances (reconstructed or original) by
    class name and fields; containers recursively."""
    if is_dataclass(x) and not isinstance(x, type):
        return ("dataclass", type(x).__name__, asdict(x))
    if isinstance(x, (list, tuple)):
        return type(x)(norm(v) for v in x)
    if isinstance(x, dict):
        return {k: norm(v) for k, v in x.items()}
    return x


def _flip(blob: bytes, pos: int) -> bytes:
    b = bytearray(blob)
    b[pos] ^= 0x40
    return bytes(b)


class NonSeekable(io.RawIOBase):
    """A read-only stream without seek support that returns short reads."""

    def __init__(self, data: bytes, max_read: int = 7):
        self._data = data
        self._pos = 0
        self._max = max_read

    def readable(self):
        return True

    def seekable(self):
        return False

    def readinto(self, b):
        n = min(len(b), self._max, len(self._data) - self._pos)
        b[:n] = self._data[self._pos:self._pos + n]
        self._pos += n
        return n


# --------------------------------------------------------------------------
# HMAC
# --------------------------------------------------------------------------


class TestHmacRoundtrip:
    @pytest.mark.parametrize("obj", SAMPLES, ids=repr)
    def test_roundtrip(self, obj):
        blob = psp.dumps(obj, hmac_key=KEY)
        assert norm(psp.loads(blob, hmac_key=KEY)) == norm(obj)

    def test_wire_format(self):
        obj = {"k": [1, 2, 3]}
        unsigned = psp.dumps(obj)
        signed = psp.dumps(obj, hmac_key=KEY)
        assert len(signed) == len(unsigned) + TAG_LEN
        assert unsigned[FLAGS_OFFSET] & 1 == 0
        assert signed[FLAGS_OFFSET] & 1 == 1
        expected_body = bytearray(unsigned)
        expected_body[FLAGS_OFFSET] |= 1
        assert signed[:-TAG_LEN] == bytes(expected_body)
        # The tag is plain HMAC-SHA256 over every preceding byte.
        assert signed[-TAG_LEN:] == hmac.new(KEY, signed[:-TAG_LEN], hashlib.sha256).digest()

    def test_unsigned_wire_format_unchanged(self):
        assert psp.loads(psp.dumps([1, 2])) == [1, 2]
        assert psp.dumps([1, 2])[FLAGS_OFFSET] == 0

    def test_bytes_like_key_and_data(self):
        blob = psp.dumps([1], hmac_key=bytearray(KEY))
        assert psp.loads(bytearray(blob), hmac_key=memoryview(KEY)) == [1]
        assert psp.loads(memoryview(blob), hmac_key=KEY) == [1]

    def test_large_payload(self):
        obj = list(range(200_000))
        blob = psp.dumps(obj, hmac_key=KEY)
        assert norm(psp.loads(blob, hmac_key=KEY)) == norm(obj)
        with pytest.raises(HMACError):
            psp.loads(_flip(blob, len(blob) // 2), hmac_key=KEY)

    def test_trailing_bytes_ignored(self):
        blob = psp.dumps([1, 2], hmac_key=KEY)
        assert psp.loads(blob + b"trailing junk", hmac_key=KEY) == [1, 2]
        assert psp.loads(blob + blob, hmac_key=KEY) == [1, 2]

    def test_allowlist_with_hmac(self):
        blob = psp.dumps(IntegrityPoint(1, 2), hmac_key=KEY)
        assert norm(psp.loads(blob, hmac_key=KEY, allowlist={"IntegrityPoint"})) == norm(IntegrityPoint(1, 2))
        with pytest.raises(UnsafeTypeError, match="not in the allowlist"):
            psp.loads(blob, hmac_key=KEY, allowlist=set())

    def test_dump_load_file(self):
        f = io.BytesIO()
        psp.dump({"a": 1}, f, hmac_key=KEY)
        f.seek(0)
        assert psp.load(f, hmac_key=KEY) == {"a": 1}
        f.seek(0)
        with pytest.raises(HMACError, match="pass hmac_key"):
            psp.load(f)

    def test_pickler_unpickler(self):
        f = io.BytesIO()
        psp.Pickler(f, hmac_key=KEY).dump([1, 2])
        assert f.getvalue()[FLAGS_OFFSET] & 1
        f.seek(0)
        assert psp.Unpickler(f, hmac_key=KEY).load() == [1, 2]
        f.seek(0)
        with pytest.raises(HMACError):
            psp.Unpickler(f, hmac_key=b"other").load()


class TestHmacRejection:
    BLOB = psp.dumps({"name": "x", "vals": [1, 2.5, b"z"], "p": IntegrityPoint(3, 4)}, hmac_key=KEY)

    @pytest.mark.parametrize("pos", range(4, len(BLOB)))
    def test_tampered_byte(self, pos):
        with pytest.raises(HMACError):
            psp.loads(_flip(self.BLOB, pos), hmac_key=KEY)

    def test_tampered_positions_explicit(self):
        n = len(self.BLOB)
        # format version, flags, first table byte, middle, first/last tag byte
        for pos in (4, FLAGS_OFFSET, 11, n // 2, n - TAG_LEN, n - 1):
            with pytest.raises(HMACError):
                psp.loads(_flip(self.BLOB, pos), hmac_key=KEY)

    def test_tampered_magic_is_decode_error(self):
        with pytest.raises(DecodeError, match="Invalid magic bytes"):
            psp.loads(_flip(self.BLOB, 0), hmac_key=KEY)

    def test_cleared_flag_is_rejected(self):
        b = bytearray(self.BLOB)
        b[FLAGS_OFFSET] &= ~1
        with pytest.raises(HMACError, match="payload is not signed"):
            psp.loads(bytes(b), hmac_key=KEY)

    def test_truncated(self):
        for cut in (1, 5, TAG_LEN, TAG_LEN + 3):
            with pytest.raises(HMACError):
                psp.loads(self.BLOB[:-cut], hmac_key=KEY)

    def test_tag_stripped(self):
        with pytest.raises(HMACError):
            psp.loads(self.BLOB[:-TAG_LEN], hmac_key=KEY)

    def test_wrong_key(self):
        with pytest.raises(HMACError, match="HMAC verification failed"):
            psp.loads(self.BLOB, hmac_key=b"wrong key")

    def test_missing_key(self):
        with pytest.raises(HMACError, match=r"payload is signed; pass hmac_key"):
            psp.loads(self.BLOB)

    def test_unsigned_with_key(self):
        with pytest.raises(HMACError, match="payload is not signed"):
            psp.loads(psp.dumps([1]), hmac_key=KEY)

    def test_empty_key(self):
        with pytest.raises(ValueError):
            psp.dumps([1], hmac_key=b"")
        with pytest.raises(ValueError):
            psp.loads(self.BLOB, hmac_key=b"")
        with pytest.raises(ValueError):
            psp.dump([1], io.BytesIO(), hmac_key=b"")
        with pytest.raises(ValueError):
            psp.Pickler(io.BytesIO(), hmac_key=b"").dump([1])
        with pytest.raises(ValueError):
            psp.load(io.BytesIO(self.BLOB), hmac_key=b"")
        with pytest.raises(ValueError):
            _loads_prefix(self.BLOB, hmac_key=b"")

    def test_hmac_errors_are_value_errors(self):
        with pytest.raises(ValueError):
            psp.loads(self.BLOB)
        try:
            psp.loads(self.BLOB, hmac_key=b"nope")
        except HMACError as e:
            assert isinstance(e, PySafePickleError)
            assert isinstance(e, ValueError)
        else:
            pytest.fail("expected HMACError")


# --------------------------------------------------------------------------
# Exception hierarchy
# --------------------------------------------------------------------------


class TestExceptions:
    def test_hierarchy(self):
        assert issubclass(UnsafeTypeError, PySafePickleError)
        assert issubclass(UnsafeTypeError, TypeError)
        assert issubclass(HMACError, PySafePickleError)
        assert issubclass(HMACError, ValueError)
        assert issubclass(SchemaVersionError, PySafePickleError)
        assert issubclass(SchemaVersionError, ValueError)
        assert issubclass(DecodeError, PySafePickleError)
        assert issubclass(DecodeError, ValueError)
        assert "DecodeError" in psp.__all__
        from pysafe_pickle import exceptions
        assert exceptions.DecodeError is DecodeError

    def test_unsupported_type_on_dumps(self):
        class Opaque:
            pass

        with pytest.raises(UnsafeTypeError, match="Unsupported type") as ei:
            psp.dumps(Opaque())
        assert isinstance(ei.value, TypeError)
        with pytest.raises(UnsafeTypeError, match="Unsupported type"):
            psp.dump([Opaque()], io.BytesIO())
        with pytest.raises(UnsafeTypeError, match="Unsupported type"):
            psp.Pickler(io.BytesIO()).dump({"k": Opaque()})

    def test_allowlist_rejection(self):
        blob = psp.dumps(IntegrityPoint(1, 2))
        with pytest.raises(UnsafeTypeError, match="not in the allowlist") as ei:
            psp.loads(blob, allowlist=set())
        assert isinstance(ei.value, TypeError)
        with pytest.raises(UnsafeTypeError):
            psp.load(io.BytesIO(blob), allowlist=set())
        with pytest.raises(UnsafeTypeError):
            psp.Unpickler(io.BytesIO(blob), allowlist=set()).load()

    @pytest.mark.parametrize(
        "payload, match",
        [
            (b"", "too short"),
            (b"NOPE" + bytes(20), "Invalid magic bytes"),
            (b"PSPK" + struct.pack("<HI B I", 1, 0, 0, 0xFFFFFFFF), "exceeds maximum possible elements"),
            (b"PSPK" + struct.pack("<HI B I I I", 1, 0, 0, 0, 0, 1) + b"\xee", "Unknown tag"),
        ],
    )
    def test_corrupt_payloads(self, payload, match):
        with pytest.raises(DecodeError, match=match) as ei:
            psp.loads(payload)
        assert isinstance(ei.value, ValueError)

    def test_truncated_payload(self):
        blob = psp.dumps({"a": [1, 2, 3], "b": "hello"})
        for cut in range(1, len(blob)):
            with pytest.raises(DecodeError):
                psp.loads(blob[:cut])

    def test_missing_migration_path(self):
        from pysafe_pickle.migrations import get_migration_chain

        with pytest.raises(SchemaVersionError, match="No migration path") as ei:
            get_migration_chain("NoSuchTypeForIntegrityTests", 1, 2)
        assert isinstance(ei.value, ValueError)

    def test_recursion_error_unchanged(self):
        obj = []
        for _ in range(sys.getrecursionlimit() + 100):
            obj = [obj]
        with pytest.raises(RecursionError):
            psp.dumps(obj)


# --------------------------------------------------------------------------
# _loads_prefix
# --------------------------------------------------------------------------


class TestLoadsPrefix:
    @pytest.mark.parametrize("obj", SAMPLES, ids=repr)
    def test_consumed_length(self, obj):
        blob = psp.dumps(obj)
        assert norm(_loads_prefix(blob)) == (norm(obj), len(blob))
        signed = psp.dumps(obj, hmac_key=KEY)
        assert len(signed) == len(blob) + TAG_LEN
        assert norm(_loads_prefix(signed, hmac_key=KEY)) == (norm(obj), len(blob) + TAG_LEN)

    def test_with_trailing_data(self):
        a, b = psp.dumps([1, 2]), psp.dumps({"x": 1})
        assert _loads_prefix(a + b) == ([1, 2], len(a))
        assert _loads_prefix(a + b, offset=len(a)) == ({"x": 1}, len(b))
        sa, sb = psp.dumps([1, 2], hmac_key=KEY), psp.dumps("s", hmac_key=KEY)
        assert _loads_prefix(sa + sb, hmac_key=KEY) == ([1, 2], len(sa))
        assert _loads_prefix(sa + sb, hmac_key=KEY, offset=len(sa)) == ("s", len(sb))
        assert _loads_prefix(sa + b"junk" * 20, hmac_key=KEY) == ([1, 2], len(sa))

    def test_bytes_like(self):
        blob = psp.dumps([1])
        assert _loads_prefix(bytearray(blob)) == ([1], len(blob))
        assert _loads_prefix(memoryview(blob + b"xx")) == ([1], len(blob))

    def test_incomplete(self):
        blob = psp.dumps([1, 2, 3])
        with pytest.raises(DecodeError):
            _loads_prefix(blob[:-1])
        signed = psp.dumps([1, 2, 3], hmac_key=KEY)
        with pytest.raises(DecodeError, match="HMAC tag truncated"):
            _loads_prefix(signed[:-1], hmac_key=KEY)

    def test_wrong_key(self):
        signed = psp.dumps([1], hmac_key=KEY)
        with pytest.raises(HMACError):
            _loads_prefix(signed + signed, hmac_key=b"wrong")

    def test_loads_still_ignores_trailing_bytes(self):
        blob = psp.dumps([1, 2])
        assert psp.loads(blob + b"\x00garbage") == [1, 2]


# --------------------------------------------------------------------------
# Multi-record streams
# --------------------------------------------------------------------------

RECORDS = [
    [1, 2, 3],
    {"name": "alice", "tags": {"a", "b"}},
    "a string",
    IntegrityPoint(5, 6),
    None,
    b"x" * 100_000,  # bigger than one read chunk
    list(range(50_000)),
    (1.5, -2),
]


class TestStreams:
    @pytest.mark.parametrize("key", [None, KEY])
    def test_pickler_unpickler_bytesio(self, key):
        f = io.BytesIO()
        p = psp.Pickler(f, hmac_key=key)
        for r in RECORDS:
            p.dump(r)
        f.seek(0)
        u = psp.Unpickler(f, hmac_key=key)
        assert norm([u.load() for _ in RECORDS]) == norm(RECORDS)
        with pytest.raises(EOFError):
            u.load()
        with pytest.raises(EOFError):
            u.load()

    def test_unpickler_empty_stream(self):
        with pytest.raises(EOFError):
            psp.Unpickler(io.BytesIO()).load()

    def test_unpickler_non_seekable(self):
        data = b"".join(psp.dumps(r) for r in RECORDS)
        u = psp.Unpickler(NonSeekable(data))
        assert norm([u.load() for _ in RECORDS]) == norm(RECORDS)
        with pytest.raises(EOFError):
            u.load()

    def test_unpickler_non_seekable_buffered(self):
        data = b"".join(psp.dumps(r, hmac_key=KEY) for r in RECORDS)
        u = psp.Unpickler(io.BufferedReader(NonSeekable(data, 1000)), hmac_key=KEY)
        assert norm([u.load() for _ in RECORDS]) == norm(RECORDS)
        with pytest.raises(EOFError):
            u.load()

    def test_unpickler_partial_record(self):
        data = psp.dumps([1]) + psp.dumps({"a": 2})[:-3]
        u = psp.Unpickler(io.BytesIO(data))
        assert u.load() == [1]
        with pytest.raises(DecodeError):
            u.load()

    def test_unpickler_partial_signed_record(self):
        data = psp.dumps([1], hmac_key=KEY) + psp.dumps([2], hmac_key=KEY)[:-5]
        u = psp.Unpickler(io.BytesIO(data), hmac_key=KEY)
        assert u.load() == [1]
        with pytest.raises((DecodeError, HMACError)):
            u.load()

    def test_unpickler_trailing_garbage(self):
        u = psp.Unpickler(io.BytesIO(psp.dumps([1]) + b"garbage"))
        assert u.load() == [1]
        with pytest.raises(DecodeError):
            u.load()

    def test_dump_load_bytesio(self):
        f = io.BytesIO()
        ends = []
        for r in RECORDS:
            psp.dump(r, f)
            ends.append(f.tell())
        f.seek(0)
        for r, end in zip(RECORDS, ends):
            assert norm(psp.load(f)) == norm(r)
            assert f.tell() == end
        with pytest.raises(EOFError):
            psp.load(f)

    def test_dump_load_signed_bytesio(self):
        f = io.BytesIO()
        for r in RECORDS:
            psp.dump(r, f, hmac_key=KEY)
        f.seek(0)
        assert norm([psp.load(f, hmac_key=KEY) for _ in RECORDS]) == norm(RECORDS)
        with pytest.raises(EOFError):
            psp.load(f, hmac_key=KEY)

    def test_dump_load_real_file(self, tmp_path):
        path = tmp_path / "records.bin"
        with open(path, "wb") as f:
            for r in RECORDS:
                psp.dump(r, f)
            p = psp.Pickler(f, hmac_key=KEY)
            p.dump({"signed": True})
        with open(path, "rb") as f:
            for r in RECORDS:
                assert norm(psp.load(f)) == norm(r)
            assert psp.load(f, hmac_key=KEY) == {"signed": True}
            assert f.tell() == path.stat().st_size
            with pytest.raises(EOFError):
                psp.load(f)
        with open(path, "rb") as f:
            u = psp.Unpickler(f)
            assert norm([u.load() for _ in RECORDS]) == norm(RECORDS)
            with pytest.raises(HMACError, match="pass hmac_key"):
                u.load()

    def test_unpickler_real_file(self, tmp_path):
        path = tmp_path / "pickler.bin"
        with open(path, "wb") as f:
            p = psp.Pickler(f)
            for r in RECORDS:
                p.dump(r)
        with open(path, "rb") as f:
            u = psp.Unpickler(f)
            assert norm([u.load() for _ in RECORDS]) == norm(RECORDS)
            with pytest.raises(EOFError):
                u.load()

    def test_load_non_seekable_reads_first_record(self):
        data = psp.dumps([1]) + psp.dumps([2])
        assert psp.load(NonSeekable(data, 1 << 20)) == [1]

    def test_load_empty_file(self):
        with pytest.raises(EOFError):
            psp.load(io.BytesIO())


class TestDumpSignature:
    def test_unknown_keyword_rejected(self):
        with pytest.raises(TypeError):
            psp.dump([1], io.BytesIO(), hmac_kye=b"k")

    def test_protocol_checked_like_dumps(self):
        with pytest.raises(ValueError, match="only protocol 5"):
            psp.dump([1], io.BytesIO(), protocol=4)

    def test_pickle_compatible_arguments_accepted(self):
        f = io.BytesIO()
        psp.dump([1], f, None, fix_imports=False, buffer_callback=None)
        assert psp.loads(f.getvalue()) == [1]


def test_bad_zoneinfo_key_is_decode_error():
    import zoneinfo
    from pysafe_pickle.exceptions import DecodeError

    blob = bytearray(psp.dumps(zoneinfo.ZoneInfo("UTC")))
    i = blob.rindex(b"UTC")
    blob[i:i + 3] = b"../"
    with pytest.raises(DecodeError, match="Invalid ZoneInfo key"):
        psp.loads(bytes(blob))
