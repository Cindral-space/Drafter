# Large-mesh renderer benchmark

Implements the contract in `ARCHITECTURE.md` ("Large-mesh benchmark contract"). It is a **simple one-draw static baseline**, not a measurement of full editor performance. Measured results are recorded below; full raw reports are retained alongside this document.

## Commands

```sh
# Release build (use this for numbers). This environment is native Wayland; use the OpenGL backend.
WGPU_BACKEND=opengl cargo run --release -p cindral-app --example renderer_benchmark

# Debug build (numbers are not representative)
WGPU_BACKEND=opengl cargo run -p cindral-app --example renderer_benchmark

# CPU-only mesh/statistics tests
cargo test -p cindral-app --example renderer_benchmark
```

The example opens a small eframe window only to obtain the adapter/device through `CreationContext::wgpu_render_state`, runs once, prints the report to stdout and closes itself. It exits with status 1 and a message on stderr if setup, validation, `device.poll`, readback or raster verification fails.

## Workload

- Static indexed heightfield, 500×500 cells: 501×501 = 251,001 vertices, 500,000 triangles, 1,500,000 `u32` indices (6,000,000 bytes). Vertices are interleaved position + analytic unit normal (24 bytes).
- Triangles are CCW seen from +Z with back-face culling; Lambert + ambient shading in `renderer_benchmark.wgsl`.
- Offscreen 1280×800 `Rgba8Unorm` colour + `Depth32Float` (no MSAA), one `draw_indexed` per submission.
- Camera: `cindral_render::camera::Camera` default (iso, perspective) fitted with `zoom_extents` to the mesh bounds.

## Methodology

1. Generate the mesh and create all GPU resources (untimed). An untimed verification render is copied to a buffer and read back; the fraction of pixels differing from the clear colour is reported, and the run fails below 1%.
2. 20 warmup submissions (untimed), then 100 measured submissions.
3. Each sample covers `std::time::Instant` from command-encoder creation through encoding, `queue.submit`, and `device.poll(Wait { submission_index })` returning. Generation, upload, readback, UI and vsync are excluded.
4. Reported: adapter/backend, dimensions, triangle/vertex counts, index memory, build profile, mean/p50/p95 (nearest rank)/max in ms, frames/s and Mtriangles/s at the mean, and ratio to the 16.67 ms (60 Hz) reference.
5. wgpu validation errors are captured with an error scope around setup and the timed loop.

## Limits

- One draw call, static buffers, no picking, instancing, dirty rebuilds, UI compositing, or egui/swapchain presentation; the grid and editor overlays are not drawn.
- Wall time includes driver submission and poll latency, so it overstates pure GPU time and is sensitive to the backend (OpenGL here).
- Build profile is derived from `debug_assertions`; a custom profile may be mislabelled.
- Readback checks only that pixels were rasterized, not image correctness.
- No Windows/macOS or Vulkan/Metal/DX12 runs were performed by the author.

## Measured baseline — 2026-10-20

Native Wayland; release build with stable Rust 1.98.1; wgpu 27 OpenGL backend. Adapter: AMD Radeon Vega 11 Graphics (radeonsi, raven, ACO, DRM 3.64, Linux 7.2.7-arch1-1); Mesa 26.2.3. Three sequential runs after compilation/test processes completed. Each used the fixed workload and methodology above; GPU readback reported 35.1% covered pixels on every run.

| Run | Mean ms | p50 ms | p95 ms | Max ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.460 | 1.419 | 1.712 | 1.865 |
| 2 | 1.628 | 1.494 | 2.028 | 3.579 |
| 3 | 1.528 | 1.502 | 1.704 | 2.085 |

The static one-draw baseline stays below the 16.67 ms reference in these runs. These are encode/submit/completion wall times for the offscreen mesh, **not achieved full-editor frame rates**. UI, presentation, picking, per-group draw calls, rebuilds, textured materials, and component instancing still need representative end-to-end benchmarks as those features arrive. No 60 fps editor guarantee follows from these measurements.

[Raw reports](benchmarks/2026-10-20-vega11-opengl.txt) include adapter details, counts, memory, coverage, throughput and timing. The benchmark's five CPU mesh/statistics tests are included in `cargo test --workspace` through the example target's `test = true` setting.

## M5 component scene

The production instanced renderer now has a separate [5,000-component benchmark](COMPONENT_BENCHMARK.md): one box definition, 60,000 submitted triangles, three instanced draws, mean offscreen frames 1.472–1.592 ms and p95 1.698–2.010 ms across three final runs. It measures partial geometry/matrix uploads separately and retains the same distinction between offscreen submissions and full editor FPS. The original 500,000-triangle static baseline above is unchanged.
