# Architecture

## Status

M0–M6 implement the workspace/CI, viewport/camera, transactional geometry kernel, pure inference and drawing, modify tools, nested organization/instancing, and native files/recovery/textures/import/export. Measured static 500k-triangle, component, kernel and connected-solid benchmarks distinguish their workloads and limits. M7a implements Styles and Scenes; M7b implements section planes and shadows. I1 adds the shared import pipeline and glTF/GLB/COLLADA import. Public interfaces are frozen by the integrating engineer before dependent tasks are delegated.

## Crate boundaries

| Crate | Owns | Allowed application dependencies |
| --- | --- | --- |
| cindral-core | Topology, entities, transforms, command history | none |
| cindral-inference | Geometric queries and snap candidates | core |
| cindral-tools | Tool state machines, CPU picking/selection and command creation | core, inference |
| cindral-render | Camera, GPU resources and styles | core |
| cindral-skp | Bounded independent SketchUp container/record reader | core |
| cindral-io | Versioned documents and import/export | core, skp |
| cindral-app | Window, egui controls, input routing | all above |

Rendering and UI dependencies never enter core. Tools emit commands; they do not mutate renderer buffers. IO deserializes into validated core documents. E1 adds independent COLLADA 1.4.1 export with shared definition geometry, nested instance nodes, meter/inch Z-up coordinates, planar textures and cancellable atomic publication. The desktop and display-free CLI share the same options and safety boundary; see [COLLADA export](DAE_EXPORT.md) and [release measurements](DAE_BENCHMARK.md).

## Core data model

A document owns stable typed entity IDs and ordered maps of vertices, undirected edges, planar faces, groups, component definitions/instances, tags, and materials. Positions use double precision. One shared tolerance constant governs geometric comparisons; derived predicates account for scale. A face has an outer vertex loop, zero or more inner loops, a plane and material. Edges carry soft/smooth flags. Adjacency is derived and validated after topology changes.

Groups own local geometry and transforms. Component instances refer to shared definitions; recursive references are rejected. Tags control visibility, independently of topology. Selections identify entities and their instance path.

Every document mutation runs through a transactional command: prepare and validate a candidate change, apply atomically, then record the inverse. Failed commands leave the document and history intact. New successful commands clear redo history. Render caches use document revision and dirty entity sets, with per-container GPU buffers and component instancing.

## Layer boundaries

Inference ranks endpoint, midpoint, intersection, edge, face and directional candidates using screen-space distances. Tools hold interaction state and interpret measurements in document units. Camera math is independent of window events. Picking shares camera transforms with rendering. `.cdraft` is a versioned compact ZIP/RON archive with explicit migrations and validation; exporters triangulate faces including holes.

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

`History::new()` owns an empty model. `model() -> &Model` is read-only. `execute(Command)`, `undo()` and `redo()` return `Result<bool, KernelError>` (false is no-op). `revision() -> u64`, `undo_depth()` and `redo_depth()` expose history state. All entity creation and attribute changes go through commands; failures preserve model, redo and revision. M2 used validated snapshots; M3 replaces these with validated inverse entity deltas (see below). A high-water ID allocator prevents ID reuse after branching edits.

Commands: `AddEdge { start: DVec3, end: DVec3 }`; `AddPolyline { points: Vec<DVec3>, closed: bool }` (closed input omits repeated endpoint); `EraseEdges(Vec<EdgeId>)`; `SetEdgeStyle { edges: Vec<EdgeId>, soft: bool, smooth: bool }`; `Batch(Vec<Command>)` (one atomic undo step, nesting limited to 64). `Model::vertices()/edges()/faces()` return borrowed BTreeMaps. `bounds()` returns optional min/max, `incident_edges(VertexId)` iterates `(EdgeId, &Edge)`, `face_area(FaceId)` returns square meters, `triangulate(FaceId)` returns world-space `Vec<[DVec3;3]>`, and `validate()` checks topology. These methods return `Result` where lookups/triangulation can fail.

M2 reconstructs bounded planar regions from split 3D edge arrangements using angular half-edge traversal, decomposes walks at bridges/articulations, and assigns nested loops as holes. Every bounded region has a face, so an inner closed loop creates both an annulus and its enclosed face. Supporting planes are derived at non-collinear vertices; existing plane orientation is retained where possible. Faces have a canonical normal, not a closed-solid outward orientation. Erasing edges rebuilds regions and dissolves redundant collinear degree-two vertices unless styles differ. Intersection splits inherit edge styles. Triangulation uses earcutr with all face rings projected relative to the same local origin.

