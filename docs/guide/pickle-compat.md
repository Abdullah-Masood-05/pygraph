# Reading Existing Pickle Files

You probably have data in standard `pickle` files already: caches, checkpoints, job results. Opening them with `pickle.load` runs whatever code the file asks for, so a file you did not write yourself is a security risk. Since v1.4.0, pysafe-pickle can read those files **without executing anything chosen by the data**, and convert them to its own format.

| Function | Use it to |
| :--- | :--- |
| `psp.loads_pickle(data, ...)` | Read one pickle from bytes |
| `psp.load_pickle(path_or_file, ...)` | Read one pickle from a path, a binary file object or bytes |
| `psp.convert_pickle(path_or_file_or_bytes, ...)` | Read one pickle and return it re-encoded with `psp.dumps` |

All three accept pickle protocols 0 to 5. See the [API reference](/api/pickle-compat) for every parameter.

---

## How it stays safe

The reader is not a wrapper around `pickle.Unpickler`. It is a small, separate interpreter for the pickle opcodes with these rules:

- **Fixed allowlist of callables.** Only known-safe constructors can be called: the builtin containers and scalars, `datetime` types, `Decimal`, `Fraction`, `UUID`, `OrderedDict`, `Counter`, `deque`, `defaultdict` (builtin factories only), `pathlib` paths, `ZoneInfo`, and the helpers pickle itself uses for them (`copyreg._reconstructor`, `codecs.encode`). Each one checks its arguments before it is called. The exact list is `pysafe_pickle.compat.SAFE_GLOBALS`.
- **No imports driven by data.** A module name in the file is never imported. It is looked up in the allowlist, in your `classes=`, or in your `allow=` pairs, and anything else raises `UnsafeTypeError` before anything is called.
- **No user hooks.** Your classes are rebuilt without calling `__init__`, `__setstate__`, `__reduce__`, `__setattr__`, `append` or `__setitem__`. State is written directly into the instance `__dict__` and declared slots.
- **Bounded resources.** Lengths are checked against the input size, copying constructors draw from a budget proportional to the input, and nesting depth is limited (`max_depth`, default `1000`).

A malicious pickle is rejected without running anything:

```python
import pickle
import pysafe_pickle as psp

class Exploit:
    def __reduce__(self):
        return (eval, ("__import__('os').system('echo pwned')",))

payload = pickle.dumps(Exploit())

try:
    psp.loads_pickle(payload)
except psp.UnsafeTypeError as e:
    print(e)
# global 'builtins.eval' is not allowed (pass the class via classes= or the pair via allow= if you trust it)
```

---

## Reading plain data

Builtins and the supported stdlib types need no configuration:

```python
import datetime as dt
import pickle
from collections import OrderedDict
from decimal import Decimal
import pysafe_pickle as psp

data = pickle.dumps({
    "when": dt.datetime(2026, 10, 28, 9, 30, tzinfo=dt.timezone.utc),
    "price": Decimal("19.99"),
    "order": OrderedDict(a=1, b=2),
    "raw": b"\x00\x01",
})
print(psp.loads_pickle(data))
# {'when': datetime.datetime(2026, 10, 28, 9, 30, tzinfo=datetime.timezone.utc), 'price': Decimal('19.99'), 'order': OrderedDict({'a': 1, 'b': 2}), 'raw': b'\x00\x01'}
```

---

## Your own classes: `classes=`

Instances of your classes are refused unless you list the classes. Pickle stores a class by its module and qualified name, so the class you pass must have the same `__module__` and `__qualname__` as when the file was written (for example `myapp.models.Point`, or `__main__.Point` for a script).

```python
import pickle
from dataclasses import dataclass
import pysafe_pickle as psp

@dataclass
class Point:
    x: int
    y: int

    def __post_init__(self):
        print("__post_init__ ran")

class Config:                      # a plain, non-dataclass class
    def __init__(self, name):
        print("__init__ ran")
        self.name = name

objs = [Point(1, 2), Config("prod")]  # creating them prints both messages
data = pickle.dumps(objs)

try:
    psp.loads_pickle(data)
except psp.UnsafeTypeError as e:
    print(e)
# global '__main__.Point' is not allowed (pass the class via classes= or the pair via allow= if you trust it)

point, config = psp.loads_pickle(data, classes=[Point, Config])
print(point, type(config).__name__, config.name)  # Point(x=1, y=2) Config prod
```

