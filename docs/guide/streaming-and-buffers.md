# Streaming & PickleBuffer

`pysafe-pickle` provides `Pickler` and `Unpickler` classes that follow the shape of the standard library ones, plus a small `PickleBuffer` helper.

---

## Writing and Reading Many Records

Each `Pickler.dump()` appends one record to the stream, and each `Unpickler.load()` returns the next one. At the end of the stream `load()` raises `EOFError`, exactly like `pickle`:

```python
import io
import pysafe_pickle as psp

stream = io.BytesIO()
pickler = psp.Pickler(stream)
for event in ["login", "click", "logout"]:
    pickler.dump({"event": event})

stream.seek(0)
unpickler = psp.Unpickler(stream)
while True:
    try:
        print(unpickler.load())
    except EOFError:
        break
# {'event': 'login'}
# {'event': 'click'}
# {'event': 'logout'}
```

The same works with files, and with `psp.dump(obj, f)` instead of a `Pickler`: the records are simply concatenated payloads.

```python
import pysafe_pickle as psp

with open("log.psp", "wb") as f:
    for i in range(3):
        psp.dump({"seq": i}, f)

with open("log.psp", "rb") as f:
    u = psp.Unpickler(f)
    print([u.load() for _ in range(3)])  # [{'seq': 0}, {'seq': 1}, {'seq': 2}]
```

### `Unpickler.load()` vs `load(f)`

| | `Unpickler(f).load()` | `psp.load(f)` |
| :--- | :--- | :--- |
| Reads | In growing chunks, keeping the unread rest in a buffer | The whole rest of the file, each call |
| File position afterwards | May be past the record returned | Right after the record (seekable files) |
| Non-seekable streams (pipes, sockets) | Supported, including short reads | Only the first record; the rest is lost |
| Best for | Many records, large files, network streams | Reading a few records, then handing the file to other code |

Both raise `EOFError` at a clean end of the stream. If the stream ends in the middle of a record, `Unpickler.load()` raises `DecodeError` (or `HMACError` for a signed record).

### Options

`Unpickler(f, allowlist=..., hmac_key=..., classes=...)` applies the same options as [`loads`](/api/core#loads) to every record, and `Pickler(f, hmac_key=...)` signs every record:

```python
import io
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class Tick:
    symbol: str
    price: float

KEY = b"stream-key"
stream = io.BytesIO()
pickler = psp.Pickler(stream, hmac_key=KEY)
pickler.dump(Tick("ABC", 10.5))
pickler.dump(Tick("XYZ", 99.0))

stream.seek(0)
unpickler = psp.Unpickler(stream, hmac_key=KEY, classes=[Tick], allowlist={"Tick"})
print(unpickler.load())  # Tick(symbol='ABC', price=10.5)
print(unpickler.load())  # Tick(symbol='XYZ', price=99.0)
```

`buffer_callback`, `buffers`, `fix_imports`, `encoding` and `errors` are accepted for signature compatibility but ignored. Only protocol 5 is supported.

---

## `PickleBuffer`

`psp.PickleBuffer` is a lightweight wrapper around a bytes-like object that exposes it as a `memoryview`:

```python
import pysafe_pickle as psp

raw_data = b"Large contiguous memory payload" * 1024
buffer = psp.PickleBuffer(raw_data, readonly=True)

mem_view = buffer.raw()  # memoryview over raw_data, no copy
print(len(mem_view) == len(raw_data), buffer.readonly)  # True True
```

::: warning No out-of-band buffers
A `PickleBuffer` instance cannot be passed to `dumps` (it raises `UnsafeTypeError: Unsupported type: PickleBuffer`), and PEP 574 out-of-band transfer is not implemented. There is no zero-copy path and no NumPy or PyTorch support. To serialize binary data, pass `bytes` or `bytearray` directly.
:::