This is a correctness-first kernel: global face rebuilding/full edge-intersection validation is at least quadratic on dense input; history snapshots cost model-sized memory per step. It is independent of the static GPU benchmark. M3 replaces the global edit path and snapshots as described below; this paragraph records the original M2 implementation. Groups/components/tags/material assignment enter at M5; solid extrusion enters at M4, not M2.

### M2 validation and remaining limits

The core has 34 tests plus a tested public-API documentation example: 26 deterministic geometry/history cases and eight property tests, configured for 1,152 generated cases per normal run plus persisted regressions. Coverage includes randomly tilted planes translated by up to 200 km; chord split/erase healing; hole area and hole-excluding triangulation; self-crossing and concave loops; bridges and touching holes; cube faces; angular degeneracies; style inheritance; rollback, no-op/redo behavior, exact undo/redo and ID high-water behavior. Random planar segment arrangements additionally check Euler's bounded-region invariant after each insertion. The complete workspace, including the benchmark example, runs 65 tests plus the documentation example.

Adversarial review found and fixed two different predicate problems: shallow-angle intersections were discarded by a fixed angular cutoff, while translated collinear overlaps were missed when rounding perturbed their directions. Plane candidates now use physical altitude, overlap detection checks physical distance before angular intersection division, and persistent proptest seeds retain the translated-overlap failure. Existing plane candidates are deduplicated and face identities compare canonical loops independent of winding to prevent duplicate regions from numerically equivalent candidate planes.

M2 does not introduce draw-tool UI, solid manifold validation, outward solid winding, groups/components, persistence, or large-document editing performance claims. Geometry is validated before command commit; M3 addresses local editing scalability below. The independent GPU baseline measures one indexed draw with depth/shading, with 1.46–1.63 ms mean and 1.70–2.03 ms p95 on the local Radeon Vega 11/OpenGL release runs; it excludes editor interaction, UI, picking, dirty rebuilds and instancing. M3 implementation and limits follow below.

## M3 drawing and pure inference

[The frozen interfaces](M3_INTERFACES.md) separate CPU inference (`core`/`glam` only) from rendering. Screen-distance ranking supports endpoints, midpoints, intersections, edges, faces including holes, axes, parallel/perpendicular guides and in-progress arc tangent hints. Arrow keys impose hard axis locks. Existing tessellated arcs have no analytic tangent metadata. Ten inference tests run without a window or GPU.

`ToolSession` owns Line, Rectangle, Circle, Arc and Polygon interaction, previews and typed measurements; it emits ordinary kernel commands. Units include m/cm/mm/in/ft, rectangle dimensions accept a comma-separated pair, and arcs accept signed degrees. Arc interaction uses center/start/end direction. Circle/polygon segment counts are configurable. The app routes pointer/keyboard input, undo/redo, construction planes and camera fitting to model bounds. Drawing failures retain interaction state for correction.

Document faces and edges render through a depth-tested offscreen wgpu pass composed into egui. Mesh slots are retained across edits, free slots become degenerate vertices, and merged dirty byte ranges are uploaded. Buffer growth copies existing geometry; capacity has headroom. Headless synthetic-input tests exercise typed drawing, axis locks and undo/redo; native captures exercise shaded shapes and growth across frames. The draw→push/pull→save→load integration test remains dependent on M4/M6.

## Incremental update plan and M3 implementation

[Kernel measurements](KERNEL_BENCHMARK.md) showed a 203,080 ms global edit and 60–74 ms full CPU mesh rebuild on the requested 50k-edge/20k-face fixture, triggering the pre-M4 optimization requirement. The following steps are implemented:

1. Maintain R-tree bounds and adjacency/face-boundary indices. Prepare a local transaction cavity around command bounds, include affected faces and loose/exterior boundaries, preserve vertices used outside the cavity, then reconstruct and validate only that candidate. Full `Model::validate()` remains available and checks cache consistency. Dirty sets contain changed vertex/edge/face IDs, exposed by `History::changes()` with the revision.
2. Record before/after entity deltas and allocator state rather than whole-model snapshots. Commit and undo/redo update the indices and dirty sets; high-water IDs and atomic failure behavior remain intact.
3. Re-triangulate only dirty faces and rebuild only dirty edges in `MeshCache`. Apply range writes to persistent GPU buffers. Camera changes only update transforms, not topology/mesh caches.

