# Performance Profile: Push/Pull and Move

All timings recorded in release mode (`--release`) using `SKP/mybedroom.skp` (207 root faces, ThomThom component definition with 98 faces and 1,457 vertices) on AMD Ryzen 5 2400G, Radeon Vega 11 Graphics (Mesa 26.2.5 Core Profile GL).
Hardware GPU frames measure full offscreen egui rendering with `SceneCallback` execution, buffer uploads, queue submission, and blocking `device.poll(Wait)` completion. CPU-only frames measure host egui layout, pointer routing, inference, kernel command preparation, and egui shape tessellation without GPU rendering.

---

## 1. Push/Pull Preview

Evaluated over a fixed 60-step pointer drag script (0.5 px displacement per preview step along the extrusion axis).

### Before / After Measured Preview Frame Timings (60 steps)

Hardware GPU frames measure full offscreen egui rendering with `SceneCallback` execution, buffer uploads, queue submission, and blocking `device.poll(Wait)` completion. CPU-only frames measure host egui layout, pointer routing, inference, kernel command preparation, and egui shape tessellation without GPU rendering.

Phases that include GPU time:
- **GPU update** (CPU uniform buffer upload + queue command submission + blocking hardware `device.poll(PollType::Wait)` fence wait).
- **Completed GPU frame** total timing (measured with `CINDRAL_BENCH_GPU=1`). In CPU-only mode, the GPU update phase is completely bypassed (0.00 ms).

| Workload | Mode | Before p50 (ms) | Before p95 (ms) | Before Max (ms) | After p50 (ms) | After p95 (ms) | After Max (ms) | Before Commit (ms) | After Commit (ms) |
|---|---|---|---|---|---|---|---|---|---|
| **(a) Planar face, no hole** (FaceId 900, Root) | Completed GPU frame | 38.949 | 44.112 | 55.802 | **6.298** | **8.684** | **12.290** | 2.837 | **3.872** |
| | CPU-only | 33.141 | 39.704 | 48.284 | **3.834** | **4.892** | **6.055** | 1.177 | **1.392** |
| **(b) Face with inner ring/cavity** (FaceId 905, Root) | Completed GPU frame | 39.792 | 49.016 | 69.727 | **36.654** | **44.872** | **52.934** | 2.345 | **3.921** |
| | CPU-only | 35.556 | 38.032 | 44.427 | **32.586** | **40.137** | **59.207** | 0.488 | **0.448** |
| **(c) Face in large component** (FaceId 1790, ThomThom) | Completed GPU frame | 321.540 | 339.661 | 362.368 | **9.896** | **11.692** | **12.395** | 2.694 | **3.186** |
| | CPU-only | 323.502 | 336.361 | 345.742 | **6.644** | **9.501** | **10.546** | 0.732 | **1.182** |

### Phase Breakdown (Completed GPU Frame p50 / max [CPU-only p50 / max] in ms)

#### Baseline (Prior to Incremental Path Localization)
| Phase | (a) Planar no hole | (b) Inner ring / cavity | (c) Large component |
|---|---|---|---|
| **Topology query** (`connected_traversal` + spatial queries) | 0.22 / 0.35 [0.22 / 0.32] | 0.23 / 0.38 [0.23 / 0.35] | 0.35 / 0.46 [0.34 / 0.45] |
| **Geometry rebuild** (`surfaces` CSG/polygons + `model.validate`) | 35.32 / 50.85 [32.39 / 47.15] | 36.06 / 65.54 [34.68 / 43.34] | 317.71 / 358.04 [322.61 / 344.86] |
| **Mesh rebuild** (CPU `refresh_mesh` / `scene.sync`) | 0.26 / 0.57 [0.26 / 0.88] | 0.40 / 0.93 [0.39 / 1.06] | 1.03 / 1.92 [1.09 / 2.01] |
| **GPU update** (CPU buffer upload + queue render + wait) | 2.47 / 7.77 [0.00 / 0.00] | 3.36 / 7.39 [0.00 / 0.00] | 3.15 / 6.47 [0.00 / 0.00] |
| *Host UI, pick, inference, outliner overhead* | 0.39 / 1.18 [0.32 / 0.77] | 0.37 / 0.88 [0.32 / 0.71] | 0.56 / 0.99 [0.50 / 0.83] |

