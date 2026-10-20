# M3 frozen interfaces

Only Gemini 3.8 Flash and Gemini 4 Argon may be used in Antigravity; effort is selected per task. OpenCode remains disabled. Existing core public APIs remain compatible.

## Inference (pure Rust; no render/window/UI dependencies)

Exports glam `DVec2`, `DVec3`, `DMat4` as needed. All types below are public, documented, Clone/Debug where appropriate. Use core's `TOLERANCE` for geometric comparisons and core typed entity IDs.

- `Axis` enum X/Y/Z (Copy, Eq); `direction(self)->DVec3`.
- `DrawingPlane { pub origin:DVec3, pub normal:DVec3 }` (Copy), Default = XY; `intersect(&self, ray:Ray)->Option<DVec3>`; reject invalid/parallel/behind-ray intersections. Normals need not be pre-normalized.
- `Ray { pub origin:DVec3, pub direction:DVec3 }` (Copy).
- `View { pub matrix:DMat4, pub size:DVec2 }` (Copy). `project(&self, point:DVec3)->Option<DVec2>` returns local screen pixels (Y down), only finite points with positive clip W and 0..1 NDC depth.
- `TangentHint { pub center:DVec3, pub normal:DVec3, pub reference:DVec3 }` (Copy): a tangent guide at reference on an in-progress arc. This is guidance, not analytic recognition of tessellated model arcs.
- `Query { pub cursor:DVec2, pub ray:Ray, pub view:View, pub plane:DrawingPlane, pub anchor:Option<DVec3>, pub previous_direction:Option<DVec3>, pub axis_lock:Option<Axis>, pub tangent:Option<TangentHint> }`. `Query::new(cursor,ray,view)` defaults plane XY and other optional fields None.
- `SnapKind` (Copy, PartialEq): Endpoint, Midpoint, OnEdge, OnFace, Intersection, Axis(Axis), Parallel, Perpendicular, Tangent, Plane.
- `SnapEntity` (Copy, Eq): Vertex(VertexId), Edge(EdgeId), Face(FaceId).
- `Snap { pub position:DVec3, pub kind:SnapKind, pub screen_distance:f64, pub guide:Option<[DVec3;2]>, pub entity:Option<SnapEntity> }`.
- `Engine { pub radius_pixels:f64 }`, Default radius 12 logical pixels. `infer(&self, model:&Model, query:&Query)->Option<Snap>`.

Endpoint/intersection, midpoint, on-edge candidates are screen-space ranked within radius; grade-3+ vertices can be labeled Intersection. Ray/plane candidates provide on-face (respect holes) and drawing-plane fallbacks. Nearest face hit wins for face candidates. With an anchor, offer automatic world-axis, previous-direction parallel and drawing-plane perpendicular guides; explicit axis lock is a HARD constraint along the anchor axis (reject incompatible geometry snaps). Use closest-ray/line math so Z locks work from a ground plane. Offer tangent guide from optional TangentHint. Stable tie breaking. Reject nonfinite/degenerate queries safely. Inference must compile/test without a GPU and must never import render, wgpu, egui or winit.

## Tools (pure command generation)

`cindral-tools` exports:
- `ToolKind` Copy/Eq: Line, Rectangle, Circle, Arc, Polygon.
- `ToolError` thiserror: InvalidPoint, Degenerate, InvalidMeasurement(String), NotReady, InvalidSegments (no unwrap/expect on user failures).
- `parse_length(&str)->Result<f64,ToolError>` finite signed meters; supports plain meters plus mm/cm/m/in/ft, rejects unknown units and malformed/trailing input.
- `ToolSession::new(kind:ToolKind)->Self`; `kind()->ToolKind`; `set_kind(&mut self,kind)` resets interaction; `cancel(&mut self)` resets uncommitted interaction; `set_plane(&mut self,DrawingPlane)->Result<(),ToolError>` normal must be finite/nonzero; `plane()->DrawingPlane`; `anchor()->Option<DVec3>`; `anchors()->&[DVec3]`; `previous_direction()->Option<DVec3>`; `tangent_hint()->Option<TangentHint>`; `set_segments(&mut self,usize)->Result<(),ToolError>` range 3..=256; `segments()->usize`; `pointer_move(&mut self,DVec3)->Result<(),ToolError>`; `click(&mut self,DVec3)->Result<Option<Command>,ToolError>`; `measure(&mut self,&str)->Result<Option<Command>,ToolError>`; `preview()->Vec<DVec3>`.

Never mutate a Model; emit core Commands. Preview returns the actual prospective polyline in world coordinates, repeating first point for closed shapes. Shapes are built in DrawingPlane; first click moves its origin to the clicked point while retaining normal. Line is free 3D and chains after each committed segment. Rectangle: first corner then opposite corner with a deterministic basis (XY plane uses +X,+Y). Circle/Polygon: center then radius/first vertex, closed polyline; defaults Circle=48, Polygon=6. Arc: center, start, end direction; start defines radius, end is projected onto that radius in the plane; positive normal CCW sweep, tessellated open polyline (default 24), reject zero/full-circle degeneracy. Arc exposes tangent hint at start when center/start are known. Rectangle default basis must be shared by preview/click/measure. Reject degenerate lengths/areas before emitting commands and preserve state on invalid inputs. Cancellation never erases committed geometry.

Measurements: Line = positive length along preview direction (fallback previous direction or plane +U); Rectangle = `width,height` (signed nonzero dimensions, units allowed); Circle/Polygon = positive radius (preserve preview radial direction, fallback plane +U); Arc after center/start = signed degrees, optional `deg` suffix, nonzero absolute angle <360. Enter commits a command; errors preserve interaction. Tools have no UI/render deps.

## Integration ownership

Integrator owns app UI/input, geometry rendering, kernel benchmark and any core performance changes. Inference and tools agents own only their assigned crate. No dependencies may be added beyond the manifests already defined without asking integrator. No unsafe. Required checks in every task worktree: fmt --check, strict workspace/all-target Clippy, workspace tests. Do not commit; integrator reviews and integrates.