Measured local edits are 0.124–0.137 ms and dirty mesh preparation 0.036–0.040 ms on the favorable separated-square fixture. Large transaction cavities still use the conservative arrangement algorithm and may exceed 16 ms. Whole-model inference candidate scans remain unbenchmarked. These results do not establish complete editor frame rate.

Before M4 movement/extrusion, propagate position changes to incident edges/faces and their spatial bounds even when endpoint IDs are unchanged; current commands only add/remove geometry or change edge styles. At M5, partition dirty sets and mesh buffers by group/definition, transform queries to local space, and share definition buffers across component instances. Root face/edge dirty tracking is implemented now; groups do not yet exist. Consider compaction thresholds for fragmented slots after substantial deletion, without returning to a rebuild on every edit.

The existing proptest suite explicitly covers randomized face splitting, erasure merging and vertex healing (`test_rectangle_chord_split_erase_healing`), plus holes, area/Euler invariants, rollback and exact history restoration: 1,152 generated cases per normal run plus persisted regressions. Local transactions additionally compare connected-grid edits against global reconstruction and verify independent geometry remains untouched.

## M4 modify tools and provisional transactions

[The M4 contract](M4_INTERFACES.md) adds Select, Push/Pull, Move, Copy, Rotate, Scale, Offset, Follow Me, Eraser, Tape Measure and Paint Bucket. Selection is pure CPU logic: visible ray picking with screen-radius vertex/edge candidates, enclosed left-to-right windows, intersecting right-to-left windows and connected traversal for triple-click. The app draws a distinct amber selection overlay. Shift adds to selection. Inference and typed measurements share M3 camera/query math and the existing measurements box.

Interactive tools generate absolute commands from their initial anchors. `History::preview` reverts the previous provisional entity delta, prepares/applies a new local delta, and reports the union of reverted and newly changed IDs. It never clones the whole model or appends an undo entry. GPU mesh preparation consumes the same dirty sets as ordinary commands. `commit_preview` records the final delta once on release; `cancel_preview` restores the exact starting geometry/allocator state, preserving committed undo/redo. Invalid previews restore the starting geometry and expose an error. Revisions still advance so mesh restoration is observable. A click without a drag leaves an anchor for typed input; Enter commits one measured command. Escape and tool changes cancel. Release outside the viewport is handled.

Move/Copy resolve selected vertices and induced edges/faces; affine transforms propagate incident face planes and dirty spatial bounds even when IDs are unchanged. Rotation/scaling use a selection-center pivot and initial cursor handle; typed degrees and scale factors use that same pivot. Nonplanar adjacent-face deformations, degenerate scales, singular matrices and out-of-range positions fail atomically. Face RGBA colors are command-owned and part of the renderer's face signature; Paint Bucket changes only affected face uploads. Full material definitions/texture assignment remain M5.

Push/Pull extrudes the chosen planar profile, including hole rings. Closed shell orientation is derived from shared edge winding and signed volume, independently of canonical face normals. Outward motion unions the extrusion with the boundary; inward motion subtracts it. Exact-depth and beyond-depth cuts naturally produce apertures. Original BSP clipping produces convex boundary pieces; coplanar seams are removed, T-junctions welded and only represented surface regions retained. Closed inputs must retain two incident faces per boundary edge. Stable IDs and attributes are reused on untouched surfaces so they do not enter the GPU dirty set. Later drawing/offsets consult surrounding hole boundaries to avoid re-capping explicit openings. Deliberate loose path/drawing edges in the edit cavity are preserved.

The Boolean cavity includes intersecting connected shells; very large connected solids can therefore cost more than small local edits. BSP depth/fragment limits reject overly complex operations rather than silently committing partial output. This is an explicit limit, not a proof of robust general-purpose CAD booleans. Open-sheet extrusion preserves adjacent sheet faces, which may yield a non-manifold sheet/solid junction; closed-solid guarantees apply to closed inputs. Group-scoped solid semantics remain M5.

