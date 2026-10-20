# Cindral Drafter

An original Rust 3D modeling application with inference-driven drawing and push/pull modeling. Cindral Drafter uses its own UI and versioned `.cdraft` format. No third-party modeling application code, assets, branding, or file-format SDKs are used.

## Development

Install stable Rust, then run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [architecture](docs/ARCHITECTURE.md) and [agent log](docs/AGENT_LOG.md).

## Milestones

- [x] M0: workspace and checks
- [x] M1: viewport
- [x] M2: geometry kernel
- [x] M3: drawing and inference
- [ ] M4: modification tools
- [ ] M5: organization
- [ ] M6: files and export
- [ ] M7: presentation

## Run the viewport

```sh
cargo run -p cindral-app
```

The native window uses eframe/winit and wgpu. Linux requires a working Vulkan or OpenGL ES driver and a Wayland or X11 display. To select the OpenGL backend explicitly:

```sh
WGPU_BACKEND=opengl cargo run -p cindral-app
```

| Action | Input |
| --- | --- |
| Orbit | Middle mouse drag |
| Pan | Shift + middle mouse drag |
| Zoom around cursor | Scroll over viewport |
| Fit model / construction area | F or Zoom extents |
| Isometric / front / right / top | 0 / 1 / 3 / 7 |
| Toggle projection | 5 |
| All standard views | View sidebar |

Keyboard shortcuts apply while the pointer is over the viewport. Grid spacing is one meter. Extents fits model bounds, or the central 20 × 20 meter area when empty.

For reproducible native GPU validation, run:

```sh
WGPU_BACKEND=opengl cargo run -p cindral-app --example capture_viewport -- /tmp/cindral-viewport.ppm
```

The example captures the rendered window and exits. Use the native display session: forcing X11 in a Wayland session can make EGL choose a different platform from the window.

## Geometry kernel (M2)

`cindral-core::History` owns a read-only `Model`. Draw with `Command::AddEdge` or `AddPolyline`, erase with `EraseEdges`, set soft/smooth flags with `SetEdgeStyle`, and combine actions into an atomic `Batch`. `execute`, `undo` and `redo` return `Result<bool, KernelError>`; failed edits preserve both geometry and history. See the core crate's tested example and [architecture](docs/ARCHITECTURE.md) for the public interface.

The kernel welds/splits intersections and overlaps, derives planar regions with holes, triangulates them, and heals erased coplanar boundaries. Coordinates are f64 meters with a shared 1e-7 m tolerance and a supported ±1e6 m coordinate range. Local transactions, delta history and dirty mesh uploads handle edits; see [kernel measurements](docs/KERNEL_BENCHMARK.md). Extrusion and other modification tools come in M4.

## Large-mesh baseline

```sh
WGPU_BACKEND=opengl cargo run --release -p cindral-app --example renderer_benchmark
```

This renders a 500,000-triangle shaded heightfield with depth at 1280×800. See [measured results and limits](docs/RENDERER_BENCHMARK.md). It measures a static GPU workload, not complete editor frame time.

## Drawing (M3)

Choose Line (L), Rectangle (R), Circle (C), Arc (A) or Polygon (P). Click successive anchors, or type a measurement and press Enter after the first anchor. Examples: `5m`, `120cm`, `2m,3m` for rectangles, `90deg` after an arc start. Arc clicks specify center, start and end direction. Escape cancels; Ctrl+Z / Ctrl+Shift+Z undo/redo. Arrow Right/Left/Up lock X/Y/Z; Down unlocks. The sidebar chooses XY/XZ/YZ construction planes and curve segment counts. Hover geometry for snap markers and guides.

Inference and tools are CPU-only libraries with independent tests. To capture the shaded demo and exercise incremental GPU buffer growth, append `--incremental` after the capture path in the capture example command.
