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

## Profiling diagnosis before algorithm changes

Instrumentation-only baseline `44c674c`, release, tracing enter/close spans (`CINDRAL_PROFILE=1`); `perf` and `flamegraph` are unavailable in this environment. Each n60 trace was bounded by a separate 25-second process deadline. The following measurements were recorded **before changing any edit algorithm**.

| Workload/phase | Measured wall time | Interpretation |
| --- | ---: | --- |
| Move, n60: connected traversal | 12.6 ms | Already nearly the whole preview budget |
| Move, n60: patch copies | 60.0 ms | Whole 21,600-face/43,200-edge shell copied twice |
| Move, n60: transform arithmetic | 24.7 ms | Repeated map lookups/plane reconstruction; above preview budget alone |
| Move, n60: validation edge-pairs | Unfinished at process deadline | 43,200 edges imply 933,098,400 unordered pairs |
| Move, n12: validation edge-pairs | 215 ms of 235.669 ms total edit | Dominant completed sample; 1,728 edges / 1,492,128 pairs |
| Push/pull, n60: connected traversal | 33.6 ms | Local prism query expands to entire connected shell |
| Push/pull, n60: patch copies | 50.8 ms | Whole-shell copying precedes actual extrusion |
| Push/pull, n60: orientation | Unfinished at process deadline | Source confirms each visited face scans every edge's incidence list (F×E) |
| Push/pull, n12: orientation | 12.4 ms | 864-face sample completes |
| Push/pull, n12: surface reconstruction | 1.92 s of 1,967.412 ms total edit | Dominant completed sample; legacy code reinserts all polygon edges, scans seams against all polygons, heals and rebuilds globally |

The spatial cache exists, but raw validation does not use it for pair rejection; extrusion discards locality by expanding the candidate cavity to the full connected shell. History already stores deltas, not snapshots, but building those deltas first copies/diffs the whole selected shell. Source-derived additional suspects inside reconstruction (not yet individually timed in this table) are linear vertex welding, all-edge segment intersections, seam polygon scans and global healing. These are distinguished from measured timings rather than assumed to be the bottleneck. Detailed reconstruction spans are being collected. Argon High/high is reviewing the measurements and redesign in a separate worktree; the rejected High/max effort combination performed no analysis and was corrected.
