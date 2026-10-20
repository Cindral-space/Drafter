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

## 2026-10-20 — M2 and large-mesh baseline

- User directive: OpenCode stays disabled for the remainder of the project. Never retry discovery or invoke any OpenCode model. Antigravity is the only delegation provider.
- Integrator owns kernel data model, topology invariants, command transactions and integration.
- Task `large-mesh-benchmark`: Antigravity `claude-sonnet-5-5-medium`; branch `agent/agy/large-mesh-benchmark`; isolated worktree. This is a bounded measurement harness using existing eframe/wgpu APIs, so a routine-cost model is appropriate. Ownership: only `crates/cindral-app/examples/renderer_benchmark.rs`, `crates/cindral-app/examples/renderer_benchmark.wgsl`, and `docs/RENDERER_BENCHMARK.md`. No core interfaces are consumed. Outcome: full source/shader review and isolated-worktree checks passed; integrated and measured locally in release mode. Required before M3.

- Task `m2-properties`: Antigravity `gemini-3.8-flash-medium`; branch `agent/agy/m2-properties`; isolated worktree; allowed only `crates/cindral-core/tests/properties.rs`. Chosen for bounded property-test authoring against the frozen API. Outcome: six properties reviewed and integrated; they confirmed a translated-collinear-overlap defect, fixed by the integrator. Property assertions were tightened to the shared tolerance; the generated failure seed was reviewed and retained.
- Task `m2-review`: Antigravity `claude-opus-5-5-high`; branch `agent/agy/m2-review`; isolated worktree. Chosen for difficult independent geometry-invariant review. Read source but produced no changes before HTTP 429 individual quota exhaustion (reported reset about 4h24m). No retry or fallback provider was attempted; the integrator completed the review and added adversarial/Euler regressions. Outcome: no delegated code to integrate; review reworked by integrator.
- Benchmark review: read the complete native harness, shader, CPU tests and methodology. Independently ran formatting, strict all-target Clippy, workspace tests and the example tests in its worktree; all passed before integration. No dependencies were added.
- Benchmark measured three times in release mode on native Wayland/wgpu OpenGL, Radeon Vega 11: mean 1.460/1.628/1.528 ms, p95 1.712/2.028/1.704 ms. Untimed GPU readback confirmed 35.1% pixel coverage. Raw reports: `docs/benchmarks/2026-10-20-vega11-opengl.txt`. This is one static draw with depth and shading, not full-editor performance. No M3 work has begun.

### M2 review and validation

- Integrator implemented typed IDs, read-only maps, double-precision geometry, shared tolerance/coordinate bounds, automatic intersection and overlap splitting, planar half-edge region extraction with nested holes, earcut triangulation, erasure healing and soft/smooth flags. Every mutation is a validated transactional command with undo/redo snapshots, atomic batches, monotonic revision and non-recycled entity IDs.
- Core API was frozen in `da48caf` before property-test delegation. The integrating engineer retained the coupled geometry/history work.
- Integrator's regressions caught shallow-angle crossing/thin-face failures and translated-collinear overlap failures. The delegated suite independently confirmed the latter and persisted a minimized seed. Fixed the production distance predicates rather than relaxing tests. Deduplicated supporting-plane reconstruction and compared face loop identities independent of winding.
- Added independent randomized Euler checks and area-conservation cases, including touching holes and annulus-crossing chords. Full source review completed locally after the stronger Antigravity review hit quota.
- Final kernel coverage: 34 tests plus one doctest. The eight property tests generate 1,152 cases per normal run plus persisted cases. Workspace coverage including the benchmark: 65 tests plus the core doctest. All formatting, strict workspace/all-target Clippy and workspace tests pass after integration. Benchmark CPU tests are enabled in normal workspace tests.
- M2 limits: full topology rebuilding/intersection validation is conservative and potentially quadratic or worse on dense graphs; snapshots consume model-sized memory per undo step. Solid extrusion/manifold winding and editing UI are later milestones. Supported coordinates are ±1e6 m with 1e-7 m tolerance. The GPU baseline is independent of these CPU topology costs.
- Property handoff verification: the integrator replayed the production distance-predicate fix in the task worktree and independently reran all three required checks there; all passed. Integrated the six meaningful properties with stricter shared-tolerance assertions and retained the saved failure seed. Excluded the temporary print-only diagnostic test. The external run was concluded after collecting/reviewing its artifacts and reproduction; final integration and validation were performed by the integrator.

## 2026-10-20 — M3 / inference and draw tools

- User restricts Antigravity delegation to Gemini 3.8 Flash and Gemini 4 Argon, with thinking effort chosen per task. No Claude models; OpenCode remains permanently disabled and is not retried.
- Confirmed M2 proptest coverage in `tests/properties.rs`: randomized tilted/translated rectangle chord split, erase merge and collinear-vertex healing, plus hole triangulation, undo/redo, ID branching and rollback. Additional Euler and crossing-chord properties are in `tests/adversarial.rs`. No missing split/merge/heal property needs to be added.
- Frozen M3 contracts in `docs/M3_INTERFACES.md` before delegation. Inference is pure logic with core/glam only; tools emit commands. The integrator retains app/render integration and core performance work.
