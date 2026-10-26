# Performance Profile: Push/Pull and Move

All timings recorded in release mode (`--release`) using `SKP/mybedroom.skp` (207 root faces, ThomThom component definition with 98 faces and 1,457 vertices) on AMD Ryzen 5 2400G, Radeon Vega 11 Graphics (Mesa 26.2.5 Core Profile GL).
Hardware GPU frames measure full offscreen egui rendering with `SceneCallback` execution, buffer uploads, queue submission, and blocking `device.poll(Wait)` completion. CPU-only frames measure host egui layout, pointer routing, inference, kernel command preparation, and egui shape tessellation without GPU rendering.

---

## 1. Push/Pull Preview

Evaluated over a fixed 60-step pointer drag script (0.5 px displacement per preview step along the extrusion axis).

### Measured Preview Frame Timings (60 steps)

| Workload | Mode | Mean (ms) | p50 (ms) | p95 (ms) | Max (ms) | Commit (ms) |
|---|---|---|---|---|---|---|
| **(a) Planar face, no hole** (FaceId 900, Root) | Completed GPU frame | 38.390 | **38.949** | 44.112 | 55.802 | 2.837 |
| | CPU-only | 33.779 | **33.141** | 39.704 | 48.284 | 1.177 |
| **(b) Face with inner ring/cavity** (FaceId 905, Root) | Completed GPU frame | 40.979 | **39.792** | 49.016 | 69.727 | 2.345 |
| | CPU-only | 35.862 | **35.556** | 38.032 | 44.427 | 0.488 |
| **(c) Face in large component** (FaceId 1790, ThomThom) | Completed GPU frame | 322.680 | **321.540** | 339.661 | 362.368 | 2.694 |
| | CPU-only | 322.590 | **323.502** | 336.361 | 345.742 | 0.732 |

### Phase Breakdown (Completed GPU Frame p50 / max [CPU-only p50 / max] in ms)

| Phase | (a) Planar no hole | (b) Inner ring / cavity | (c) Large component |
|---|---|---|---|
| **Topology query** (`connected_traversal` + spatial queries) | 0.22 / 0.35 [0.22 / 0.32] | 0.23 / 0.38 [0.23 / 0.35] | 0.35 / 0.46 [0.34 / 0.45] |
| **Geometry rebuild** (`surfaces` CSG/polygons + `model.validate`) | 35.32 / 50.85 [32.39 / 47.15] | 36.06 / 65.54 [34.68 / 43.34] | 317.71 / 358.04 [322.61 / 344.86] |
| **Mesh rebuild** (CPU `refresh_mesh` / `scene.sync`) | 0.26 / 0.57 [0.26 / 0.88] | 0.40 / 0.93 [0.39 / 1.06] | 1.03 / 1.92 [1.09 / 2.01] |
| **GPU update** (CPU buffer upload + queue render + wait) | 2.47 / 7.77 [0.00 / 0.00] | 3.36 / 7.39 [0.00 / 0.00] | 3.15 / 6.47 [0.00 / 0.00] |
| *Host UI, pick, inference, outliner overhead* | 0.39 / 1.18 [0.32 / 0.77] | 0.37 / 0.88 [0.32 / 0.71] | 0.56 / 0.99 [0.50 / 0.83] |

### Analysis for (b): Largest Phase and Incremental Path Limitation

1. **Single largest phase for (b)**:
   **Geometry rebuild** (`modification.surfaces` segment insertion, seam stitching, healing, and coplanar polygon reconstruction), accounting for **36.06 ms p50** (over 88% of the 40.98 ms frame).
2. **Why the incremental path does not cover it**:
   - `crates/cindral-core/src/local_extrusion.rs` (line 24) explicitly checks:
     ```rust
     if model.faces.values().any(|face| !face.holes.is_empty()) {
         return Ok(None);
     }
     ```
     Because the model (and the selected face itself) contains inner rings/holes, `local_extrusion::prepare` aborts immediately and returns `Ok(None)`.
   - The incremental extrusion algorithm only supports globally closed 2-manifolds or disjoint single planar sheets (`planar_sheet`). It lacks the topology logic to prism-sweep inner hole loops into cavity walls and preserve untouched adjacent geometry without invoking full CSG fallback (`push_pull` triangulates all 207 connected faces into 634 polygons and executes full pairwise segment cuts).

### Proposed Smallest Change (< 16 ms preview, 33 ms minimum bound)

*Do not implement.*

**Bounded local cavity prism sweep**:
In `local_extrusion::prepare`, allow faces with inner rings by sweeping both outer and inner boundary loops along `normal * distance`:
1. Translate vertices of outer and inner rings by `delta`.
2. Generate quad side walls connecting corresponding edges of each loop to its translated counterpart (outer walls facing outward, inner cavity walls facing inward).
3. Update the face's plane equation and vertex positions without touching non-incident geometry.
4. Triangulate only the translated cap face and the newly synthesized side walls rather than triangulating the entire connected shell into CSG polygons.

This bypasses `surfaces()` entirely for non-intersecting sweeps, reducing geometry rebuild from ~36 ms to < 1.0 ms and total frame preview from ~41 ms to **~4–5 ms**, well under the 16 ms target.
