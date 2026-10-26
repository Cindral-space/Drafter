# Status Check

Evaluation performed against `docs/AGENT_LOG.md`, git log, CLI, and test/benchmark runs.

## Items

### 1. JPEG export
- **Status**: Not done
- **Evidence**: Commit `53672b6`, `docs/AGENT_LOG.md` (line 688: *"no JPEG export path exists (JPEG is texture import only)"*), and `crates/cindral-app/src/main.rs`.
- **Missing**: Viewport / document JPEG export encoder and CLI/UI export options (only PNG and COLLADA export are implemented; JPEG is used only for texture decoding).

### 2. Adjustable FOV and eye-height
- **Status**: Not done
- **Evidence**: `crates/cindral-render/src/camera.rs` line 40 (`pub const FOV_Y: f32 = FRAC_PI_4;`), `docs/FEATURE_GAPS.md` lines 143 and 216–217 (cataloged as open P0 gap: *"Missing — fixed 45-degree vertical FOV"*).
- **Missing**: Dynamic/persisted camera vertical FOV, scene/archive camera FOV migration, and numeric eye-height / target placement controls.

### 3. Incremental Push/Pull + cached Move session
- **Status**: Partial
- **Evidence**: Commits `8549a8b`, `e4bdb6d`, `c5830d7` (local extrusion for planar imported faces), commit `8047176` (cached instance transforms), `docs/REAL_FILE_PROFILE.md` line 146 (*"No broader redesign was started... incremental general-surface extrusion... retain validated rigid-selection transaction structure across changing Move offsets"*), and `docs/AGENT_LOG.md` line 263.
- **Missing**: General-surface cavity-based incremental extrusion for Push/Pull, and cached Move transaction sessions for loose/rigid geometry (Push/Pull preview on `SKP/mybedroom.skp` still takes 33.3–44.5 ms).

### 4. H1 hardening: fuzzing, undo/redo audit, real UI frame time
- **Status**: Partial [unverified]
- **Evidence**: `crates/cindral-io/tests/i1_formats.rs` line 179, `crates/cindral-skp/src/legacy.rs` line 2192, commit `e6040b3` (malformed-mesh fuzz batch), `docs/TOOL_PARITY/` (tool parity audit documents with undo/redo checks), commit `8047176`, and benchmarks in `crates/cindral-app/src/real_file_benchmark.rs` and `pan_benchmark.rs`.
- **Missing**: Formal named "H1 hardening" phase / continuous cargo-fuzz harness and consolidated undo/redo audit report across all tools [unverified].

### 5. H2 hardening: units, preferences, packaging, README
- **Status**: Partial [unverified]
- **Evidence**: Commits `4544618` (`crates/cindral-core/src/measurement.rs`, unit preferences), `13918a1` / `b5b346f` / `c396c05` (theme and PNG export preferences), `README.md`, and `docs/FEATURE_GAPS.md` line 193.
- **Missing**: Formal named "H2 hardening" phase, unified global preferences dialog / shortcut rebinding, and native application packaging / installers (e.g. .deb, Flatpak, AppImage) [unverified].

### 6. C1: CLI --convert (.skp -> .cdraft), Document Info panel
- **Status**: Partial
- **Evidence**: `crates/cindral-app/src/main.rs` lines 5–24 (only `--export-png` and `--export-dae` supported; no `--convert` argument exists), `crates/cindral-app/src/workspace_ui.rs` lines 864–870 ("Document" card with active-context edge/face count), and `docs/FEATURE_GAPS.md` line 198.
- **Missing**: CLI `--convert` argument for standalone `.skp` to `.cdraft` conversion, and full Document Info panel with document-wide statistics and entity diagnostics.

### 7. Blender hand-check of OBJ/DAE texture orientation (1-V)
- **Status**: Not done [unverified]
- **Evidence**: Exporters write `1 - V` UVs (`docs/MATERIALS.md` lines 130–136), but `docs/AGENT_LOG.md` line 723 explicitly records: *"Blender hand-check of OBJ/DAE V-inversion still pending user confirmation."*
- **Missing**: Manual visual verification in Blender of texture orientation on exported OBJ and DAE models.

### 8. Branch codex/i1-recovery-*: is its work fully superseded on main? List the archive/* tags.
- **Status**: Done
- **Evidence**: Fully superseded on `main`. `docs/AGENT_LOG.md` line 740 records: *"codex/i1-recovery-20261021-143752 is now fully superseded (algorithm ported, tests exceeded); left in place"* via commits `df2d3f0`, `d9c14c6`, and `e6040b3`.
- **Archive tags (`archive/*`)**:
  - `archive/agent-agy-m4-modify`
  - `archive/agent-agy-m4-selection`
  - `archive/agent-dark-mode`
  - `archive/agent-fix-import-pushpull-freeze`
  - `archive/agent-fix-pushpull-lag`
  - `archive/agent-fix-real-file-freeze`
  - `archive/agent-fix-skp-pushpull`
  - `archive/agent-import-vs-open`
  - `archive/agent-measurements-box`
  - `archive/agent-pan-tool`
  - `archive/agent-png-export`
  - `archive/agent-rotate-parity`
  - `archive/agent-shortcuts`
  - `archive/agent-skp-import-warnings`
  - `archive/codex-i1-recovery`

---

## Release-build Timings for `SKP/mybedroom.skp`

Measured on release build (`cargo run --release -p cindral-app --features kernel-benchmark --example real_file_benchmark SKP/mybedroom.skp`):

- **Component Move**:
  - Preview full frame: **mean 0.530 ms**, **max 1.022 ms** (n=60) (Target: 16 ms preferred / 33 ms max bound — **PASSES**)
  - Release / commit frame: **0.420 ms** (Target: < 100 ms — **PASSES**)
- **Push/Pull**:
  - Preview full frame: **mean 33.308 ms**, **max 44.544 ms** (n=60) (Target: 16 ms preferred / 33 ms max bound — **FAILS** 16 ms and 33 ms targets)
  - Release / commit frame: **0.874 ms** (Target: < 100 ms — **PASSES**)
- *Additional context (Loose Move)*:
  - Preview full frame: **mean 20.192 ms**, **max 27.117 ms** (n=60) (Target: 16 ms preferred / 33 ms max bound — misses 16 ms, meets 33 ms)
  - Release / commit frame: **16.037 ms** (Target: < 100 ms — **PASSES**)