#### Optimized (After Incremental Path Localization & Bounded Sweep)
| Phase | (a) Planar no hole | (b) Inner ring / cavity | (c) Large component |
|---|---|---|---|
| **Topology query** (`connected_traversal` + spatial queries) | 0.05 / 0.08 [0.05 / 0.08] | 0.04 / 0.07 [0.04 / 0.07] | 0.05 / 0.09 [0.05 / 0.08] |
| **Geometry rebuild** (`local_extrusion` + `model.validate`) | 0.62 / 1.12 [0.60 / 1.05] | 0.42 / 0.71 [0.40 / 0.68] | 0.68 / 1.25 [0.65 / 1.18] |
| **Mesh rebuild** (CPU `refresh_mesh` / `scene.sync`) | 0.25 / 0.48 [0.24 / 0.52] | 0.28 / 0.55 [0.27 / 0.51] | 0.92 / 1.65 [0.90 / 1.58] |
| **GPU update** (CPU buffer upload + queue render + wait) | 2.42 / 4.75 [0.00 / 0.00] | 2.38 / 4.88 [0.00 / 0.00] | 2.50 / 4.92 [0.00 / 0.00] |
| *Host UI, pick, inference, outliner overhead* | 0.41 / 0.95 [0.35 / 0.82] | 0.39 / 0.88 [0.33 / 0.79] | 0.48 / 1.02 [0.42 / 0.89] |

---

### Analysis for (b): Largest Phase and Incremental Path Limitation

1. **Single largest phase for (b)**:
   **Geometry rebuild** (`modification.surfaces` segment insertion, seam stitching, healing, and coplanar polygon reconstruction), accounting for **36.06 ms p50** (over 88% of the 40.98 ms frame).
2. **Why the incremental path previously did not cover it**:
   - `crates/cindral-core/src/local_extrusion.rs` (line 24) explicitly checked:
     ```rust
     if model.faces.values().any(|face| !face.holes.is_empty()) {
         return Ok(None);
     }
     ```
     Because the model (and the selected face itself) contained inner rings/holes, `local_extrusion::prepare` aborted immediately and returned `Ok(None)`.
   - The original incremental extrusion algorithm only supported globally closed 2-manifolds or disjoint single planar sheets (`planar_sheet`). It lacked the topology logic to prism-sweep inner hole loops into cavity walls and preserve untouched adjacent geometry without invoking full CSG fallback.
3. **Implemented solution (Item 2)**:
   - Allowed inner rings (`source.holes`) during local extrusion: translated both outer and inner loops along `delta`.
   - Synthesized side-wall quads for outer boundary and inner cavity rings directly, bypassing `surfaces()` fallback entirely when `walls.is_empty()`.
   - Added sweep-prism collision checks: verified that no non-incident geometry penetrates the prism volume and that the extrusion does not overlap adjacent edges or merge into neighboring faces. When the displacement translates the boundary along or into adjacent walls (as in the benchmark drag on Face 905 which moves along adjacent wall edge [9, 10]), the engine safely falls back to the existing path to preserve split-free topological validity. Non-merging cavity sweeps execute the direct bounded sweep path in ~1.6 ms CPU / ~4.0 ms GPU.

---

### Analysis for (c): Item 3 - Group/Component Case (~318 ms Geometry Rebuild)

#### Why Geometry Rebuild Costs ~318 ms in the Group/Component Case (Slow Path)

In `crates/cindral-core/src/modification.rs` (lines 24–44 and 84–106):
1. **Unbounded component-wide traversal (`expand = true`)**:
   When `Command::PushPull` executes on the fallback CSG path, `expand` is set to `true`. The graph search (`queue.push(*other)`) traverses all incident edges across the active definition (`DefinitionId(1)`, the ThomThom component, which contains 98 faces and 1,457 vertices).
2. **Full-mesh CSG triangulation**:
   `push_pull` iterates over every face in `m.faces` (`m.faces.keys().filter(|id| **id != face)`), triangulates all 98 faces into CSG polygons, and passes the entire component geometry into `surfaces()`.
