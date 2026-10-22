# E1 COLLADA export boundary

Public API: `export_dae(path, bundle, options, control) -> Result<ExportSummary>`.
`DaeOptions`: `units: DaeUnits` (Meters/Inches), `triangulate_all`,
`include_hidden`, `include_textures`, `force`, `selection: Option<DaeSelection>`.
`DaeSelection`: `context: InstancePath`, selected `faces: BTreeSet<FaceId>`,
`edges: BTreeSet<EdgeId>`, `instances: BTreeSet<InstanceId>` in that context.
`None` exports the document; an empty selection exports an empty scene.
`DaeControl`: cloneable worker progress, cancel and committed-state arbitration.

Native right-handed Z-up coordinates remain Z_UP. Meters use unit meter=1;
inches divide coordinates/translations by 0.0254 and declare meter=0.0254.
COLLADA matrices serialize row by row. Definitions/paint variants share geometry;
component occurrences reference library_nodes. Planar convex faces use polylist;
concave/holed faces use core triangulation. Separate position/normal/UV indices.
Current native faces have one material side and planar UVs only: separate authored
back-side materials and source UV seams are unavailable, so cannot be recovered.
No core schema expansion is part of E1.

Writes stage a complete XML file and encoded image directory beside destination.
Exclusive no-clobber publication by default, replacement only with force; cancellation
arbitrates with final publication. Newly published images roll back if XML fails.
Existing unrelated assets are never replaced. Worker owns validation and IO.

CLI: --export-dae INPUT OUTPUT [--units meters|inches] [--triangulate-all]
[--include-hidden] [--selection-only] [--face ID] [--edge ID] [--instance ID]
[--context ID/ID] [--no-textures] [--force]. Selection IDs are context local.
