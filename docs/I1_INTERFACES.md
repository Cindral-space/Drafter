# I1 frozen import contract

All formats produce `ImportedScene` (cindral-io import_scene module, reexported).
DTOs (all Clone, Debug, Default except UpAxis):
- ImportedScene { definitions: Vec<ImportedDefinition>, roots: Vec<ImportedInstance>, materials: Vec<cindral_core::Material>, textures: BTreeMap<String, TextureAsset>, tags: Vec<cindral_core::Tag>, meters_per_unit: f64, up_axis: UpAxis, warnings: Vec<String>, skipped: usize }. Default meters=1, Z up.
- ImportedDefinition { name: String, geometry: cindral_core::BulkGeometry, instances: Vec<ImportedInstance> }
- ImportedInstance { name: String, definition: usize, transform: glam::DMat4, tag: Option<usize> }. Default identity.
- BulkGeometry { vertices: Vec<DVec3>, faces: Vec<BulkFace>, edges: Vec<BulkEdge> }
- BulkFace { outer: Vec<usize>, holes: Vec<Vec<usize>>, material: Option<MaterialId>, color: [u8;4] }. MaterialId(index+1) refers to scene materials.
- BulkEdge { vertices: [usize;2], soft: bool, hidden: bool }
- UpAxis::{X,Y,Z}
- ImportOptions { merge_coplanar: bool, soften_coplanar: bool, as_component: bool }; default true/true/true.
- ImportControl: Clone+Default; cancel(), is_cancelled()->bool, progress()->f32. Shared atomics; check()->Result<()>.
- ImportReport { vertices, edges, faces, instances, skipped: usize, warnings: Vec<String> }
- ImportResult { history: DocumentHistory, textures: BTreeMap<String,TextureAsset>, report: ImportReport }
- convert_scene(scene, &ImportOptions, &ImportControl)->Result<ImportResult>
- import_file(&Path, &ImportOptions, &ImportControl)->Result<ImportResult>
- import_gltf(&Path, &ImportControl)->Result<ImportedScene>, import_gltf_bytes(&[u8], Option<&Path>, &ImportControl)->Result<ImportedScene>
- import_dae(&Path, &ImportControl)->Result<ImportedScene>, import_dae_bytes(&[u8], Option<&Path>, &ImportControl)->Result<ImportedScene>
Limits: 64MiB main/combined assets, 1M vertices/triangles, 100k nodes, 64 hierarchy/XML depth, 127 textures. No absolute/traversal/network asset references; bounded reads. DAE rejects DTD/entities. Unsupported features warn. No panic on malformed input.
Core builds without intersection reconstruction; import is one undo entry. Core meters/Z up. glTF defaults meter/Y up, DAE missing metadata meter/Y up. Transform basis conversion applies to both vertices and placements.
Ownership: parser task owns import_gltf.rs/import_dae.rs and parser fixtures/tests only; UI task owns files_ui.rs only; integrator owns core/DTO/converter/registration/dependencies/docs. No agent commits or workspace-wide checks; targeted checks only after files exist. OpenCode disabled.

Final integration: core `append_import` prepares graph/geometry changes in one document
entry and preserves the previous timeline. The UI worker appends to its source snapshot,
merges assets and prepares CPU render caches before publishing a result. A changed revision
invalidates the result. XML depth is checked in a markup-aware preflight before DOM parsing.
