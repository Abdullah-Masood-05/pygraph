# Rust Engine & PyO3 Architecture

`pysafe-pickle` is built as a compiled Rust cdylib extension module using PyO3 and Maturin.

---

## High-Level Pipeline

```mermaid
graph TD
    subgraph Serialization: dumps
        PyObj[Python Object Graph] -->|Walker DFS| Walker[Rust Walker]
        Walker -->|Cycle Detection| Memo[Memo Table: ptr -> id]
        Walker -->|Interning| StrTable[String Table]
        Walker -->|Schema Registry| TypeTable[Type Registry]
        Walker -->|Record Graph| Encoder[Binary Encoder]
        Encoder -->|PSPK Bytes| OutBytes[Serialized Bytes]
    end

    subgraph Deserialization: loads
        InBytes[Serialized Bytes] -->|Header & Tag Parser| Decoder[Binary Decoder]
        Decoder -->|Strings & Types| Reconstruct[Object Reconstructor]
        Reconstruct -->|Dynamic Class Builder| PyDataclass[Safe Python Object Graph]
        Reconstruct -->|Version Mismatch?| Migrations[Apply Python Migrations]
    end
```

---

## 1. DFS Object Traversal (`traversal.rs`)
The `Walker` traverses the Python object graph using depth-first search:
- Inspects Python object type strings.
- Replaces duplicate pointers with `Record::Reference(id)` to prevent infinite loops and preserve identical object topologies.
- Unknown or dangerous types (e.g. modules, functions) immediately trigger `UnsafeTypeError` inside Rust before writing any bytes.

---

## 2. ABI3 Wheel Distribution
`pysafe-pickle` targets PyO3's `abi3-py310` feature:
- A single pre-built wheel runs across Python 3.10, 3.11, 3.12, 3.13, and future Python 3.x minor releases without recompilation.
- Zero local C/Rust compiler requirement for end users.
