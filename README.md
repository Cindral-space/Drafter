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
- [ ] M2: geometry kernel
- [ ] M3: drawing and inference
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
| Fit construction area | F or Zoom extents |
| Isometric / front / right / top | 0 / 1 / 3 / 7 |
| Toggle projection | 5 |
| All standard views | View sidebar |

Keyboard shortcuts apply while the pointer is over the viewport. Grid spacing is one meter; until document geometry is introduced, extents fits the central 20 × 20 meter construction area.

For reproducible native GPU validation, run:

```sh
WGPU_BACKEND=opengl cargo run -p cindral-app --example capture_viewport -- /tmp/cindral-viewport.ppm
```

The example captures the rendered window and exits. Use the native display session: forcing X11 in a Wayland session can make EGL choose a different platform from the window.
