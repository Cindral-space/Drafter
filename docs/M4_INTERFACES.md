# M4 frozen integration contracts

Core commands extend existing enum:
- `PushPull { face: FaceId, distance: f64 }`: signed meters along canonical face normal.
- `Transform { vertices: Vec<VertexId>, matrix: glam::DMat4, copy: bool }`: affine transform selected vertices; copy duplicates selected induced edges/faces.
- `Offset { face: FaceId, distance: f64 }`: inset/outset face boundary (positive inward).
- `FollowMe { face: FaceId, path: Vec<DVec3> }`: sweep profile along ordered path.
- `PaintFace { faces: Vec<FaceId>, color: [u8;4] }` and `Face.color: [u8;4]`.
`History::preview(Command) -> Result<bool,KernelError>`, `cancel_preview() -> Result<bool,KernelError>`, `commit_preview() -> Result<bool,KernelError>`: replace live provisional delta, no undo entries until commit; cancel restores exact geometry. Changes report all reverted/applied dirty IDs. Undo/redo cancel preview first.

## Pure selection module (tools)
`Selection { pub vertices: BTreeSet<VertexId>, pub edges: BTreeSet<EdgeId>, pub faces: BTreeSet<FaceId> }`, derive Clone/Debug/Default/PartialEq/Eq.
`Hit` enum Vertex(VertexId), Edge(EdgeId), Face(FaceId).
`pick(model:&Model, ray:Ray, view:View, cursor:DVec2, radius:f64)->Option<Hit>`: screen-near vertex/edge and nearest ray/face (holes excluded), foreground occlusion respected.
`Selection::clear()`, `select(&mut self, model:&Model,hit:Hit,add:bool)`, `connected(&mut self,model:&Model,hit:Hit,add:bool)`, `region(&mut self,model:&Model,view:View,start:DVec2,end:DVec2,add:bool)` left-to-right enclosed/window, right-to-left crossing; `vertex_ids(&self,model:&Model)->Vec<VertexId>` includes face/edge vertices.

## Pure modify state module (tools)
`ModifyKind`: Select,PushPull,Move,Copy,Rotate,Scale,Offset,FollowMe,Eraser,TapeMeasure,PaintBucket.
`ModifySession::new(kind)`, `kind()->ModifyKind`, `set_kind(kind)`, `cancel()`, `active()->bool`, `anchor()->Option<DVec3>`.
`begin(&mut self,model:&Model,selection:&Selection,hit:Option<Hit>,point:DVec3,axis:DVec3)->Result<(),ToolError>` snapshots only selected IDs/face/anchors, no Model/history clone. Axis is face normal for extrusion; chosen inference/construction normal for rotation.
`update(&mut self,point:DVec3)->Result<Option<Command>,ToolError>` produces absolute command from initial anchor. `measure(&mut self,text:&str)->Result<Option<Command>,ToolError>` typed distance, angle degrees or scale factor. Last cursor point determines Move direction; default X if none. Scale uniform around anchor, rotate axis around anchor. `finish(&mut self)->Option<Command>` ends session returning latest command; app uses preview commit instead of executing twice. Tape creates no commands.
`Guide { pub origin:DVec3, pub direction:DVec3 }`; `guides()->&[Guide]`, `measurement()->Option<f64>`, `add_guide()->Result<(),ToolError>` from measured anchor/cursor; guides independent of model/undo. Re-export types at tools root.

No new dependencies, no unsafe, no changes to unowned files. Integrator owns core, app and module declarations.

## Integration additions

`ModifySession::axis()->DVec3` exposes the fixed interaction normal. `set_reference(point:DVec3)->Result<(),ToolError>` establishes the rotation reference/scale handle around the begin pivot. The app uses the selected geometry centroid as pivot; a zero-length handle retains the session default. Push/Pull always uses the actual face canonical normal. `Model::closed_solid_volume()->Result<Option<f64>,KernelError>` reports volume only for orientable closed boundaries without loose wires. Private CSG union/subtract return `Result` after integration so resource-budget exhaustion aborts a transaction.