Offset draws a mitered outer boundary inset/outset; collapsing/reversing offsets are rejected. Follow Me parallel-transports a profile along an ordered selected edge path, or along a single cursor/typed segment. The first path segment must be normal to the profile; side quads are triangulated at bends. General self-intersecting sweep regularization and offsetting all hole rings are not implemented. Tape Measure stores editor-only guide lines and a readout; measuring, adding a guide and cancelling never modify real geometry/history. Guides can be displayed while working; persistence is deferred to M6.

### M4 validation and performance

Kernel tests cover outward extrusion, blind pockets, exact/beyond-depth tunnels, annular profiles, pulling adjacent faces, repeated edits without aperture closure, mirrors/copies/transforms, straight/bent sweeps, painting and stable dirty IDs. Three new proptests generate 160 cases for closed-solid incidence, volume/undo invariants, pockets/tunnels and tilted solids translated by up to 200 km. Together with M2/M3 properties this is 1,312 generated cases plus regression seeds. Scripted app tests draw a rectangle, extrude, move, undo twice and redo, verifying each state; synthetic input also covers live move/cancel, typed extrusion/rotation/scale, triple-click selection and non-mutating Tape Measure.

The large-model benchmark now includes 30 repeated closed-box extrusion previews, one final undo entry, and separate kernel/mesh/completed-upload timings. See [M4 measurements and limits](KERNEL_BENCHMARK.md#m4-rerun). It measures the update path, excluding inference scans, selection, egui and rendering frames; it does not establish 60 fps for arbitrary connected models. Native Linux/OpenGL capture exercises preview, cancellation, a committed through-hole, painted faces and selection highlighting. Windows/macOS remain CI targets rather than locally tested runtimes.

## M5 organization and persistence boundary

`DocumentHistory` is the editor's document owner. Each definition owns a local `History`/`Model`; the root owns loose geometry, groups own private definitions, and components share definitions. Instances store a local affine transform, tag/material references, name and explicit visibility. Nested instances form a definition DAG: cycles, singular/nonfinite transforms and paths over 64 levels are rejected; visible traversal has a 100,000-occurrence budget. Group duplication recursively clones private nested groups while retaining component references. Make Unique clones the target definition and its private group descendants. Reparent preserves world placement by composing the destination inverse and source world transform; changes inside a shared parent affect all its occurrences.

The global undo timeline stores geometry-definition references or metadata record deltas. Geometry history remains local delta history; metadata deltas hold only changed instance/header/tag/material records. Runtime geometry removed by undo is retained for redo. Instance drag previews replace a single placement; Copy creates one provisional occurrence and reuses it until release. Cancel restores the placement/definition link and preserves the redo branch. Context entry/exit is navigation, without an undo entry; global undo targets the originally edited definition and removes invalid/hidden context suffixes. A shared edit therefore reverts every occurrence with one undo.

Double-click enters an occurrence; Escape, Exit, or a click outside its geometry/bounds leaves it. Tools operate only on the active definition, with rays transformed by inverse world matrices and projection multiplied by the composed world matrix. Plane/anchor/axis constraints stay local. Construction guides are cached per definition, so context changes do not reinterpret guides from another local frame. Pure tools-side scene picking preserves foreground world distance, face holes and screen radii under nesting, nonuniform scale and reflection. Definition bounds are computed once per pick. Outside occurrences are dimmed; other placements of an edited shared definition still display the shared live geometry but remain dimmed.

Tags are assigned to containers; loose vertices/edges/faces remain untagged so visibility cannot tear connected topology. Deleting a tag clears its assignments. Materials contain linear RGB, opacity and an optional texture slot; explicit face material wins over the nearest inherited instance material. Deleting a material clears face/container references atomically. Texture decoding belongs to M6. The Outliner virtualizes the hierarchy, supports selection, editing, rename, hide, duplicate/unique/delete and drag-to-reparent; Tags and Materials controls share the sidebar.

`DocumentData`/`DefinitionData`/`GeometryData` are serde values with `schema_version = 1`. They contain no spatial indices, history, active navigation, mesh caches or GPU resources. Load validation rebuilds runtime indices and rejects unsupported schema versions and invalid references. The feature-gated known-fixture constructor skips quadratic topology validation only for deterministic benchmark inputs; it must never be used for files. Native container encoding and migrations remain M6.

The CPU `SceneCache` triangulates each visible definition once, applies dirty face/edge ranges only to affected definitions, and builds occurrence batches separately. Repeated components share GPU geometry buffers. `SceneRenderer` uses instanced opaque faces, edges, then transparent faces, three draws per nonempty definition rather than per occurrence. Normal matrices use inverse transpose, including mirrors and small valid scales. Instance buffers compare placements and upload coalesced changed spans; one placement writes 160 bytes when ordering/count is stable. Capacity changes or reordered batches may upload the whole batch. Multiple queued CPU updates are composed before a GPU callback so cancel/undo cannot leave stale preview slots. Fully hidden definitions release their cached/GPU resources and rebuild when revealed; document replacement explicitly removes old GPU definitions. Transparency is premultiplied with depth testing and no transparent depth writes; overlapping translucent surfaces currently use definition order without triangle sorting.

M5 tests add actual proptests for nested/mirrored query transforms and definition sharing/global undo, plus deterministic tests for private-group cloning, make-unique, transform-preserving reparent/cycle rejection, material/tag deletion, DTO restoration, preview cancellation/redo, shared dirty caches, double-click/Escape navigation and the application-level three-instance shared-edit/undo script. See [M5 measurements](COMPONENT_BENCHMARK.md) and [connected solid measurements](CONNECTED_SOLID_BENCHMARK.md). These distinguish fast occurrence placement from expensive raw topology changes; M5 exposed the connected-solid bottleneck addressed by the measured pre-M6 redesign below.

### M5 diagnosis, superseded by the measured redesign below

The existing local edit/index/delta/partial-buffer infrastructure is in place, but dense connected-shell Boolean work and raw whole-solid transforms still exceed interactive budgets. Preserve the M5 per-definition boundaries. Next: add face-to-shell/adjacency indexes; broad-phase candidate faces for the extrusion prism; preserve untouched shell regions instead of rebuilding all connected surfaces; and use an affine whole-shell fast path with deferred index-bound updates when topology cannot change. Keep per-face material/geometry dirty ranges and metadata matrix spans separate. Add cancellation/bounded work before expensive native edits can stall the event thread. Re-measure the same 21,600-face fixture with completed operations and compare topology/volume/history against the existing kernel before enabling a fast path. Container placement measurements are not evidence that raw connected edits meet 16 ms.

## Connected raw-edit optimization before M6

The measured redesign is implemented and documented in `CONNECTED_PROFILE_ANALYSIS.md` and `CONNECTED_SOLID_BENCHMARK.md`. Whole-definition translations use compact original-value deltas and translated index/render frames; local support-face extrusion preserves untouched B-Rep regions. Full validation uses edge-envelope candidates, orientation uses direct face-edge incidence, and general Boolean reconstruction indexes welds/segments/seams and duplicate polygons. A subsequent indexed edit after translation exposed ULP differences during exact R-tree removal; removal now locates the stable ID in a conservative local neighborhood and removes the exact stored item. Undo restores original coordinates/planes exactly.

At 21,600 faces the raw kernel preview maxima are 2.337 ms push/pull and 4.602 ms Move, with 0.006 ms commits. The 60,000-face scaling runs are approximately linear. A timeout/budget runner and small-model reference equivalence tests guard these paths. General collision/through-cut/hole Boolean latency is still workload-dependent; the support/pocket proof must succeed before the local path is used. M6 can use full load validation, which is hundreds of milliseconds on these fixtures, without putting it into live translations.

## M6 native files, recovery, interchange and textures

`cindral-io` owns versioned ZIP/RON archives, migrations, atomic writes, bounded image decoding and OBJ/STL/glTF boundaries. `DraftBundle` combines the existing runtime-independent `DocumentData` schema 1 with original encoded image assets. Archive wrapper version 2 adds coordinates and checksummed image records; its explicit v1 migration is covered by a saved binary fixture. Full document validation rebuilds runtime spatial indices at file boundaries. History, active context and construction guides remain session state; loading starts at root with empty undo/redo. See [file format and user-visible behavior](FILE_FORMAT.md).

The app owns file dialogs and asset slot→GPU layer mapping, without putting IO into the kernel. It saves committed snapshots, cancels provisional drags for explicit save/open/export, and queues periodic background autosaves after capturing a DTO on the UI thread. Each document/session has a distinct recovery target. A synchronous outgoing-draft backup uses a fresh path so an older in-flight autosave cannot overwrite newer outgoing values. Recovery is explicit and opens an unsaved draft; native files are never silently replaced by recovery.

Source PNG/JPEG bytes are embedded once and reused across materials/instances. Renderer previews use a bounded sRGB texture array (white layer 0, up to 127 asset layers), planar local UVs, repeat sampling and texture alpha through the opaque/transparent passes. `FaceVertex` remains 44 bytes: its material float packs the explicit-material bit and texture layer; instance parameters carry inherited texture layers. No new vertex attributes are needed. Changing the asset table updates painted face slots and instance bindings, preserving incremental geometry caches and translated render frames. Queued scene updates retain the newest texture table. Preview images are 256×256; source resolution is retained for files/export. Transparency still has the M5 ordering limitation.

OBJ/STL flatten visible transformed occurrences; glTF retains nested placements, shared definition geometry and shared accessors across material variants. Native/OBJ/STL coordinates are metres/right-handed Z-up; glTF gets a single Y-up root conversion. OBJ import builds welded DTO geometry directly rather than executing a global topology rebuild per face. Native face normals/planar UVs replace arbitrary imported authored normal/UV seams; general nonplanar OBJ polygons are rejected. OBJ sidecars are individually atomic, not a multi-file transaction.

M6 final IO means: 21,600-face save/load 412.748/536.676 ms;60,000-face 1079.619/1482.567 ms;5,000-instance 14.457/39.187 ms. These include full file-boundary validation, not GPU meshing or the app's second reconstruction. [File benchmark](FILE_BENCHMARK.md) records methods, sizes and raw repetitions. The strict connected kernel runner still passes; completed GPU update maxima 4.501/5.247 ms at 21,600 faces and 7.624/14.887 ms at60,000 faces (push/pull/Move), with commits 0.003–0.007 ms. The 5,000-instance render regression averages 1.556 ms (p95 1.736 ms). These workloads do not imply a full-editor frame-rate guarantee.


## M7a Styles and Scenes

Core presentation DTOs are separate from topology: `ViewStyle`, `SceneCamera`, `SavedScene` and `Presentation`. Scenes capture camera, tag visibility, explicit per-definition/instance hidden flags and a style. Capture reads graph metadata without copying geometry. `RestoreVisibility` restores flags as one metadata transaction, skips deleted IDs and retains flags on objects added after capture. Shared-definition children retain consistent visibility across occurrences. Visibility restoration uses the existing incremental instance/cache path and global undo timeline.

Styles are Shaded, Textured, Wireframe, Hidden Line and X-ray. Uniforms select texture sampling, face shading/occlusion and edge behavior; prebuilt pipelines handle depth-tested or overlaid edges. Soft/smooth attributes remain in persistent edge slots. Cached adjacent-face normals identify view-dependent silhouettes; boundary and silhouette profiles expand existing lines by one physical pixel in each screen direction. These controls never rebuild geometry or instance buffers. Edge vertices now include adjacent normals, increasing edge storage from 24 to 48 bytes per vertex. Background, optional sky and optional ground colors are drawn behind the grid/model, with a Z-up camera horizon; they do not introduce geometry or shadows.

The Styles/Scenes UI supports named scene capture, update, rename and delete. Camera transitions last 0.25 seconds and can be skipped. Presentation values persist in archive v3 while topology remains schema1. v1/v2 files migrate with default presentation. Presentation edits participate in autosave through a separate revision, including protection against stale background completion. Runtime transition state is excluded from archives. M7b section planes and shadows are outside this milestone.

M7a validation: 241 all-feature workspace tests plus one doctest, formatting and strict all-target/all-feature Clippy pass. Native OpenGL exercises all five styles. The changed mesh path measures 1.749 ms mean / 1.879 ms maximum for kernel + dirty mesh + completed upload on the existing local-edit fixture; see [the measured workload and limits](KERNEL_BENCHMARK.md#m7a-cached-profiles-regression-2026-10-21). This does not establish full editor frame latency.

## M7b section planes and sunlight

`SectionState` and `ShadowSettings` are presentation values, captured alongside style in each scene and persisted by wrapper v4. Section normals point into the retained half-space. CPU affine plane conversion uses the transpose of local-to-world (including nonuniform scales/shears/mirrors). Picking filters faces, clips edge segments and ignores removed occluders; inference filters candidates before ranking and clips on-edge candidates. Original geometry/history stays unchanged and interchange exports ignore section effects.

Face/edge fragment shaders clip interpolated world-space plane distances. `section_cap` intersects triangulated closed shells, welds segments using the kernel tolerance, traces loops and triangulates even/odd nested contours with earcut. This is the equivalent-mesh alternative to a stencil fill: caps have independent world-space GPU storage and never enter the document, editable mesh slots, undo or exports. Open/nonmanifold shells are not filled. Caps cache by document revision and effective plane; repeated occurrences reuse local cross sections within a rebuild. Plane visibility is independent of cut enable. Face/axis placement, widget dragging with cancel, numeric offset, active plane, flip and delete live in the presentation panel.

Sun direction uses NOAA Julian-century solar equations, Gregorian civil date, explicit UTC offset and geographic coordinates. X=east, Y=north, Z=up; below-horizon sunlight is suppressed. The renderer maintains a separate depth-only map (1024/2048/4096 quality, medium default), tightly fits visible occurrence bounds in light space including ground Z=0, and uses filtered comparison sampling for soft edges. Opaque faces and section caps cast; optional edge casting is off by default. Face/ground receivers and light/dark intensities are independent controls. Alpha-blended materials do not cast. Depth-map invalidation follows geometry, placement/visibility, effective section, sun and quality; camera-only and intensity/receiver changes reuse the map. Toggling shadows touches uniforms/resources, never geometry buffers.

The shadow benchmark reports GPU-completed offscreen color frames separately from map-invalidating instance updates; neither implies full editor/vsync latency. See `SECTION_SHADOW_BENCHMARK.md` for measured results and limitations. Current float32 raster precision and existing unsorted transparency limits remain. Native GPU checks are local Linux/Vega11; cross-platform runtime remains untested.

Solar math reference: [NOAA solar calculation details](https://gml.noaa.gov/grad/solcalc/calcdetails.html). Unit tests compare the geometric result against the NREL SPA Boulder example (2003-10-17, 12:30:30 MST) within 0.05 degrees, allowing the reference atmospheric-refraction difference.


## I1 general import pipeline

All File > Import formats emit `cindral_io::ImportedScene`: indexed polygons with holes,
styled edges, shared definitions, instances, materials/assets, tags and source units/up-axis.
A common converter performs basis conjugation, unit conversion, cleanup and bounded
validation. glTF/GLB and COLLADA retain shared geometry and nested placements. The legacy
OBJ bundle APIs remain compatible; the UI uses its intermediate-scene entry point.
See [import contracts, security limits and release workloads](IMPORT_PIPELINE.md).

`BulkGeometry` builds welded mesh topology directly using tolerance spatial hashing and
unique edge lookup. It never routes imported triangles through edit intersection splitting.
Cleanup joins disjoint coplanar triangle pairs and softens compatible remaining seams;
material/color boundaries survive. Final validation rejects unsplit crossings with a
five-million candidate-pair budget; polygon work and input counts are bounded separately.
Geometry and organization deltas produce one import undo entry, including when appending
to an existing editing context. Later edits retain normal undo/redo behavior.

Import workers parse, validate, append, decode textures and prepare initial CPU render
caches; the foreground installs prepared state and schedules GPU uploads. Cancellation,
document generations and geometry revisions guard completed results. Snapshot cloning and
GPU uploads remain foreground costs. Original source normals/UV sets are replaced by native
face normals/planar mapping; alpha MASK uses blending. No `.skp` implementation is included.


## I2 native read-only SketchUp import

`cindral-skp` decodes MFC and VFF records independently of UI and IO into raw
inch/Z-up shared geometry, placements, palette/images, visibility tags and
per-primitive assignments. `cindral-io::import_skp` adapts those values to I1,
validates/recover-isolates polygons, partitions authored tags/hidden faces into
child containers and converts units once. `ImportedInstance` now carries hidden
state and optional instance paint. Bulk/kernel edges retain hidden independently
of soft and smooth; old serialized edges default hidden to false. File dispatch
bypasses coplanar cleanup for SKP polygons. The existing async append worker,
progress/cancellation, stale-result guards and import summary are reused. Open
routes SKP to import and native saves require `.cdraft`. Details, version evidence,
recovery boundaries and limits: [SKP_IMPORT.md](SKP_IMPORT.md).
