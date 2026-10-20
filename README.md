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
- [x] M4: modification tools
- [x] M5: organization
- [x] M6: files and export
- [x] M7a: Styles and Scenes
- [x] M7b: section planes and shadows

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

The kernel welds/splits intersections and overlaps, derives planar regions with holes, triangulates them, and heals erased coplanar boundaries. Coordinates are f64 meters with a shared 1e-7 m tolerance and a supported ±1e6 m coordinate range. Local transactions, delta history and dirty mesh uploads handle edits; see [kernel measurements](docs/KERNEL_BENCHMARK.md). Extrusion and modification tools use the same command/history boundary.

## Large-mesh baseline

```sh
WGPU_BACKEND=opengl cargo run --release -p cindral-app --example renderer_benchmark
```

This renders a 500,000-triangle shaded heightfield with depth at 1280×800. See [measured results and limits](docs/RENDERER_BENCHMARK.md). It measures a static GPU workload, not complete editor frame time.

## Drawing (M3)

Choose Line (L), Rectangle (R), Circle (C), Arc (A) or Polygon (P). Click successive anchors, or type a measurement and press Enter after the first anchor. Examples: `5m`, `120cm`, `2m,3m` for rectangles, `90deg` after an arc start. Arc clicks specify center, start and end direction. Escape cancels; Ctrl+Z / Ctrl+Shift+Z undo/redo. Arrow Right/Left/Up lock X/Y/Z; Down unlocks. The sidebar chooses XY/XZ/YZ construction planes and curve segment counts. Hover geometry for snap markers and guides.

Inference and tools are CPU-only libraries with independent tests. To capture the shaded demo and exercise incremental GPU buffer growth, append `--incremental` after the capture path in the capture example command.

## Modify (M4)

Choose a modify tool in the sidebar. Select by clicking, drag left-to-right to enclose geometry, drag right-to-left to cross it, or triple-click for connected geometry. Shift adds to selection; amber outlines show the selection.

Drag Push/Pull, Move/Copy, Rotate or Scale for a live preview; release commits one undo step, and Escape cancels. Alternatively click an anchor, type a value and press Enter: `2m`, `-50cm`, `90deg`, `2x` or `150%`. Rotation/scaling use the selection center as the pivot. Arrow locks and inference guide placement. Nonplanar deformations are rejected atomically.

Offset makes an outer boundary inset/outset. Follow Me uses selected ordered path edges and a selected profile, starting normal to the profile; a cursor or typed normal segment also works. Eraser heals erased edges. Paint Bucket applies the chosen face color. Tape Measure displays distances and creates guide lines without changing geometry or undo history.

To validate preview/cancel/commit and capture a painted through-hole with selection highlighting:

```sh
WGPU_BACKEND=opengl cargo run -p cindral-app --example capture_viewport -- /tmp/cindral-modify.ppm --modify
```

See [kernel benchmark](docs/KERNEL_BENCHMARK.md) for measured update costs and [architecture](docs/ARCHITECTURE.md) for solid/sweep limits.

## Organization (M5)

Select connected geometry and use **Group selection** or **Component** in the Outliner. Components share a definition; **Unique** breaks the selected link. Double-click a container to edit it, and use Escape, Exit, or click outside to leave. Tools and snaps use that context's local geometry; surrounding instances are dimmed. Global undo/redo reverts shared edits across every instance.

The Outliner supports rename, hide, duplicate and drag-to-reparent. Tags control container visibility; loose geometry remains untagged. Create RGB/opacity materials in the sidebar, select one for Paint Bucket or **Paint selected**; load PNG/JPEG textures through the Materials panel. Use Move/Copy, Rotate and Scale on containers for live matrix previews and one release commit.

See [component measurements](docs/COMPONENT_BENCHMARK.md) and [connected-solid measurements](docs/CONNECTED_SOLID_BENCHMARK.md). Reproduce the native organization capture with `--organization` on the `capture_viewport` example. Native archive version 2 stores document schema 1, embedded textures and shared definitions.


## Files and interchange (M6)

Use **File** to create/open/save `.cdraft`, import OBJ, export OBJ/STL/glTF 2, and recover autosaves. Ctrl+O opens, Ctrl+S saves and Ctrl+Shift+S uses Save As. Opening fits visible geometry and starts fresh undo/redo history at root. Manual saves and recovery snapshots use atomic ZIP writes; changed committed values autosave every 30 seconds in the background. Startup offers retained crash snapshots.

Materials can embed PNG/JPEG images; original bytes survive native round trips, while the viewport uses 256×256 planar previews. OBJ/STL flatten visible instances in metres/Z-up; glTF shares meshes with nested placement nodes and converts to metres/Y-up. See [format, migration, recovery and interchange limits](docs/FILE_FORMAT.md) and [save/load measurements](docs/FILE_BENCHMARK.md).

```sh
cargo run --release -p cindral-io --features bench-fixtures --example file_benchmark
cargo run -p cindral-io --example make_texture_demo -- /tmp/textured.cdraft
CINDRAL_CAPTURE_DRAFT=/tmp/textured.cdraft WGPU_BACKEND=opengl cargo run -p cindral-app --example capture_viewport -- /tmp/textured.ppm
```

Section planes and geographic shadows are available in the Styles & Scenes panel. See [controls, validation and release measurements](docs/SECTION_SHADOW_BENCHMARK.md). This completes the planned M0–M7b feature milestones.
