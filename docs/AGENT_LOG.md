# Agent log

## 2026-10-20 — M0 / environment

- Repository: initially empty; initialized `main`.
- Rust: cargo 1.98.1, rustc 1.98.1 (stable, edition 2021 workspace).
- Antigravity: 1.4.2. Ran `agy --help` once. List models with `agy models`; select with `--model <ID>`; run headlessly with `-p <prompt>`. Output must be redirected into ignored `.agent-out/` and verified through diffs and status; inspect disk transcripts when output is empty.
- Available models: gemini-4-argon-{low,medium,high}, gemini-3.8-flash-{low,medium,high}, claude-opus-5-5-{low,medium,high}, claude-sonnet-5-5-{low,medium,high}, gpt-oss-120b-medium. Strong reasoning choice: claude-opus-5-5-high. Routine choice: gemini-3.8-flash-medium.
- OpenCode: 1.19.3. Ran `opencode models` and filtered with `grep -i minimax`; no MiniMax model was listed. No valid M3.1 Flash ID exists in this environment. OpenCode is disabled; no fallback model will be used. User notified. Work can proceed using Antigravity or the integrating engineer.
- Integrator / M0 / main: six library crates, common lint settings, architecture plan, README and CI checks. No delegated implementation yet; interfaces are still being established.

### M0 validation

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` passed (six unit tests plus doc tests). Committed as `95de59d`.

## 2026-10-20 — M1 / viewport

- Integrator froze the camera API in `418175a` before dispatch. Native composition: eframe 0.33.3 (winit 0.30 / wgpu 27), egui-wgpu callback, glam 0.30.
- Task `m1-camera`: provider Antigravity; model `claude-opus-5-5-high`; branch `agent/agy/m1-camera`; worktree `../cindral-wt/m1-camera`. Chosen for strong reasoning on camera basis, cursor zoom invariants and robust bounds fitting. Ownership limited to `camera.rs` and the render library module declaration. No dependencies or other crates permitted. Outcome: reviewed and integrated with an additional finite-target guard and regression test by the integrator.
- CLI stdout remained empty during execution. Verified the on-disk transcript under the CLI's brain conversation `774ea323-e16f-4375-8d61-d3d4bf84d9e8` and checked worktree status rather than treating empty stdout as completion.
- Integrator owns the GPU grid pipeline, shader, app UI/input and integration tests. Grid geometry is uploaded once; camera navigation updates only its uniform buffer.
- Environment limitation: Vulkan loader reports no drivers. GLX reports accelerated AMD Radeon Vega 11 Graphics, Mesa 26.2.3, GLES 3.2. Validate runtime using wgpu OpenGL backend. Other desktop platforms are CI targets, not locally tested systems.

### M1 review and validation

- Reviewed the full camera implementation and test suite. Independently ran all three required checks in the agent worktree; all passed. Only the allowed camera/module changes were integrated. Cargo's regenerated lockfile in the task worktree was not copied; the main workspace owns the complete dependency resolution.
- Integrator removed the render scaffold identity test after real camera/grid tests existed. Added supported-range checks for extreme finite camera targets (±1e9), a regression test, and three headless app tests (projection toggling and synthetic egui orbit/pan/hover input).
- Final checks: formatting, strict workspace/all-target Clippy, and 27 workspace unit tests plus doc tests pass. Native binary and GPU capture example build successfully.
- Runtime validation: native Wayland + `WGPU_BACKEND=opengl` initializes successfully and renders the custom GPU pipeline. First forcing X11 while EGL selected Wayland caused an incompatible-surface error; corrected the test invocation to use the native display. No source workaround or driver installation was required.
- Captured the actual 1280×800 GPU output with the `capture_viewport` example; visually verified sidebar, grid, RGB axes and status bar. Raw PPM and converted PNG remain ignored in `.agent-out/`.
- M1 limitations: construction grid is finite, empty-document extents uses the central 20×20 m area, geometry has no depth pass yet, 500k-triangle performance is unmeasured, and non-Linux runtimes are untested. Next milestone is M2 transactional topology and kernel property tests.