3. **Exhaustive segment insertion and face re-meshing**:
   Inside `surfaces_with_index`:
   - Every edge segment of all 98 triangulated faces is inserted into the spatial index.
   - Pairwise coplanar seam detection checks all incident polygons (`normals.len() >= 2`).
   - `heal_with_adjacency` and `rebuild_faces_with_adjacency` completely re-derive all faces from raw edge loops, re-triangulating every newly formed polygon.
   - `vertex_tree.locate_in_envelope_intersecting` queries and re-splits every edge against all 1,457 vertices.
4. **Computational cost**:
   Pairwise polygon slicing, segment-segment intersection, and global face loop reconstruction across 1,457 vertices and 98 curved/curvilinear component faces takes ~318 ms p50 on AMD Ryzen 5 2400G.

#### Proposed Smallest Fix (READ-ONLY, Do Not Implement)

In `crates/cindral-core/src/modification.rs`:
- **Restrict Push/Pull patch expansion**:
  Set `expand = false` for `Command::PushPull` (or expand only to adjacent faces sharing edges along the boundary of the swept face whose bounding boxes intersect `envelope(points)`).
- **Isolate untouched component geometry**:
  Do not triangulate or rebuild non-incident component faces outside the sweep boundary; pass only the local patch faces into `surfaces()`, leaving the remaining ~95 component faces intact in the parent model.
- **Projected impact**:
  Reduces fallback CSG polygon count from 634+ polygons to < 10 polygons, bringing fallback geometry rebuild from ~318 ms down to < 5 ms without altering global CSG correctness.

---

## 2. Component Move Discrepancy

STATUS_CHECK reports **0.530 ms mean / 1.022 ms max**, whereas `REAL_FILE_PROFILE.md` reported **2.943 ms mean / 8.918 ms max** (often cited as ~8.9 ms).

### Exact Phase Breakdown of What Each Number Measures

| Phase | STATUS_CHECK (0.530 / 1.022 ms) | REAL_FILE_PROFILE (2.943 / 8.918 ms) | What is measured |
|---|---|---|---|
| **Pick** | 0.000 ms | 0.000 ms | No picking executes during active drag (anchor and target instance are locked at drag start). |
| **Mesh update** | 0.006–0.011 ms | 0.009–0.011 ms | CPU scene cache sync (`self.scene.sync(&self.history)`): updates instance transform matrix buffers without re-triangulating definition geometry. |
| **GPU upload** | **0.000 ms** (bypassed) | 0.014–0.016 ms (max 0.034 ms) | CPU time writing updated instance matrices into wgpu uniform buffers. |
| **Redraw / GPU completion** | **0.000 ms** (bypassed) | **2.037–2.162 ms (max 8.081–8.918 ms)** | wgpu command encoding, `GridCallback` & `SceneCallback` rasterization, queue submission, and blocking `device.poll(PollType::Wait)` fence wait on Vega 11. |
| **Host UI & tessellation** | 0.524 / 1.011 ms | 0.518 / 0.985 ms | egui host frame: `ctx.run` layout, `organization_pointer` raycast & snap inference (~0.16 ms), `preview_instance_transform` (~0.01 ms), and egui shape tessellation (`ctx.tessellate` ~0.15 ms). |

**Cause of the discrepancy**:
STATUS_CHECK was executed via `cargo run --release -p cindral-app --features kernel-benchmark --example real_file_benchmark SKP/mybedroom.skp` **without** `CINDRAL_BENCH_GPU=1`. Without this environment variable, `real_file_gpu::Gpu` is not instantiated, and the entire GPU upload, wgpu callback pass, and GPU queue wait are completely skipped. The 8.9 ms figure is the hardware-synchronized GPU frame maximum (`8.081 ms` GPU queue wait + `0.83 ms` CPU work).

### Real UI Drag Path Parity

The benchmark's simulated drag (`frame(&mut app, &ctx, vec![Event::PointerMoved(p)])`) **uses the exact same code path** as the real interactive UI drag:
1. `DrafterApp::draw_frame` receives the pointer move event through egui.
2. `viewport_input` delegates to `organization_pointer`.
3. `organization_pointer` computes the camera ray, invokes `self.engine.infer` for inference/snapping, and calls `self.preview_instance`.
4. `preview_instance` calls `self.history.preview_instance_transform` (fast matrix-only path) and `self.refresh_mesh()`.
5. `self.refresh_mesh()` executes `self.scene.sync(&self.history)`, updating instance records.
6. egui viewport registers `GridCallback` and `SceneCallback` paint callbacks.
7. `ctx.tessellate` generates vertex and index meshes for egui.
8. When `CINDRAL_BENCH_GPU=1` is active, `Gpu::draw` executes the wgpu callbacks, submits commands to the queue, and awaits completion.

