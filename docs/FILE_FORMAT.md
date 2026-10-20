# Cindral Drafter files and interchange (M6)

Native `.cdraft` files are ZIP containers containing a compact `manifest.ron` and optional `textures/<index>.png|jpg` entries. The archive wrapper is version **2**, independently of the core document schema **1**. The manifest stores units (metres), right-handed Z-up coordinates, complete definition/instance/tag/material values, and asset slots, MIME types and SHA-256 checksums. Encoded source images are embedded once per slot; GPU previews and runtime caches are excluded.

The explicit v1→v2 migration adds coordinate metadata and an empty asset table to the document-only v1 wrapper. Unsupported future wrapper/document schemas fail without replacing the active document. `crates/cindral-io/tests/fixtures/v1-square.cdraft` is an actual saved binary fixture, created for this migration contract; reproduce it with `cargo run -p cindral-io --example make_v1_fixture`. M5 introduced document DTOs, not an earlier released file encoder. Saving migrated files always writes v2.

Saves validate values, write a temporary file in the destination directory, flush/sync it, atomically replace the destination, then sync the directory on Unix. The old destination is never deleted first. Interrupted serialization leaves the previous file intact; filesystem/device durability still depends on the OS. Tests inject a failed write and check both the old bytes and removal of the temporary file. Readers bound compressed/expanded bytes, entry counts, manifest recursion and image dimensions, reject duplicate/unexpected entries and verify image checksums. No ZIP extraction is performed.

Opening a file fits visible geometry and starts fresh undo/redo history at the root edit context. Subsequent edits and undo operate normally. Selection, navigation, camera, construction guides, pending drags and CPU/GPU caches are session state. Native round-trip tests compare every serialized value, including nested/mirrored instances, shared definitions, tags, materials and encoded textures.

## Autosave and recovery

The editor snapshots committed values after 30 seconds when the document has changed and no modify/instance drag is active. Compression, validation and durable writing run on a background thread; DTO snapshot creation occurs on the UI thread. Recovery files use the same ZIP format and atomic protocol, with `.cdraft.autosave` names. Each session/document gets a distinct snapshot; New/Open first preserve the outgoing nonempty draft. Startup offers recovery, and the File menu can refresh, recover or discard snapshots. Recovery opens an unsaved document with fresh history and retains the original snapshot. A successful manual save removes its superseded snapshot when no autosave is in flight.

Recovery directory: `$XDG_STATE_HOME/cindral-drafter/recovery` when set; otherwise Linux `~/.local/state/cindral-drafter/recovery`, macOS `~/Library/Application Support/CindralDrafter/recovery`, Windows `%LOCALAPPDATA%/CindralDrafter/recovery`. Recovery never silently overwrites the user's saved draft. The most recent edits within the autosave interval can be lost on a crash.

## Textures

PNG/JPEG images can be loaded from the Materials panel and assigned through material painting. Content-derived slots deduplicate loaded bytes; undo retains assets for restoring assignments. Limits: 127 assets, 32 MiB encoded per asset, 4096 pixels per dimension, and bounded decoded allocation. Original images are saved/exported; the renderer uses 256×256 sRGB previews with repeat sampling, planar local coordinates and image alpha. Layer0 is white. Texture slot strings without an embedded asset (legacy DTOs) display untextured.

## Interchange

- OBJ: visible occurrences are flattened into triangulated objects/groups, with transformed vertices/normals and corrected mirrored winding. A sibling MTL and copied source images carry material color, opacity and diffuse textures. Metres, right-handed Z-up; OBJ has no standard unit metadata, so comments document this convention. Sidecars are individually atomic; the multi-file export is not a filesystem transaction.
- STL: binary facets, transformed and flattened, in metres/right-handed Z-up. STL does not encode units, colors, textures, hierarchy or instancing; configure the receiving application accordingly.
- glTF 2.0: self-contained `.gltf` with embedded buffers/images, shared meshes/accessors and nested placement nodes. A root transform maps `(x,y,z)` to `(x,z,-y)` for metres/right-handed Y-up. Material variants reuse geometry accessors; color, opacity and diffuse images are exported as PBR base color. Hidden/tag-invisible instances are excluded. Local shear matrices or transforms that cannot be represented nonsingularly in float32 are rejected explicitly; OBJ/STL can flatten such affine placements.
- OBJ import: positive/negative `v/vt/vn/f` references, planar polygons, `o/g` private groups, MTL `Kd/d/Tr/map_Kd`, epsilon welding and bounded reads. Associated files must remain under the OBJ directory. String imports perform no external reads. The current native face model regenerates planar UVs and face normals; arbitrary imported UV seams and authored smoothing normals are not retained. Unsupported OBJ primitives/directives are ignored; nonplanar/invalid faces are rejected.


Validation uses the [Khronos glTF Validator](https://github.com/KhronosGroup/glTF-Validator/blob/main/node/README.md) in addition to Rust format/geometry tests. The M6 textured mirrored-component export and empty scene produced zero errors and warnings (validator2.0.0-dev.3.10); the empty scene has one informational empty-node notice. The development validation package is external tooling, not a runtime dependency.
