# Architecture

## Status

M0–M2 are implemented: workspace/CI, native GPU grid/axes and camera controls, plus a rendering-independent transactional geometry kernel. A measured static 500k-triangle GPU baseline is also available. Organization and file APIs below remain design intent until their milestones. Public interfaces are frozen by the integrating engineer before dependent tasks are delegated.

## Crate boundaries

| Crate | Owns | Allowed application dependencies |
| --- | --- | --- |
| cindral-core | Topology, entities, transforms, command history | none |
| cindral-inference | Geometric queries and snap candidates | core |
| cindral-tools | Tool state machines and command creation | core, inference |
| cindral-render | Camera, GPU resources, picking and styles | core |
| cindral-io | Versioned documents and import/export | core |
| cindral-app | Window, egui controls, input routing | all above |

Rendering and UI dependencies never enter core. Tools emit commands; they do not mutate renderer buffers. IO deserializes into validated core documents.

## Planned core data model

A document owns stable typed entity IDs and ordered maps of vertices, undirected edges, planar faces, groups, component definitions/instances, tags, and materials. Positions use double precision. One shared tolerance constant governs geometric comparisons; derived predicates account for scale. A face has an outer vertex loop, zero or more inner loops, a plane and material. Edges carry soft/smooth flags. Adjacency is derived and validated after topology changes.

Groups own local geometry and transforms. Component instances refer to shared definitions; recursive references are rejected. Tags control visibility, independently of topology. Selections identify entities and their instance path.

Every document mutation runs through a transactional command: prepare and validate a candidate change, apply atomically, then record the inverse. Failed commands leave the document and history intact. New successful commands clear redo history. Render caches use document revision and dirty entity sets, with per-container GPU buffers and component instancing.

## Planned layers

Inference ranks endpoint, midpoint, intersection, edge, face and directional candidates using screen-space distances. Tools hold interaction state and interpret measurements in document units. Camera math is independent of window events. Picking shares camera transforms with rendering. `.cdraft` will be a versioned compact archive with explicit migrations and validation; exporters triangulate faces including holes.

## Validation

Every crate has a test target. CI runs formatting, Clippy with warnings denied, and workspace tests on Linux, macOS and Windows. Kernel work adds example-based and property tests for topology and command reversibility. Performance claims require measured representative scenes; the static 500k-triangle baseline is recorded in `RENDERER_BENCHMARK.md` with workload limits.

## M1 camera interface (frozen)

`cindral_render::camera` uses glam f32 for viewport transforms (core topology will use f64). Z is up; X red, Y green, Z blue. `Projection` is Perspective/Orthographic; `StandardView` is Iso/Top/Bottom/Front/Back/Left/Right. `Camera: Clone + Debug + Default` has public `target: Vec3`, `distance: f32`, `yaw: f32`, `pitch: f32`, `projection: Projection`. Methods: `eye(&self)->Vec3`, `view_projection(&self, aspect:f32)->Mat4` (wgpu depth 0..1), `orbit(&mut self, delta:Vec2)`, `pan(&mut self, delta:Vec2, viewport_height:f32)`, `zoom(&mut self, scroll:f32, cursor_ndc:Vec2, aspect:f32)`, `zoom_extents(&mut self, min:Vec3, max:Vec3, aspect:f32)`, `set_view(&mut self, view:StandardView)`, `ray(&self, ndc:Vec2, aspect:f32)->(Vec3,Vec3)`. Orbit/pan deltas are logical pixels, scroll is points (positive zooms in), cursor NDC has Y up. Zoom preserves the intersection with the target-depth view plane. Top/bottom must be nonsingular. Methods ignore nonfinite input and clamp distance and pitch to safe ranges. `Camera` is independent of egui/winit.

## M1 renderer and input integration

The app embeds a custom `egui-wgpu` callback in the central viewport. `GridRenderer` owns a persistent line-list vertex buffer, pipeline, uniform buffer and bind group; only the 64-byte camera matrix is updated while navigating. winit is provided by eframe, which owns the surface lifecycle and GPU device. The callback respects egui's viewport/scissor and uses the same sample count and color format as its enclosing render pass. A future geometry pass needs a dedicated depth target; M1 renders construction lines only.

The app routes MMB orbit, Shift+MMB pan, smooth scroll zoom and hovered keyboard shortcuts into the camera. UI actions use the same camera methods. The static grid spans ±50 meters and is not yet adaptive. Zoom extents fits the central ±10 meter area until core geometry supplies document bounds. M1 does not establish the 500k-triangle performance requirement.

### M1 validation and limits

The camera uses an analytic basis that remains valid at the poles. Perspective and orthographic projection share target-plane scale; scroll keeps a cursor ray's target-plane intersection anchored. Read-only transforms sanitize malformed public fields. Targets beyond ±1e9 per coordinate are outside the current f32 viewport range and are rejected/reset before transform evaluation. A later large-coordinate implementation will need a floating origin.

27 workspace tests pass, including synthetic egui input frames for middle-drag orbit, Shift+middle-drag pan and shortcut hover scoping, and camera tests covering 1500 combinations of extents bounds/orientations/projections/aspects. The `capture_viewport` example uses egui's native screenshot command and wgpu GPU readback, writes a PPM and exits; its 1280×800 capture was visually checked on native Wayland with the OpenGL backend. Formatting and strict workspace Clippy pass. No Windows/macOS runtime was tested in M1. The later large-mesh baseline is documented separately.

