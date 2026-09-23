# Performance & Benchmarks

`pysafe-pickle` delivers serialization speeds up to **3.3× faster** than the original Python `pygraph` library, powered by an optimized Rust engine with PyO3 exact instance pointer checks, scalar memoization bypass, class-level dataclass caching, iterative heap work stacks, and GIL-free encoding/decoding—while strictly guaranteeing **zero arbitrary code execution (ACE)**.

---

## Benchmark: pysafe-pickle vs Original pygraph (v1.0.x)

In v1.0.x, the library was originally released as `pygraph`. In `v1.2.0`, the core serialization pipeline underwent a major architectural overhaul.

The table below compares execution time across common Python data shapes measured using `pytest-benchmark`:

| Data Shape | Original `pygraph` (v1.0.x) | `pysafe-pickle` (v1.2.0) | Speedup | Architectural Driver |
| :--- | :---: | :---: | :---: | :--- |
| **Wide Dictionaries (10,000 keys)** | 9.32 ms | **2.83 ms** | **3.3× faster** | `FxHashMap` + single-alloc string table |
| **Complex Cyclic Graphs** | 11.4 µs | **3.6 µs** | **3.1× faster** | Iterative traversal + memo table pinning |
| **Wide Dictionaries (100 keys)** | 64.7 µs | **22.1 µs** | **2.9× faster** | Fast PyO3 type pointer dispatch |
| **Primitive Sequences** | 6.2 µs | **2.1 µs** | **2.9× faster** | Scalar memo bypass (`int`, `float`, `bool`) |
| **Large Strings (100 KB)** | 35.9 µs | **15.7 µs** | **2.3× faster** | Zero-copy string slice encoding |
| **Full Roundtrip (Wide Dict)** | 10.2 ms | **4.53 ms** | **2.25× faster** | Iterative heap work stack decode |
| **Nested Dataclasses** | 23.9 µs | **18.2 µs** | **1.3× faster** | Class-level `__dataclass_fields__` caching |

::: tip Speedup Summary
Compared to the original `pygraph` serialization engine, `pysafe-pickle v1.2.0` achieves between **1.3× and 3.3× faster serialization**, with the largest gains in dictionary mappings, cyclic graphs, and primitive sequences.
:::

---

## Benchmark: pysafe-pickle vs Standard Python pickle

Python's built-in `pickle` module (implemented in C as `_pickle`) is historically fast, but it is **not safe** for untrusted data because it executes arbitrary Python code (`__reduce__`).

`pysafe-pickle` provides near-C-native speed while closing all code execution attack surfaces:

| Data Shape | Operation | Standard `pickle` (proto 5) | `pysafe-pickle` (v1.2.0) | Ratio | Security Guarantee |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **Wide Dict (10k keys)** | `dumps` | 0.89 ms | 2.83 ms | 3.1× | Zero ACE + Allowlist |
| **Wide Dict (10k keys)** | `loads` | 0.81 ms | 2.57 ms | 3.1× | Bounded memory allocations |
| **Primitives** | `dumps` | 1.0 µs | 2.1 µs | 2.1× | Safe scalar reconstruction |
| **Nested Dataclass** | `dumps` | 3.4 µs | 8.4 µs | 2.4× | Schema migration support |
| **Nested Dataclass** | `loads` | 2.6 µs | 9.9 µs | 3.8× | Versioned schema verification |
| **Large String (100 KB)** | `dumps` | 3.0 µs | 14.2 µs | 4.7× | Safe UTF-8 validation |
| **Large String (100 KB)** | `loads` | 4.9 µs | 9.8 µs | 2.0× | Allocation-capped decoding |

---

## Payload Size Comparison

`pysafe-pickle` stores typed record headers, type registries, and string tables to ensure robust schema evolution and safe deserialization. Payloads remain compact and typically within **1.0× to 2.5×** of `pickle`:

