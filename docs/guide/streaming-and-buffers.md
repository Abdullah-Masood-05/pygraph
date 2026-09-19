# Streaming & PickleBuffer (PEP 574)

For large payloads, IPC communication, and distributed frameworks, `pysafe-pickle` provides streaming classes and zero-copy out-of-band buffer support.

---

## Streaming with `Pickler` and `Unpickler`

When working with streams, sockets, or binary files, use the `Pickler` and `Unpickler` classes:

```python
import io
import pysafe_pickle as psp

# Streaming into an in-memory buffer or network socket
buffer = io.BytesIO()
pickler = psp.Pickler(buffer, protocol=5)

# Write records sequentially
pickler.dump({"event": "login", "user_id": 101})
pickler.dump({"event": "click", "target": "button_buy"})

# Read records sequentially
buffer.seek(0)
unpickler = psp.Unpickler(buffer)

event1 = unpickler.load()
event2 = unpickler.load()

print(event1)  # {'event': 'login', 'user_id': 101}
print(event2)  # {'event': 'click', 'target': 'button_buy'}
```

---

## Zero-Copy Out-of-Band Buffers (`PickleBuffer`)

Python's **PEP 574** introduced `PickleBuffer` to allow large contiguous blocks of memory (such as NumPy arrays, Arrow tables, or PyTorch tensors) to be transferred without memory copies.

`pysafe-pickle` includes a native implementation of `PickleBuffer`:

```python
import pysafe_pickle as psp

raw_data = b"Large contiguous memory payload" * 1024
buffer = psp.PickleBuffer(raw_data, readonly=True)

# Access underlying memoryview without copying
mem_view = buffer.raw()
assert len(mem_view) == len(raw_data)
assert buffer.readonly is True
```

### Benefits:
- **Shared Memory IPC**: Directly map memory between processes.
- **Zero Serialization Overhead**: Huge numerical datasets bypass serialization overhead.
- **Protocol 5 Compatibility**: Fully interoperable with standard Protocol 5 streaming protocols.
