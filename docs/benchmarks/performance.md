# Performance & Benchmarks

`pysafe-pickle` is a Rust engine (PyO3) that serializes Python object graphs with **zero arbitrary code execution (ACE)**. `dumps` is faster than the C `pickle` module (protocol 5) on almost every benchmark shape we measure. `loads` is mixed: faster on dataclass-heavy data, **slower on some container-heavy shapes**. Payloads are **larger** than pickle's, typically **1.0x to 2.8x**.

The timing tables below were measured on `pysafe-pickle` **v1.3.2** against stdlib `pickle` (protocol 5), using the 16 data shapes in `benchmarks/bench_data.py`. Each figure is the best of 5 `timeit` repeats, in microseconds (µs). Absolute values depend on your CPU and Python version; the ratios are what matter.

::: info v1.4.0
The encoding of these shapes did not change in v1.4.0: payload sizes are byte-for-byte identical, and a re-run of the same script on v1.4.0 gave the same ratios within measurement noise, with one exception: `dumps` of 1 MB of `bytes` measured about equal to pickle (0.99x to 1.1x). The new stdlib types, `classes=` and the pickle reader are not part of these tables. See [HMAC overhead](#hmac-overhead) for the cost of signing.
:::

---

## Benchmark: pysafe-pickle vs Standard Python pickle

Python's built-in `pickle` module (implemented in C as `_pickle`) is fast, but it is **not safe** for untrusted data because it executes arbitrary Python code (`__reduce__`). Ratios are `pickle time / pysafe-pickle time`: "faster" means pysafe-pickle is faster, "slower" means pickle is faster.

### `dumps`

| Data Shape | Standard `pickle` (proto 5) | `pysafe-pickle` (v1.3.2) | Ratio |
| :--- | :---: | :---: | :---: |
| **Primitives** | 0.91 µs | 0.53 µs | 1.7× faster |
| **Nested list (depth 5)** | 1.13 µs | 0.57 µs | 2.0× faster |
| **Nested list (depth 20)** | 1.92 µs | 0.93 µs | 2.1× faster |
| **Wide Dict (100 keys)** | 6.49 µs | 3.93 µs | 1.7× faster |
| **Wide Dict (10,000 keys)** | 702 µs | 399 µs | 1.8× faster |
| **Simple Dataclass** | 2.19 µs | 0.86 µs | 2.5× faster |
| **Nested Dataclass** | 3.43 µs | 1.38 µs | 2.5× faster |
| **Complex Graph (shared refs)** | 1.31 µs | 0.87 µs | 1.5× faster |
| **Large String (100 KB)** | 2.80 µs | 2.31 µs | 1.2× faster |
| **Mixed Container** | 1.66 µs | 1.15 µs | 1.4× faster |
| **List of Records (1,000)** | 263 µs | 234 µs | 1.1× faster |
| **List of Dataclasses (1,000)** | 1,353 µs | 231 µs | 5.8× faster |
| **Ints (10,000)** | 150 µs | 122 µs | 1.2× faster |
| **Floats (10,000)** | 198 µs | 130 µs | 1.5× faster |
| **Short Strings (10,000)** | 505 µs | 263 µs | 1.9× faster |
| **Bytes (1 MB)** | 258 µs | 246 µs | 1.05× faster |

### `loads`

| Data Shape | Standard `pickle` (proto 5) | `pysafe-pickle` (v1.3.2) | Ratio |
| :--- | :---: | :---: | :---: |
| **Primitives** | 0.54 µs | 0.58 µs | 1.07× slower |
| **Nested list (depth 5)** | 0.75 µs | 0.76 µs | about equal |
| **Nested list (depth 20)** | 1.80 µs | 1.90 µs | 1.06× slower |
| **Wide Dict (100 keys)** | 8.01 µs | 9.85 µs | 1.2× slower |
| **Wide Dict (10,000 keys)** | 809 µs | 972 µs | 1.2× slower |
| **Simple Dataclass** | 1.41 µs | 0.77 µs | 1.8× faster |
| **Nested Dataclass** | 2.69 µs | 1.38 µs | 1.9× faster |
| **Complex Graph (shared refs)** | 0.97 µs | 1.26 µs | 1.3× slower |
| **Large String (100 KB)** | 4.84 µs | 5.12 µs | 1.06× slower |
| **Mixed Container** | 1.39 µs | 1.84 µs | 1.3× slower |
| **List of Records (1,000)** | 359 µs | 326 µs | 1.1× faster |
| **List of Dataclasses (1,000)** | 898 µs | 408 µs | 2.2× faster |
| **Ints (10,000)** | 190 µs | 238 µs | 1.25× slower |
| **Floats (10,000)** | 273 µs | 231 µs | 1.2× faster |
| **Short Strings (10,000)** | 343 µs | 340 µs | about equal |
| **Bytes (1 MB)** | 259 µs | 235 µs | 1.1× faster |

::: tip Summary
- **`dumps`**: faster than pickle on all 16 shapes in v1.3.2, from about 1.05× (raw bytes) to 5.8× (lists of dataclasses). In v1.4.0, raw bytes are about equal.
- **`loads`**: faster on dataclasses, lists of records, floats and bytes; roughly equal on small nested lists and short strings; up to about 1.3× slower on wide dicts, shared-reference graphs, mixed containers and int lists.
- Pickle is not a like-for-like competitor: it will run arbitrary code embedded in a payload, while `pysafe-pickle` validates every record, bounds every allocation and never calls user-controlled callables.
:::

---

## Payload Size Comparison

`pysafe-pickle` stores typed record headers, type registries, and string tables to ensure robust schema evolution and safe deserialization. Payloads are **larger than pickle's** on every shape except large opaque data, where they are equal:

| Data Shape | Standard `pickle` (proto 5) | `pysafe-pickle` (v1.3.2) | Size Ratio |
| :--- | :---: | :---: | :---: |
| **Large String (100 KB)** | 100,009 B | 100,032 B | 1.00× |
| **Bytes (1 MB)** | 1,048,585 B | 1,048,604 B | 1.00× |
| **Simple Dataclass** | 93 B | 115 B | 1.24× |
| **List of Dataclasses (1,000)** | 67,900 B | 88,446 B | 1.30× |
| **Floats (10,000)** | 90,043 B | 130,028 B | 1.44× |
| **Nested Dataclass** | 173 B | 252 B | 1.46× |
| **Ints (10,000)** | 39,778 B | 69,900 B | 1.76× |
| **List of Records (1,000)** | 44,680 B | 89,942 B | 2.01× |
| **Wide Dict (10,000 keys)** | 138,688 B | 280,662 B | 2.02× |
| **Primitives List** | 51 B | 104 B | 2.04× |
| **Mixed Container** | 155 B | 343 B | 2.21× |
| **Complex Graph (shared refs)** | 96 B | 217 B | 2.26× |
| **Short Strings (10,000)** | 78,933 B | 178,918 B | 2.27× |
| **Wide Dict (100 keys)** | 1,106 B | 2,554 B | 2.31× |
| **Nested list (depth 5)** | 32 B | 79 B | 2.47× |
| **Nested list (depth 20)** | 77 B | 214 B | 2.78× |

If payload size matters more than safety features, compress the output (for example with `zlib` or `zstd`) or use pickle for trusted data. A signed payload (`hmac_key=`) is 32 bytes larger.

---

## HMAC Overhead {#hmac-overhead}

Signing and verifying add one HMAC-SHA256 pass over the payload. On the machine used above (v1.4.0) that was roughly **1 ms per MB** of payload for each of `dumps` and `loads`: noticeable for multi-megabyte blobs, negligible for small records.

---

## Security & Reliability Comparison

Performance is meaningless if the deserializer can crash or be exploited by malicious inputs. Here is how `pysafe-pickle v1.4.0` compares with standard `pickle`:

| Security & Stability Dimension | Standard `pickle` | `pysafe-pickle` (v1.4.0) |
| :--- | :---: | :---: |
| **Arbitrary Code Execution (ACE)** | <span style="color:#ef4444">CRITICAL VULNERABILITY</span><br>(Executes arbitrary callables via `__reduce__`) | <span style="color:#10b981">SAFE</span><br>(No `eval` or dynamic method dispatch) |
| **Untrusted Capacity Bounds** | <span style="color:#ef4444">Vulnerable</span> | <span style="color:#10b981">Protected</span><br>(Allocations strictly bounded by remaining bytes) |
| **Malformed Input Validation** | <span style="color:#ef4444">Executes opcodes as it reads</span> | <span style="color:#10b981">Validated</span><br>(Every child, record, string and type index is bounds-checked before objects are built) |
| **Deep Recursion / Stack Overflow** | <span style="color:#ef4444">C Crash</span> | <span style="color:#10b981">Protected</span><br>(Iterative heap work stacks; clean Python `RecursionError`) |
| **Cyclic Immutable Sequences** | Reconstructs tuples via state | <span style="color:#10b981">Validated</span><br>(Explicitly rejects unconstructible cyclic tuples) |
| **Tamper Detection** | None | <span style="color:#10b981">HMAC-SHA256</span><br>(Verified before decoding when `hmac_key` is given) |
| **GIL Concurrency** | Holds the GIL | <span style="color:#10b981">Partly released</span><br>(Validation and HMAC for inputs of 64 KiB+, final output assembly for 8 MiB+) |
| **Schema Evolution** | None (Brittle) | Versioned types, migrations run on load with `classes=` |

---

## Architectural Performance Pillars

1. **Breadth-First Streaming Encoder**: The graph walker assigns record ids in breadth-first order and writes records straight into the output buffer, without building an intermediate object tree.
2. **Direct Shared/Cyclic References**: A shared or cyclic object reuses the record id of its first occurrence directly, so no `Reference` records are emitted when encoding. (The decoder still accepts `Reference` records for backward compatibility.)
3. **Per-Type Dataclass Caching on Decode**: Dataclass types are resolved once and cached (class object plus interned field names), then reused across `loads` calls. This is the main reason dataclass-heavy `loads` beats pickle.
4. **String-Object Caching**: Strings that appear more than once in a payload are materialised once and reused as the same Python object. The cache is skipped when no string is shared.
5. **Fast Type Pointer Checks**: Exact PyO3 type pointer comparisons (`is_exact_instance_of::<PyDict>()`) instead of string-based type name checks.
6. **Iterative Heap Stacks**: Explicit heap work stacks in both encoder and decoder remove C call-stack recursion, so extremely deep structures raise a clean error instead of overflowing.
7. **GIL Release for Large Inputs**: Phase-1 validation and HMAC work release the Python Global Interpreter Lock via `py.allow_threads` for inputs of 64 KiB or more; object construction always needs the GIL.
8. **Cache-Friendly `FxHashMap`**: Internal hash maps use `rustc-hash::FxHashMap`, cutting hash overhead during traversal.

---

## Running Benchmarks Locally

You can reproduce the benchmark suite locally with `pytest-benchmark`:

```bash
# Run performance benchmarks with pytest-benchmark
uv run pytest benchmarks/ -v --benchmark-only

# Verify payload sizes
uv run pytest benchmarks/test_benchmarks.py -k TestSize -s
```

The data shapes live in `benchmarks/bench_data.py`. The tables above use plain `timeit` (best of 5) over those same shapes; results vary with hardware and Python version, so run the suite on your own machine before drawing conclusions.
