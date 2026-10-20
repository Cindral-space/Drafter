# Kernel and buffer-update benchmark

Measured 2026-10-20 on Linux/native Wayland, Radeon Vega 11 (radeonsi/raven, Mesa 26.2.3), wgpu OpenGL, stable Rust 1.98.1, release profile. [Raw reports](benchmarks/2026-10-20-kernel-vega11.txt) retain every sample.

## Workload and method

The deterministic fixture contains 10,000 separated unit squares, each divided diagonally: **40,000 vertices, 50,000 edges, 20,000 triangular faces**. Direct known-topology construction is outside timed sections; a small equivalent fixture has a full validation test. One added segment from (0,0,0) to (1,0.5,0) splits one existing face, producing 20,001 triangles. Undo and redo immediately follow that edit.

The legacy global implementation was measured once at commit `bc7fbf2`: the edit includes global face reconstruction and intersection validation; undo/redo copy full snapshots. The final implementation was measured in three separate runs. Setup is excluded: legacy 15 ms; indexed setup 141–158 ms. No warm-cache distribution or confidence interval is claimed.

| Operation | Global/snapshot baseline (one run) | Local/delta implementation (3 runs, range) |
| --- | ---: | ---: |
| One topology edit | 203,080.039 ms | 0.124–0.137 ms |
| Undo | 10.169 ms | 0.021–0.030 ms |
| Redo | 8.031 ms | 0.017–0.034 ms |
| Triangulate every face | 18.979 ms | 17.056–17.301 ms |

The GPU harness separately edits the same fixture, then compares full mesh rebuilding/new buffer allocation against updating the existing cache and buffers. Initial mesh construction/upload happens before timing. GPU upload times include submission and waiting for completion; CPU mesh times include triangulation and serialization into vertex bytes.

| Mesh/buffer operation after edit | Full rebuild (3 runs, range) | Dirty update (3 runs, range) |
| --- | ---: | ---: |
| CPU mesh preparation | 59.720–73.657 ms | 0.036–0.040 ms |
| GPU buffer allocation/upload or update, completed | 2.065–7.606 ms | 0.362–1.630 ms |
| Bytes submitted | 4,560,204 | 360 |
| Allocated buffer capacity | 9,120,408 bytes | 9,120,000 bytes |

The edit reports three affected face IDs and three edge IDs (including replaced/deleted entities). Spare capacity permits range writes without reallocating; growth copies retained ranges into larger buffers.

## Decision and limits

The global edit exceeds the requested ~16 ms gate by a wide margin. Local cavity reconstruction, spatial/adjacency indices, inverse entity deltas and partial GPU uploads are therefore implemented in M3, before M4. The measured local edit and upload fit the budget for this fixture. Full rebuilds still exceed it.

Separated squares favor local edits; this is not a guarantee for arbitrary connected models or edits affecting many faces. Connected-grid differential tests check correctness against global reconstruction, not large connected-model latency. CPU inference, selection, rendering frames, UI, frame scheduling and future push/pull are excluded. Upload completion timings are not frame-rate measurements. Root geometry is supported; per-group caches and component instancing belong to M5. The static 500k-triangle rendering benchmark remains a separate workload.

## Reproduce

```sh
cargo run --release -p cindral-core --features bench-fixtures --example kernel_benchmark
WGPU_BACKEND=opengl cargo run --release -p cindral-app --features kernel-benchmark --example kernel_gpu_benchmark
```

For the original global/snapshot CPU result, run the first command in a separate checkout of `bc7fbf2`; allow several minutes. Benchmarks are explicit examples, not timed CI tests.

## M4 rerun

Same host, release profile and OpenGL adapter, measured after M4 integration. [All six raw reports](benchmarks/2026-10-20-m4-kernel-vega11.txt) are retained. The original 50k-edge/20k-face line edit still runs first. For interactive previews, setup then adds an isolated closed 2×2×1 m box outside the tile array. Its top face is pulled by 0.11–0.40 m in 30 absolute provisional updates, each replacing the previous delta. Final commit adds exactly **one undo entry** in every run.

CPU preview fixture: 50,012 edges / 20,006 faces before the pull. GPU preview fixture: 50,014 edges / 20,007 faces because it retains the earlier line edit. A complete mesh cache and buffers are initialized outside the preview loop. Only the isolated closed box is edited; this is a large document containing a small affected solid, not a benchmark of one 20k-face connected solid.

| Original CPU operation | M4, three runs |
| --- | ---: |
| Line edit | 0.151–0.217 ms |
| Undo | 0.021–0.027 ms |
| Redo | 0.017–0.019 ms |
| Full triangulation | 16.421–18.322 ms |
| Closed-box push/pull preview, per-run mean | 0.577–0.595 ms |
| Closed-box push/pull preview, worst sample | 0.819 ms |
| Whole-box Move preview | 0.204–0.213 ms |
| Whole-box Rotate preview | 0.130–0.135 ms |
| Whole-box Scale preview | 0.129–0.135 ms |

Move/rotate/scale are individual successful previews of all box vertices, followed by cancellation. Push/pull has 30 samples per run; the other transforms have one per run, so these figures are not latency distributions.

| Native closed-box update stage (30 samples/run) | Per-run mean range | Worst sample across 3 runs |
| --- | ---: | ---: |
| Kernel provisional delta | 0.599–0.842 ms | 1.574 ms |
| Dirty CPU mesh preparation | 0.026–0.038 ms | 0.115 ms |
| Completed GPU range upload | 0.269–0.471 ms | 1.457 ms |
| Combined update path | **0.908–1.363 ms** | **2.368 ms** |

**The measured interactive update path remains below 16 ms** on this fixture. Full mesh rebuilds still take 50.239–51.844 ms (before adding the box), while the original single-face line edit's partial mesh preparation takes 0.029–0.042 ms and completed range upload 0.333–0.416 ms.

These times exclude inference candidate scans, picking, egui, actual drawing and frame scheduling. They establish update cost, not full-editor frame rate. Push/pull evaluates affected connected shells for Boolean operations; a very large connected solid may exceed 16 ms even though untouched boundary IDs and GPU ranges are retained. That workload remains unmeasured. Conservative complexity limits abort the operation rather than commit incomplete Boolean output. The earlier static 500k-triangle raster baseline is independent.

The reproduction commands above now include the M4 measurements. Native `capture_viewport --modify` additionally exercises preview, cancel, committed through-hole, face painting and selection rendering; its screenshot is visual validation rather than a timing sample.

## M5 organization rerun

The new [5,000-component kernel/renderer workload](COMPONENT_BENCHMARK.md) measures the document/global-history/definition-cache path: shared geometry edit 0.479–0.668 ms, undo 0.041–0.059 ms, redo 0.039–0.041 ms, and dirty mesh preparation 0.050–0.072 ms. Shared edit + mesh + completed GPU upload totals 0.800–1.136 ms across three final quiet runs. One occurrence placement + CPU batching + completed GPU update totals 1.671–2.055 ms. These are below 16 ms for that workload and exclude full UI frame scheduling.

The new [21,600-face connected solid](CONNECTED_SOLID_BENCHMARK.md) changes the assessment for large connected topology: single-face push/pull and raw whole-solid Move both exceeded a 60-second process deadline. Neither meets 16 ms. The earlier disconnected-tile/small-box fixture must not be generalized to connected shells. Matrix-only group placement remains fast, with costs/limits reported separately.
