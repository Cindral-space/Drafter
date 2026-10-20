# M5 large connected solid

Measured 2026-10-20, Rust 1.98.1 release, Linux, on the same Vega 11 machine as the component benchmark. This is a CPU kernel measurement, independent of the GPU adapter.

The fixture is the welded boundary of a cube `[0,2]^3`, with each of six sides subdivided into a 60×60 quad grid: **21,602 vertices, 43,200 edges and 21,600 faces**. Shared edge incidence is two and Euler characteristic is two. Small fixture variants run full kernel validation in tests; the large fixture uses the feature-gated known-topology ingestion path to avoid charging quadratic load validation to edit timing.

```sh
cargo build --release -p cindral-core --example connected_benchmark --features bench-fixtures
timeout 60s target/release/examples/connected_benchmark pushpull 60
timeout 60s target/release/examples/connected_benchmark move 60
```

Each command constructs a fresh fixture and times exactly one edit. Push/pull extrudes one top-grid face outward by 0.1 m; Move translates all raw vertices by 0.1 m in Z. Setup prints and flushes before the edit starts.

| Operation | Setup ms | Result | Under 16 ms? |
| --- | ---: | --- | --- |
| Single-face push/pull | 357.578 | Did not finish before 60 s process deadline; exit 124 | No |
| Whole-solid raw vertex Move | 278.358 | Did not finish before 60 s process deadline; exit 124 | No |

The deadline includes setup and startup. Thus each edit remained unfinished after roughly 59 seconds, rather than having a measured completed duration of 60 seconds. No correctness/undo claim is made for a timed-out result. [Raw output](benchmarks/2026-10-20-m5-connected-vega11.txt) is retained.

Placing the same solid inside a group exercises a different operation: it changes a local matrix without touching its topology. Three runs measured placement preview **0.004–0.007 ms**, commit **0.004–0.007 ms** and undo **0.004–0.008 ms**, excluding initial mesh preparation, scene batching and GPU upload. These are from the final [component workload](COMPONENT_BENCHMARK.md), which also contains the connected-group fixture. Fast matrix placement does not repair the raw geometry-edit bottleneck.

The remaining cost is dense connected-shell Boolean reconstruction and topology validation/index updates for raw transformations. The follow-up plan in [architecture](ARCHITECTURE.md#next-incremental-work-after-the-connected-solid-measurement) targets adjacency/face broad-phase indexes, preservation of untouched shell regions, affine whole-shell fast paths, and bounded/cancellable work. The current synchronous raw edit can stall the UI on this workload; it is a documented limitation, not an interactive-performance pass.
