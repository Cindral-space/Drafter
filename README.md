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

## Modeling workspace

The editor uses a neutral modeling canvas, a theme-aware two-column tool palette, and a flat, resizable **Inspector**. All 16 drawing and modification tools have visible icons and captions; hover for keyboard shortcuts. The **Model** inspector tab contains Outliner, Tags and Materials; **Appearance** contains styles, scenes, sections and shadows. Selection details appear when geometry is selected, and the Outliner opens when a group or component is selected. Close the inspector with **×** beside its tabs; reopen it from the **Inspector** toolbar button or **Window → Model inspector**. The visible **File, Edit, Camera, Draw, Tools, View, Window and Help** menus retain the familiar desktop layout. The toolbar provides Undo/Redo, Fit model, segmented work-plane controls, Inspector and Save; segment counts and paint color appear for their respective tools. **Camera** includes standard views, projection and a construction-grid toggle. The grid starts hidden, with muted world axes and a small orientation indicator. The header shows the document name and Unsaved, Modified or Saved state; recovery autosaves do not mark a draft as saved. Dimensions stay at the bottom right. The native editor starts with Select; **Space** returns to Select.

See the [UI/UX review](docs/UI_UX_REVIEW.md) for findings, changes and validation.

## Preferences

Use **View → Preferences → Light / Dark / System** to change the UI live. System is the default. The existing per-user `cindral-drafter/png-export.json` settings file stores the choice with atomic replacement; missing, corrupt or invalid settings fall back to System. Save failures appear in the status bar and the live choice remains usable.

System follows eframe/winit theme events where available. On Linux, including Wayland, the desktop appearance portal (`org.freedesktop.appearance`, `color-scheme`) is checked off the UI thread every two seconds; native theme events remain the fallback. If neither reports a preference, System uses **Dark**. A portal value of “no preference” also uses the native setting or Dark. Desktop portal availability depends on the installed desktop environment.

UI themes never alter document/scene viewport backgrounds, sky, ground, materials, axis meanings, embedded PNG/JPEG assets or PNG/COLLADA/glTF export colors. Viewport overlay hues retain their meaning and adjust brightness against the document background when necessary. As with other viewport ink, arbitrary geometry or a contrasting sky/ground behind a marker can affect its local contrast.

Hardcoded UI colors replaced (five groups):

- Status bar and measurements: white, gray input fill, pink error fill and red error text.
- Tool hints and toolbar: fixed ink, muted labels, purple accent/selection and pale hover fills.
- Outliner/Tags/Materials inspector: pale panel backgrounds and fixed headings; swatch colors remain document data.
- Dialogs, menus, progress and tooltips: white surfaces, fixed borders, muted/error/warning text; PNG action/quality foregrounds.
- Scene tabs and orientation chrome: inherited selection/hover/text palette and indicator discs; axis hues remain unchanged.

## Run the viewport

```sh
cargo run -p cindral-app
```

If linking fails with `ld.lld: error: undefined hidden symbol`, clear the app's generated build files and retry:

```sh
cargo clean -p cindral-app
cargo run -p cindral-app
```

This resolved a local stale-build failure without changing application source or documents.

