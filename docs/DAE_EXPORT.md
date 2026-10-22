# COLLADA 1.4.1 export (E1)

Use **File > Export > COLLADA…** or the display-free CLI:

```sh
cargo run --release -p cindral-app -- --export-dae in.cdraft out.dae
cargo run --release -p cindral-app -- --export-dae in.cdraft out.dae --units inches --triangulate-all --force
```

| Flag / dialog option | Meaning |
| --- | --- |
| `--units meters` / Meters | Default: native positions, explicit `meter="1"`. |
| `--units inches` / Inches | Positions and every translation divided by 0.0254; `meter="0.0254"`. |
| `--triangulate-all` | Triangulate all faces; otherwise convex faces use polylist. |
| `--include-hidden` | Include hidden/tag-hidden containers and hidden loose edges; default off. |
| `--selection-only` | Export only the explicit selection; empty selection means empty scene. |
| `--face ID`, `--edge ID`, `--instance ID` | Repeatable context-local selection IDs; require `--selection-only`. |
| `--context ID/ID` | Root-relative instance path for selection; requires `--selection-only`. |
| `--no-textures` | Omit image files and use material color/opacity. Textures default on. |
| `--force` | Permit replacing the destination; the dialog asks for confirmation. |

CLI input must be `.cdraft` and output `.dae`. Invalid/duplicate flags fail before
loading the source. Paths retain platform-native encoding. Selection IDs refer to
native entity IDs, not Outliner row numbers. In the dialog, selected faces/edges
use the active edit context; an Outliner selection uses that container's parent
context. Selecting a container exports its complete subtree. Selected vertices
alone have no exported surface or line. Selecting a hidden context requires
Include hidden. Exported selections retain their world placement.

`cindral_io::export_dae(&Path, &DraftBundle, &DaeOptions, &DaeControl)` returns
`cindral_io::Result<ExportSummary>`. Use a fresh control per export. Settings and
selection types are defined in [the frozen boundary](E1_INTERFACES.md).

## Coordinates and representation

Both unit modes use **right-handed Z_UP**, matching native coordinates. Normals
and UV coordinates do not change with output units. Matrices are serialized row
by row as required by COLLADA; local transforms, including mirrors, rotations,
nonuniform scale and shear, are retained. Source asset timestamps are unavailable,
so required created/modified metadata uses a reproducible Unix epoch.

Convex planar faces use `polylist`; concave faces and faces with holes use core
triangulation with winding aligned to the face normal. Holed/concave faces can
therefore become several imported faces. Position sources share native vertices;
normal/UV indices are independent so face corners do not duplicate positions.
Loose edges use `lines`. Groups become placement nodes; component definitions
and inherited-paint variants live in `library_nodes` with `instance_node`
references. Paint variants reuse geometry sources. Export cost depends on unique
definitions/paint variants plus structural instances, not expanded surfaces.

Lambert effects carry linear diffuse colors and scalar transparency. Textures
use relative paths inside an exclusively allocated adjacent `cindral-textures-*`
directory. PNG/JPEG source bytes are copied when untinted. Because standard
COLLADA diffuse permits color OR texture, native linear tint is baked into an
sRGB PNG when required; image alpha is retained and referenced by the transparency
channel. This preserves the image appearance but replaces the original textured
material's scalar tint with baked pixels. Current dominant-axis planar UVs are
written with `TEXCOORD` and explicit material UV bindings. No images are written
when textures are disabled.

## Safety and execution

The desktop takes a document/selection snapshot, then validates, serializes and
writes in a worker thread with progress and cooperative cancellation. Snapshot
cloning remains foreground work. Cancel and publication share a lock: an accepted
cancel prevents publication; after publication Cancel reports completion.

The XML is written to a synchronized temporary file beside the destination,
then atomically published. Without force, publication cannot clobber a file that
appears during export. Images are staged in an exclusively created directory and
retained only if XML publication succeeds. Failure/cancel removes staged files
and preserves a previous destination. IO errors include the output path and the
OS reason, including full disks or unwritable directories. Filesystem errors that
also prevent cleanup cannot guarantee removal; abrupt process/power loss is outside
the cancellation guarantee. Successful replacements use a fresh texture directory;
old directories remain intact for other documents and may be removed manually
once no file references them. Multi-file crash durability is not claimed.

## What is kept and lost

Kept: current faces/holes, loose lines, container hierarchy and names, shared
components, placement transforms, current face color/material/opacity and planar
texture appearance. Visibility filters determine which containers/loose lines
are included; layer membership and visibility metadata are not exported.

Lost: **soft/hidden edge flags, tags/layers, scenes, styles, section planes and
shadows**. Guides, analytic curves and editor history are outside this surface
format path. Smooth-normal interpolation is not exported; normals are per face.
The native face schema has a single material side, so independent authored back
materials are unavailable. Original source UV seams/tile mapping are also absent;
export cannot recover them. Back-face display and mirrored-node culling depend
on the receiving application's material and transform handling. The I1 importer
currently ignores loose `lines`, and triangulated faces need not return to their
original polygon count. No native SketchUp application or cross-platform runtime
validation is claimed.

This implementation uses the [public Khronos 1.4.1 specification](https://www.khronos.org/files/collada_spec_1_4.pdf),
without Trimble SDK, SketchUp binaries/code or branding. There is no affiliation.

## Validation and measurements

Targeted tests cover nested components, mirrored/scaled placements, holes,
concavity, material inheritance, texture alpha/UVs, empty/selected output,
visibility, cancellation and publication races. Round trips use the I1 COLLADA
importer: bounds, geometry-bearing occurrences, materials and emitted face counts
are compared, accounting for mandated triangulation. Run:

```sh
cargo test -p cindral-io --test dae_export
cargo run --release -p cindral-io --features bench-fixtures --example dae_benchmark
scripts/validate-dae.sh model.dae
```

The validator uses installed xmllint with networking disabled. Set
`COLLADA_141_SCHEMA` to an existing offline 1.4.1 XSD for schema validation;
otherwise it explicitly skips the schema step. See [benchmark evidence](DAE_BENCHMARK.md).
