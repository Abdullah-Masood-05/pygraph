# Migration from pygraph (v1.0.x)

If your project was using `pygraph` (v1.0.0 or v1.0.1), this guide explains the transition to `pysafe-pickle` in **v1.1.0**.

---

## Why the Rename?

The package name on PyPI has always been `pysafe-pickle` (`pip install pysafe-pickle`). In v1.0.x, it was imported as `import pygraph`.

To eliminate confusion between the PyPI distribution name and Python code, the canonical import is now:

```python
import pysafe_pickle as psp
```

---

## The Zero-Breakage Bridge in v1.1.0

`v1.1.0` was designed as a **strictly backward-compatible minor release**:

1. **Existing Code Continues to Run**:
   - `import pygraph` works with 100% identical behavior.
   - `from pygraph.migrations import migrate` works without errors.
   - `from pygraph.exceptions import PyGraphError` works without errors.
2. **Gentle Warning**:
   - Importing `pygraph` emits a `FutureWarning` (`stacklevel=2`) indicating that `pygraph` will be removed in `pysafe-pickle` 2.0.0. The shim stays available throughout v1.x.
   - `pygraph` is a thin alias: `pygraph.dumps is pysafe_pickle.dumps`, and `pygraph.migrations`, `pygraph.exceptions`, and `pygraph.pickler` resolve to the same modules as their `pysafe_pickle` counterparts.
   - To silence the warning while you migrate, update the import rather than filtering it.
3. **Dual Binary Compatibility**:
   - Data files created with `v1.0.x` start with the `PYGR` magic header.
   - The decoder accepts **both** the legacy `PYGR` and the current `PSPK` header, with no warning when reading old data.
   - New payloads are always written with the `PSPK` header.
   - You can load old files without doing any data conversion!
4. **Dual Attribute Support**:
   - Dataclasses with `__pygraph_version__` are versioned just like those with `__pysafe_pickle_version__` (the new name takes precedence if both exist). See [Schema Evolution](/guide/schema-evolution) for how migrations behave.

---

## How to Update Your Code

Updating takes less than a minute. Search and replace:

```python
# Before (v1.0.x)
import pygraph

data = pygraph.dumps(obj)
restored = pygraph.loads(data)

@pygraph.migrate(from_version=1, to_version=2, type_name="User")
def migrate_user(state):
    ...
```

```python
# After (v1.1.0+)
import pysafe_pickle as psp

data = psp.dumps(obj)
restored = psp.loads(data)

@psp.migrate(from_version=1, to_version=2, type_name="User")
def migrate_user(state):
    ...
```

---

## Behavior Notes for Upgrading

- Anything loaded from old `PYGR` files is rebuilt as a dynamically created class named after the original dataclass, exactly as with new files (not your original class).
- Since v1.3.0, additional types are supported (arbitrary-size `int`, `bytearray`, `complex`, lone-surrogate strings). Old decoders can still read payloads that do not use them.
- `hmac_key` raises `NotImplementedError` (HMAC is not yet implemented).
