# M5 component kernel and renderer benchmark

Measured 2026-10-20 on native Wayland, Rust 1.98.1 release, wgpu 27 OpenGL, AMD Radeon Vega 11 / Mesa 26.2.3. Three final sequential runs after compilation/tests finished. [Raw reports](benchmarks/2026-10-20-m5-components-vega11.txt) retain both final runs and earlier exploratory runs made during compilation.

```sh
cargo build --release -p cindral-app --example component_benchmark --features kernel-benchmark
WGPU_BACKEND=gl target/release/examples/component_benchmark
```

The scene has 5,000 placements of one six-face closed box definition: 60,000 submitted triangles. An empty root is a separate definition, without draws. CPU setup uses validated schema DTOs. Timed edits use actual `DocumentHistory`, `SceneCache` and `SceneRenderer` paths. The shared edit changes one definition; the placement preview changes one occurrence. Undo/redo times are for local deltas routed through the global timeline. Picking traverses all 5,000 visible placements with shared bounds and ray/screen filtering.

| Operation (ms) | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Document setup (one-time) | 6.927 | 7.493 | 5.564 |
| Initial traversal/CPU mesh+instances | 2.023 | 3.449 | 1.920 |
| Shared face edit | 0.479 | 0.668 | 0.488 |
| Dirty definition mesh preparation | 0.050 | 0.072 | 0.050 |
| Shared edit undo | 0.059 | 0.042 | 0.041 |
| Shared edit redo | 0.039 | 0.041 | 0.039 |
| One placement preview | 0.007 | 0.007 | 0.007 |
| Instance-batch CPU preparation | 1.295 | 1.499 | 1.277 |
| Transformed scene pick | 1.058 | 1.548 | 1.035 |
| Initial GPU upload + completion | 2.137 | 2.099 | 1.785 |
| Shared edit GPU upload + completion | 0.271 | 0.370 | 0.598 |
| Placement GPU update + completion | 0.369 | 0.446 | 0.771 |

Shared edit + dirty mesh + completed upload totals **0.800 / 1.110 / 1.136 ms**. Placement preview + CPU batching + completed upload totals **1.671 / 1.952 / 2.055 ms**. These measured paths are below 16 ms. Geometry is uploaded once per definition; the shared edit has one dirty definition and no matrix upload, while placement has zero geometry writes. GPU buffers allocate 1,605,088 bytes including capacity headroom. Stable instance ordering permits a single changed placement to upload only one 160-byte record; dimming/count/order changes can affect larger ranges.

The native benchmark renders to 1280×800 RGBA8/depth32 offscreen targets using the production scene pipelines. It runs 20 warmup and 100 measured submissions, timing encoder creation through submission and device completion. Each nonempty definition uses three instanced draws (opaque faces, edges, transparent faces); opaque faces are also submitted to the transparent shader pass, where they are discarded.

| Offscreen frame (ms) | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Mean | 1.472 | 1.592 | 1.556 |
| p95 (nearest rank) | 2.010 | 1.698 | 1.711 |
| Draw calls | 3 | 3 | 3 |

This is not full editor latency or achieved presented FPS: UI/Outliner layout, inference, grid, overlays, swapchain compositing/vsync and continuous drag scheduling are excluded. Picking is measured separately. Transparent surfaces are not sorted per triangle. Native `capture_viewport --organization` separately verifies actual scene compositing, opacity, nested/mirrored placements and context dimming. No cross-platform or other GPU measurements were made. Earlier runs during compilation included a 20.354 ms completed shared upload; the final quiet runs do not establish a hard latency guarantee under desktop contention.

The prior [500,000-triangle static baseline](RENDERER_BENCHMARK.md) is a different workload. [Connected-solid topology edits](CONNECTED_SOLID_BENCHMARK.md) remain far beyond 16 ms despite fast component placement.
