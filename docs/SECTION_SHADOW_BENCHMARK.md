# M7b section planes and shadows — 2026-10-21

Hardware: AMD Ryzen 5 2400G / Radeon Vega 11, local Linux Wayland/OpenGL (radeonsi). Release Rust build, production scene pipelines, 1280×800 offscreen color/depth target. The fixture is the existing 5,000-box shared-component scene: one cube definition, 60,000 submitted face triangles across opaque/transparent passes, retained geometry and instance buffers. The shadow map defaults to Medium (2048, nine filtered comparison taps); Low is 1024/four taps, High is 4096/nine taps. Shadows and edge casting are off by default.

Reproduce without concurrent builds/agents:

```sh
cargo build -p cindral-app --example component_benchmark --example capture_viewport --features kernel-benchmark --release
timeout 90s target/release/examples/component_benchmark
```

Each static mode gets 20 warmup and 100 measured submissions. Timers include encoder creation, submission and device completion. Shadow redraw counters assert no redraw during unchanged frames, camera movement, receiver toggles or intensity changes; buffer-capacity assertions check that toggling effects retains geometry allocation. Instance edits get 30 measured changed placements and redraw the map. Section movements use the real app cap cache on the same 5,000-instance document: 30 changed Z cuts, with CPU cross-section generation, separate cap upload, invalidated map, color pass and GPU completion all timed together. Bounds traversal/setup is outside that section timing. Map drawing includes clipped faces and caps, with edge casting disabled.

| GPU-completed workload | Mean ms | p95 ms | Maximum ms |
| --- | ---: | ---: | ---: |
| Shadows off | 1.752 | 1.882 | 1.973 |
| Shadows on, Medium | 2.485 | 2.890 | 3.159 |
| Instance update + invalidated map + color | 4.076 | — | 4.573 |
| Live section: cap CPU + upload + map + color | 7.956 | — | 15.606 |

Section cap CPU time alone averages 2.376 ms, maximum 3.871 ms; each section frame draws 90,000 separate cap vertices. Every sampled edit/section frame is below 16 ms, so Medium remains the default. The section maximum leaves little headroom for excluded UI work; it is not a full editor latency guarantee. Static shadows render the depth map exactly once across 120 frames, versus zero renders when disabled. Camera/intensity/receiver changes reuse it, and geometry allocation is unchanged across effect toggles.

[Final raw output](benchmarks/2026-10-21-m7b-sections-shadows.txt) records adapter and CPU setup paths. The preliminary run preceded the white-array/alpha-caster corrections and is superseded by this final run. Native release captures of solid/transparent/mirrored groups and checker-textured shared components pass without GPU validation errors; orange filled caps, plane widget, cut-away geometry and ground/face shadows were visually checked. The white-only texture fallback now allocates a real two-layer array to avoid black materials on the OpenGL backend.

These are measured editor paths, not full presented frame latency: egui/Outliner layout, inference, grid, widget painting, swapchain composition, event scheduling and vsync are excluded. They establish neither a hard real-time guarantee nor arbitrary connected-solid/Boolean performance. Map resolution/filtering, screen coverage, transparency, model complexity and desktop contention affect timings. Only the affected component/section/shadow benchmark was rerun for M7b; unrelated file and connected-kernel benchmarks were not rerun.

## Using the feature

Expand **Section planes** in Styles & Scenes. Choose Face or X/Y/Z and click Place, then click a face or the axis plane. Multiple named widgets can coexist; choose the active plane and Enable cut. Widget visibility is independent of clipping. Flip changes the retained half-space. Move widget enables normal-axis dragging at the widget center with live caps; Escape cancels the drag. Numeric Offset moves the plane directly. Named scene capture/update stores all planes, their active/enable/visibility values and sunlight settings; switching scenes restores them together.

Expand **Sun & shadows** to enable shadows, edit Gregorian date, local time, latitude/longitude and UTC offset, and adjust light/dark intensities. The model uses +X east/+Y north/+Z up. UTC offset includes DST explicitly; there is no network geocoding/timezone database. Face/ground receiving and optional edge casting are independent. Ground is a finite Z=0 receiving plane around the visible model bounds. Nighttime uses the dark intensity and no depth-map render.

Sections never alter topology or geometry history. Closed shells generate filled caps, including holes, transformed groups and shared/mirrored components. Open/nonmanifold shells do not get a fictitious fill. Picking ignores removed faces/vertices/edge portions and removed occluders; caps are view-only surfaces without editable entity IDs. Native v4 files persist settings; v1/v2/v3 fixtures migrate. OBJ/STL/glTF export ignores sections and shadows.