## Large-mesh benchmark contract

Before M3, a native example (`renderer_benchmark`) measures a static indexed 500×500-cell heightfield (500,000 triangles), with position/normal vertices, shaded rendering and a depth buffer at 1280×800. Existing eframe/wgpu setup supplies the adapter/device; benchmark resources are independent of document geometry. Geometry generation/upload are outside steady-state timing. At least 20 warmup and 100 measured draws each wait for GPU completion; report mean, p50, p95, derived throughput, adapter/backend, build profile and limitations. This is a simple one-draw-call raster baseline, not a claim about complete editor performance, picking, instancing or dirty rebuilds. No dependency changes are required; the example has no public cross-crate interface.

## M2 kernel interface (frozen for test delegation)

`cindral-core` reexports `DVec3`, `TOLERANCE = 1e-7` meters, `MAX_COORDINATE = 1e6` meters, typed `VertexId`/`EdgeId`/`FaceId` (each `from_raw(u64)` and `value()`), `Vertex { position }`, `Edge { vertices: [VertexId; 2], soft, smooth }`, `Face { outer, holes, plane }`, `Plane { normal, distance }`, `Model`, `Command`, `History`, and `KernelError`.

`History::new()` owns an empty model. `model() -> &Model` is read-only. `execute(Command)`, `undo()` and `redo()` return `Result<bool, KernelError>` (false is no-op). `revision() -> u64`, `undo_depth()` and `redo_depth()` expose history state. All entity creation and attribute changes go through commands; failures preserve model, redo and revision. Undo/redo uses validated snapshots initially. A high-water ID allocator prevents ID reuse after branching edits.

Commands: `AddEdge { start: DVec3, end: DVec3 }`; `AddPolyline { points: Vec<DVec3>, closed: bool }` (closed input omits repeated endpoint); `EraseEdges(Vec<EdgeId>)`; `SetEdgeStyle { edges: Vec<EdgeId>, soft: bool, smooth: bool }`; `Batch(Vec<Command>)` (one atomic undo step, nesting limited to 64). `Model::vertices()/edges()/faces()` return borrowed BTreeMaps. `bounds()` returns optional min/max, `incident_edges(VertexId)` iterates `(EdgeId, &Edge)`, `face_area(FaceId)` returns square meters, `triangulate(FaceId)` returns world-space `Vec<[DVec3;3]>`, and `validate()` checks topology. These methods return `Result` where lookups/triangulation can fail.

M2 reconstructs bounded planar regions from split 3D edge arrangements using angular half-edge traversal, decomposes walks at bridges/articulations, and assigns nested loops as holes. Every bounded region has a face, so an inner closed loop creates both an annulus and its enclosed face. Supporting planes are derived at non-collinear vertices; existing plane orientation is retained where possible. Faces have a canonical normal, not a closed-solid outward orientation. Erasing edges rebuilds regions and dissolves redundant collinear degree-two vertices unless styles differ. Intersection splits inherit edge styles. Triangulation uses earcutr with all face rings projected relative to the same local origin.

This is a correctness-first kernel: global face rebuilding/full edge-intersection validation is at least quadratic on dense input; history snapshots cost model-sized memory per step. It is independent of the static GPU benchmark. Spatial indexing, incremental topology and compact inverse deltas are future optimization work. Groups/components/tags/material assignment enter at M5; solid extrusion enters at M4, not M2.

### M2 validation and remaining limits

The core has 34 tests plus a tested public-API documentation example: 26 deterministic geometry/history cases and eight property tests, configured for 1,152 generated cases per normal run plus persisted regressions. Coverage includes randomly tilted planes translated by up to 200 km; chord split/erase healing; hole area and hole-excluding triangulation; self-crossing and concave loops; bridges and touching holes; cube faces; angular degeneracies; style inheritance; rollback, no-op/redo behavior, exact undo/redo and ID high-water behavior. Random planar segment arrangements additionally check Euler's bounded-region invariant after each insertion. The complete workspace, including the benchmark example, runs 65 tests plus the documentation example.

Adversarial review found and fixed two different predicate problems: shallow-angle intersections were discarded by a fixed angular cutoff, while translated collinear overlaps were missed when rounding perturbed their directions. Plane candidates now use physical altitude, overlap detection checks physical distance before angular intersection division, and persistent proptest seeds retain the translated-overlap failure. Existing plane candidates are deduplicated and face identities compare canonical loops independent of winding to prevent duplicate regions from numerically equivalent candidate planes.

M2 does not introduce draw-tool UI, solid manifold validation, outward solid winding, groups/components, persistence, or large-document editing performance claims. Geometry is validated before command commit; the global arrangement algorithm and snapshot history still require future scalability work. The independent GPU baseline measures one indexed draw with depth/shading, with 1.46–1.63 ms mean and 1.70–2.03 ms p95 on the local Radeon Vega 11/OpenGL release runs; it excludes editor interaction, UI, picking, dirty rebuilds and instancing. M3 has not started.
