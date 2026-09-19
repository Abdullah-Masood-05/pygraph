# Performance & Benchmarks

`pysafe-pickle` delivers competitive serialization performance powered by its Rust core while strictly maintaining zero arbitrary code execution.

---

## Benchmark Comparison (pysafe-pickle vs Standard pickle)

Benchmarks were collected using `pytest-benchmark` across diverse data shapes:

| Data Shape | Operations/Sec (`pysafe-pickle`) | Operations/Sec (`pickle`) | Notes |
| :--- | :---: | :---: | :--- |
| **Primitives (ints, floats, bools)** | ~216,000 ops/sec | ~250,000 ops/sec | Near parity |
| **Nested Dataclasses** | ~52,000 ops/sec | ~54,000 ops/sec | Full schema tracking |
| **Complex Graphs (with cycles)** | ~122,000 ops/sec | ~95,000 ops/sec | Fast Rust memo table |
| **Wide Dictionaries (100 keys)** | ~17,800 ops/sec | ~18,500 ops/sec | Deduplicated string table |
| **Large Strings (100KB)** | ~32,800 ops/sec | ~34,000 ops/sec | UTF-8 zero-copy slices |

---

## Running Benchmarks Locally

You can replicate the benchmark suite using `pytest-benchmark`:

```bash
uv run pytest benchmarks/ -v --benchmark-only
```
