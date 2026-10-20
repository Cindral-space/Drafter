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
