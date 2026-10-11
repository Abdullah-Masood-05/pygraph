# Streaming & PickleBuffer

`pysafe-pickle` provides `Pickler` and `Unpickler` classes that follow the shape of the standard library ones, plus a small `PickleBuffer` helper. These are thin wrappers over `dumps` / `loads`.

::: warning Current limitations
- There is **no zero-copy or out-of-band buffer transfer**, and no NumPy or PyTorch support.
- `Unpickler.load()` reads the **entire** file object and decodes it as one payload, so you cannot read several `Pickler.dump()` records back from one stream one at a time. Use one payload per stream or file, or frame the records yourself.
- `buffer_callback`, `buffers`, `fix_imports`, `encoding`, and `errors` are accepted for signature compatibility but ignored.
:::

---

## `Pickler` and `Unpickler`

```python
import io
import pysafe_pickle as psp

buffer = io.BytesIO()
pickler = psp.Pickler(buffer, protocol=5)
pickler.dump({"event": "login", "user_id": 101})

buffer.seek(0)
unpickler = psp.Unpickler(buffer)
print(unpickler.load())  # {'event': 'login', 'user_id': 101}
```

`Pickler(file, *, protocol=5, ...)` writes the serialized bytes to `file` on each `dump()` call. Only protocol 5 is supported. `Unpickler(file, *, allowlist=None, ...)` accepts the same `allowlist` set as `loads`.

### Multiple records in one stream

Because `Unpickler.load()` consumes the whole stream, frame records yourself when you need several, for example with a length prefix:

```python
import io, struct
import pysafe_pickle as psp

stream = io.BytesIO()
for record in [{"event": "login"}, {"event": "click"}]:
    blob = psp.dumps(record)
    stream.write(struct.pack("<I", len(blob)) + blob)

stream.seek(0)
while header := stream.read(4):
    (n,) = struct.unpack("<I", header)
    print(psp.loads(stream.read(n)))
```

---

## `PickleBuffer`

`psp.PickleBuffer` is a lightweight wrapper around a bytes-like object that exposes it as a `memoryview`:

```python
import pysafe_pickle as psp

raw_data = b"Large contiguous memory payload" * 1024
buffer = psp.PickleBuffer(raw_data, readonly=True)

mem_view = buffer.raw()          # memoryview over raw_data, no copy
assert len(mem_view) == len(raw_data)
assert buffer.readonly is True
```

A `PickleBuffer` instance itself cannot currently be passed to `dumps` (it raises `TypeError: Unsupported type`), and PEP 574 out-of-band transfer is not implemented. To serialize binary data, pass `bytes` or `bytearray` directly.