Creating the objects printed both messages; loading them printed nothing, because neither `__post_init__` nor `__init__` runs during the load. Dataclasses, plain classes, slotted classes, namedtuples, `Enum` members and exception subclasses can all be listed. Unlike `psp.loads`, `classes=` here is a list of classes only (not a name mapping).

A `__reduce__` on one of your classes that returns `(YourClass, args)` would need `YourClass(*args)`, which runs `__init__`. That is refused for listed classes (except enums and exceptions) unless the class is also in `allow=`.

---

## Trusting more: `allow=`

`allow=` takes `(module, name)` pairs that you trust completely. They are imported on demand and called with full pickle semantics, including `__setstate__` and arbitrary `__reduce__` arguments. Only list things you would happily run on attacker-controlled input:

```python
import collections
import pickle
import pysafe_pickle as psp

data = pickle.dumps(collections.ChainMap({"a": 1}, {"b": 2}))
print(psp.loads_pickle(data, allow=[("collections", "ChainMap")]))
# ChainMap({'a': 1}, {'b': 2})
```

---

## Files with several pickles

`load_pickle` on a seekable file object stops right after the pickle it read, like `pickle.load`, so you can call it repeatedly until `EOFError`:

```python
import pickle
import pysafe_pickle as psp

with open("events.pkl", "wb") as f:
    for i in range(3):
        pickle.dump({"event": i}, f)

with open("events.pkl", "rb") as f:
    while True:
        try:
            print(psp.load_pickle(f))
        except EOFError:
            break
# {'event': 0}
# {'event': 1}
# {'event': 2}
```

A non-seekable stream is read to the end, so only its first pickle is returned.

---

## Converting to the pysafe-pickle format

`convert_pickle` reads one pickle safely and returns `psp.dumps(obj)`. Extra keyword arguments (such as `hmac_key=`) go to `dumps`. Read the result back with the same `classes=`:

```python
import pickle
from dataclasses import dataclass
from pathlib import Path
import pysafe_pickle as psp

@dataclass
class Point:
    x: int
    y: int

Path("cache.pkl").write_bytes(pickle.dumps({"points": [Point(1, 2), Point(3, 4)]}))

blob = psp.convert_pickle("cache.pkl", classes=[Point], hmac_key=b"secret")
Path("cache.psp").write_bytes(blob)

print(psp.loads(Path("cache.psp").read_bytes(), classes=[Point], hmac_key=b"secret"))
# {'points': [Point(x=1, y=2), Point(x=3, y=4)]}
```

The result can only contain types that `psp.dumps` supports. Plain (non-dataclass) classes can be *read* from a pickle but not converted; `convert_pickle` then raises `UnsafeTypeError: Unsupported type: <name>`. Turn them into dataclasses or plain dicts first.

---

## Python 2 pickles

Protocols 0-2 written by Python 2 store `str` as bytes. As with `pickle.loads`, `encoding=` (default `"ASCII"`) and `errors=` decide how they are decoded, `encoding="bytes"` keeps them as `bytes`, and `fix_imports=True` maps Python 2 module names (`__builtin__`, `copy_reg`, ...) to their Python 3 names.

---

## Errors

| Exception | When |
| :--- | :--- |
| `psp.UnsafeTypeError` (a `TypeError`) | The pickle references a global or operation that is not allowed. Raised before anything is called |
| `pickle.UnpicklingError` | Malformed opcodes, a truncated stream, or a resource limit (depth, copy budget) |
| `ValueError`, `TypeError` | Bad arguments to an allowed constructor, or bad keyword arguments |
| `EOFError` | Empty input or end of file |

Only `UnsafeTypeError` is a `PySafePickleError`. Catch `(psp.UnsafeTypeError, pickle.UnpicklingError, ValueError, TypeError, EOFError)` if you need to handle every failure on untrusted input.

---

## Limits

- NumPy arrays, pandas objects, PyTorch tensors and other third-party types are refused unless you put their reconstructors in `allow=`, which means trusting the file.
- Classes are matched by module and qualified name. A class that has been moved or renamed since the file was written no longer matches, so the file is refused until the class is available under its old name.