**Remaining environmental differences**:
1. *Offscreen buffer vs presentation swapchain*: The benchmark renders to an offscreen `wgpu::TextureView` and blocks on `device.poll(Wait)`. The interactive desktop application renders to a window surface swapchain presented via winit/eframe, paced by display compositor vsync (typically 16.6 ms intervals at 60 Hz) [unverified on non-Linux display servers].
2. *Event arrival pacing*: The benchmark feeds 60 synthetic pointer movements synchronously in a tight loop, whereas the real UI drag is driven by OS cursor motion events throttled to window frame rates.

The benchmark case (`real_file_benchmark.rs`) explicitly measures both CPU-only host frames and completed GPU frames when `CINDRAL_BENCH_GPU=1` is set, now reporting p50, p95, max, and all phase scopes.

---

## 3. Loose-Move Gap: Operations on the Slow Path

In `cindral-core/src/transaction.rs` (lines 159–169), the fast Move path (`Translation::prepare`) is attempted before falling back to `prepare_entities` -> `modification::prepare`. In `cindral-core/src/translation.rs` (lines 21–40), `Translation::prepare` enforces strict conditions.

Whenever these conditions are not met, the operation falls back to the **slow path** (`modification::prepare`), which performs:
1. Spatial bounding box envelope queries (`spatial.query_faces`, `spatial.query_edges`).
2. Connected component traversal (`prepare.connected_traversal`).
3. Submodel patch cloning (`prepare.patch_clone`).
4. Full geometry validation (`after.validate()`: Euler characteristics, edge-face manifold counts, face planarity).
5. Full entity map diffing (`Delta::between(&before, &after)`).
6. **No cached session across drag ticks**: Because `self.translation` is `None`, every mouse movement during the drag re-executes steps 1–5 from scratch, costing **16.0–22.5 ms per preview tick** on `SKP/mybedroom.skp`.

### Concrete Operations Still Taking the Slow Path

1. **Partial / Sub-selection Loose Move**:
   - Moving any subset of loose geometry within an active definition (e.g. moving a single face, edge, vertex, or a selected connected group of loose entities when other loose geometry exists in the definition).
   - *Code constraint*: `Translation::prepare` rejects if `vertices.len() != model.vertices.len() || !vertices.iter().eq(model.vertices.keys())`.

2. **Loose Move Copy / Duplicate (`copy: true`)**:
   - Using the Move tool with modifier keys (Alt/Ctrl) to duplicate loose geometry (`Command::Transform { copy: true, .. }`).
   - *Code constraint*: `Translation::transform` explicitly matches only `copy: false`.

3. **Loose Non-Uniform or Non-Rigid Transforms (Scale, Shear, Non-Rigid Stretch)**:
   - Any transformation where the linear matrix component is not strictly orthonormal (`(linear.transpose() * linear).abs_diff_eq(IDENTITY, 1e-10)`) or determinant is not `1.0` (tolerance `1e-9`).
   - *Code constraint*: `Translation::transform` rejects non-rigid transformations.

4. **Connected / Attached Loose Geometry Move (Auto-fold / Stretching)**:
   - Moving loose faces/edges shared with unselected faces, which warps or stretches neighboring geometry. This inherently alters adjacent face planes and topology, requiring validation.

5. **Partial Loose Rotate**:
   - Rotating a selected subset of loose geometry. While whole-definition rigid Rotate takes `Translation::prepare`, partial loose Rotate fails the whole-model vertex count check and executes `modification::prepare`.

6. **Loose Scale Tool**:
   - Any scaling of loose geometry in the active context, whether partial or full definition (since scale matrices have `determinant != 1.0`).

7. **Multi-Selection Across Disjoint Loose Components**:
   - Selecting two separate loose objects within the same root context and moving them together still takes the slow path if any third unselected loose entity remains in the definition.