The dev profile keeps debug assertions and overflow checks on while optimizing the workspace crates and dependencies, so everyday testing on imported models stays responsive. With [bacon](https://dystroy.org/bacon/), `bacon run` rebuilds and restarts the dev app on change. Use `bacon run-release` to compare against a fully optimized build without dev debug assertions.

The native window launches maximized on every start, including after closing it at a smaller size. The workspace fills the window and adjusts its canvas and panels when resized. Normal window controls remain available.

The native window uses eframe/winit and wgpu. Linux requires a working Vulkan or OpenGL ES driver and a Wayland or X11 display. To select the OpenGL backend explicitly:

```sh
WGPU_BACKEND=opengl cargo run -p cindral-app
```

<!-- shortcuts:start -->
| Category | Action | Binding | During operation |
| --- | --- | --- | --- |
| Tools | Select | Space | Yes |
| Tools | Line | L | Yes |
| Tools | Rectangle | R | Yes |
| Tools | Circle | C | Yes |
| Tools | Arc | A | Yes |
| Tools | Polygon | N | Yes |
| Tools | Push/Pull | P | Yes |
| Tools | Move | M | Yes |
| Tools | Rotate | Q | Yes |
| Tools | Scale | S | Yes |
| Tools | Offset | F | Yes |
| Tools | Eraser | E | Yes |
| Tools | Tape Measure | T | Yes |
| Tools | Paint Bucket | B | Yes |
| Navigation | Orbit | O | Yes |
| Navigation | Pan | H | Yes |
| Navigation | Zoom | Z | Yes |
| Navigation | Fit model | Shift+Z | Yes |
| Navigation | Iso | 0 | Idle only |
| Navigation | Front | 1 | Idle only |
| Navigation | Right | 3 | Idle only |
| Navigation | Top | 7 | Idle only |
| Navigation | Toggle projection | 5 | Idle only |
| Actions | Undo | Ctrl+Z | Yes |
| Actions | Redo | Ctrl+Y / Ctrl+Shift+Z | Yes |
| Actions | Select all | Ctrl+A | Idle only |
| Actions | Erase selection | Delete | Idle only |
| Actions | Make Group | G | Idle only |
| Files | Save | Ctrl+S | Yes |
| Files | Open | Ctrl+O | Yes |
| Files | Save As… | Ctrl+Shift+S | Yes |
| Actions | Cancel | Escape | Yes |
| Help | Shortcut help | F1 | Yes |
| Constraints | Lock X | ArrowRight | Yes |
| Constraints | Lock Y | ArrowLeft | Yes |
| Constraints | Lock Z | ArrowUp | Yes |
| Constraints | Clear axis lock | ArrowDown | Yes |
| Tools | Copy | Unbound: Use the existing Copy button; M activates Move | Yes |
| Tools | Follow Me | Unbound: No conventional default | Yes |
| Tools | Section Plane | Unbound: Use the existing section panel; no conventional default | Idle only |
| Measurements | Apply measurement | Enter | Yes |
| Measurements | Edit measurement | Backspace | Yes |
<!-- shortcuts:end -->

Bindings are generated/checked against the central action registry. F1 opens searchable
help; focused text editors and nonempty measurement entries keep their keys.
See [shortcut behavior and verification questions](docs/SHORTCUTS.md).
Mouse navigation is unchanged: middle drag orbits, Shift+middle drag pans, scroll zooms.
Grid spacing is one meter; Fit uses model bounds or a central 20 × 20 meter area.

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

Choose Line (L), Rectangle (R), Circle (C), Arc (A) or Polygon (P). Drag to draw a line or rectangle in one gesture, or click successive anchors; the click workflow remains available for precise placement. Type a measurement and press Enter after the first anchor. Examples: `5m`, `120cm`, `2m,3m` for rectangles, `90deg` after an arc start. Arc clicks specify center, start and end direction. Escape cancels; Ctrl+Z / Ctrl+Shift+Z undo/redo. Arrow Right/Left/Up lock X/Y/Z; Down unlocks. The top toolbar chooses XY/XZ/YZ construction planes and curve segment counts. Hover geometry for snap markers and guides.

Inference and tools are CPU-only libraries with independent tests. To capture the shaded demo and exercise incremental GPU buffer growth, append `--incremental` after the capture path in the capture example command.

## Modify (M4)

Choose a modify tool in the left palette. Select by clicking, drag left-to-right to enclose geometry, drag right-to-left to cross it, or triple-click for connected geometry. Shift adds to selection; amber outlines show the selection.

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

Use **File** to create/open/save `.cdraft`, import OBJ/glTF/GLB/COLLADA, export OBJ/STL/glTF 2/COLLADA (.dae), and recover autosaves. Ctrl+O opens, Ctrl+S saves and Ctrl+Shift+S uses Save As. Opening fits visible geometry and starts fresh undo/redo history at root. Manual saves and recovery snapshots use atomic ZIP writes; changed committed values autosave every 30 seconds in the background. Startup offers retained crash snapshots.

Imports append as one undo step, with component/loose geometry and coplanar cleanup options, background progress, cancellation and a summary. See the [import pipeline and limits](docs/IMPORT_PIPELINE.md).

Materials can embed PNG/JPEG images; original bytes survive native round trips, while the viewport uses 256×256 planar previews. OBJ/STL flatten visible instances in metres/Z-up; glTF shares meshes with nested placement nodes and converts to metres/Y-up. COLLADA 1.4.1 preserves right-handed Z-up, shared components, nesting, transforms and current face materials/planar textures, with meter or inch units. See [COLLADA export](docs/DAE_EXPORT.md), [format and recovery limits](docs/FILE_FORMAT.md) and [save/load measurements](docs/FILE_BENCHMARK.md).

```sh
cargo run --release -p cindral-io --features bench-fixtures --example file_benchmark
cargo run -p cindral-io --example make_texture_demo -- /tmp/textured.cdraft
CINDRAL_CAPTURE_DRAFT=/tmp/textured.cdraft WGPU_BACKEND=opengl cargo run -p cindral-app --example capture_viewport -- /tmp/textured.ppm
```

Section planes and geographic shadows are available in the Styles & Scenes panel. See [controls, validation and release measurements](docs/SECTION_SHADOW_BENCHMARK.md). This completes the planned M0–M7b feature milestones.

## Known limitations: SketchUp import

Read-only `.skp` import accepts classic MFC versions **3–8 and 2013–2020** and
modern VFF versions **2021–2026**. Compatibility varies by legacy dialect;
unknown/corrupt records can produce an explicitly reported partial import.
Future versions fail with a named error. Open/Import uses authored faces with
holes, shared definitions, placements, materials/images, tags and edge flags;
save the result as `.cdraft`. There is no `.skp` export.

Scenes/cameras, styles, section planes, dynamic behavior/custom dictionaries,
text/dimensions, construction guides, analytic curves, billboard behavior,
tag colors/folders, separate front/back rendering and source display preferences
are omitted. Textures use planar mapping; source UV pins/tile sizes and exact
colorization are not reproduced. Boundary-edge tags follow their face container.
The created, ignored `tests/fixtures/skp-local/` directory had no user files;
production-file compatibility/performance remains unverified. See the
[reader, version evidence, safety limits and cross-check workflow](docs/SKP_IMPORT.md).

## Exporting to SketchUp

1. Open your model and choose **File > Export > COLLADA…**.
2. Choose **Meters** or **Inches**. Keep **Include textures** enabled for textured models. Use **Triangulate all faces** if the receiving tool needs triangles. Hidden geometry is excluded by default. Enable **Selection only** to export selected faces/edges or the Outliner container; an empty selection exports an empty scene.
3. Click **Choose file and export…**, select a `.dae` destination and confirm any replacement. The background export shows progress and supports cancellation before publication.
4. Keep the `.dae` and its adjacent `cindral-textures-*` folder together when moving or sharing them.
5. In SketchUp desktop, choose **File > Import**, choose **COLLADA Files (*.dae)**, select the file and click **Open** (Windows) or **Import** (macOS). See the [official COLLADA import instructions](https://help.sketchup.com/en/sketchup/importing-and-exporting-collada-files).

Geometry, holes (triangulated), groups/components, shared instance transforms, colors, transparency, textures and current planar UVs are kept. Soft/hidden edge flags, tags/layers, scenes, styles, section planes and shadows are lost. Independent back-face materials and original UV seams are unavailable in the native model; only its current face material and planar mapping can be exported. See [export options, CLI and limitations](docs/DAE_EXPORT.md).

This independent exporter follows the public COLLADA specification. Cindral Drafter is not affiliated with Trimble or SketchUp.

## Exporting images

Use **File > Export > Image (PNG)…** to save the current view at a chosen size with its styles, materials, sections and shadows. Transparent backgrounds and Off/Medium/High antialiasing are available. The dialog explains framing and shared-memory limits. For headless single-scene CLI export and limitations, see [PNG export](docs/PNG_EXPORT.md).