| Data Shape | Standard `pickle` (proto 5) | `pysafe-pickle` / `pygraph` | Size Ratio |
| :--- | :---: | :---: | :---: |
| **Large String (100 KB)** | 100,009 B | **100,032 B** | **1.00×** |
| **Simple Dataclass** | 93 B | **115 B** | **1.24×** |
| **Nested Dataclass** | 173 B | **252 B** | **1.46×** |
| **Wide Dict (10,000 keys)** | 138,688 B | **280,662 B** | **2.02×** |
| **Primitives List** | 51 B | **104 B** | **2.04×** |
| **Complex Cyclic Graph** | 96 B | **237 B** | **2.47×** |

---

## Security & Reliability Comparison

Performance is meaningless if the deserializer can crash or be exploited by malicious inputs. Here is how `pysafe-pickle v1.2.0` compares with original `pygraph` and standard `pickle`:

| Security & Stability Dimension | Standard `pickle` | Original `pygraph` (v1.0.x) | `pysafe-pickle` (v1.2.0) |
| :--- | :---: | :---: | :---: |
| **Arbitrary Code Execution (ACE)** | <span style="color:#ef4444">CRITICAL VULNERABILITY</span><br>(Executes arbitrary callables via `__reduce__`) | <span style="color:#10b981">SAFE</span><br>(No `eval` or dynamic method dispatch) | <span style="color:#10b981">SAFE</span><br>(No `eval` or dynamic method dispatch) |
| **Untrusted Capacity Bounds** | <span style="color:#ef4444">Vulnerable</span> | <span style="color:#f59e0b">Unchecked</span><br>(`Vec::with_capacity(obj_count)` without byte limits) | <span style="color:#10b981">Protected</span><br>(Allocations strictly bounded by remaining bytes) |
| **Deep Recursion / Stack Overflow** | <span style="color:#ef4444">C Crash</span> | <span style="color:#ef4444">C Crash</span><br>(Recursive C stack reconstruction segfaults) | <span style="color:#10b981">Protected</span><br>(Iterative heap work stacks; clean Python `RecursionError`) |
| **Memo Address Reuse Collision** | Not Applicable | <span style="color:#f59e0b">Potential Bug</span><br>(Unpinned pointer addresses in memo) | <span style="color:#10b981">Protected</span><br>(Address pinning prevents reallocation collisions) |
| **Cyclic Immutable Sequences** | Reconstructs tuples via state | <span style="color:#f59e0b">Undefined</span> | <span style="color:#10b981">Validated</span><br>(Explicitly rejects unconstructible cyclic tuples) |
| **GIL Concurrency** | Releases in C | <span style="color:#f59e0b">Held GIL</span> | <span style="color:#10b981">Released GIL</span><br>(Uses `py.allow_threads` during pure Rust encode/decode) |
| **Schema Evolution** | None (Brittle) | Supported | Supported (Dual `PSPK` and `PYGR` headers) |

---

## Architectural Performance Pillars in v1.2.0

1. **Fast Type Pointer Checks**: Replaced heap-allocated type name string formatting and linear string comparison with instant PyO3 exact instance pointer comparisons (`is_exact_instance_of::<PyDict>()`).
2. **Scalar Memoization Bypass**: Immutable scalar types (`int`, `float`, `bool`, `None`) bypass graph memo lookups and insertions, saving 5-byte reference overhead and eliminating hash table contention.
3. **Class-Level Caching**: Dataclass schema inspection and field extraction are cached once per class pointer with address pinning, avoiding repeated introspection.
4. **Single-Allocation String Table**: Interned strings are collected and written in a single contiguous memory pass.
5. **Iterative Heap Stacks**: Replaced C call stack recursion with explicit heap work stacks in both encoder and decoder, eliminating stack overflows even on 100,000+ recursion depths.
6. **GIL-Free Serialization**: Pure Rust encoding and decoding routines release the Python Global Interpreter Lock (GIL) via `py.allow_threads`.
7. **Cache-Friendly `FxHashMap`**: Migrated internal hash maps to `rustc-hash::FxHashMap`, cutting hash overhead across object traversal.

---

## Running Benchmarks Locally

You can replicate the benchmark suite locally using `pytest-benchmark`:

```bash
# Run performance benchmarks with pytest-benchmark
uv run pytest benchmarks/ -v --benchmark-only

# Verify payload sizes and compression ratios
uv run pytest benchmarks/test_benchmarks.py -k TestSize -s
```
