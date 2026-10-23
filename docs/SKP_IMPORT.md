# Editable SketchUp import (I2)

`cindral-skp` independently reads the binary containers into shared local
polygons. It is Rust only, forbids unsafe code, has no native dependency or
SDK, and never invokes Python in the application. `cindral-io` adapts it to
I1 `ImportedScene`; File > Import and the Open dialog accept `.skp`.
Opening asynchronously replaces the draft, with loose source geometry at root and
shared components retained as instances. Enter a component before editing its faces.
File > Import still appends; its component option wraps the source once.
The source is never a
native save target: the status/report say “imported from .skp” and Save As uses
`.cdraft`. Importing into an existing draft preserves its native save target.
There is no `.skp` export. This project is not affiliated with SketchUp or Trimble.

## Version policy and evidence

| Container | Accepted header versions | Evidence and limits |
| --- | --- | --- |
| Classic MFC CArchive | 3–8, 13–20 (SketchUp 3–8 and 2013–2020) | Variable preambles, persistent IDs, store-map backrefs, early inline textures, active-layer bootstrap and 2019/2020 padding handled. Reference synthetic models exercised majors 3,4,5,6,7,14,19,20. These are compatibility targets, not certification of every file/schema. |
| Modern VFF + embedded ZIP | 21–26 (2021–2026) | Bounded ZIP extraction and model.dat TLV records. Independently authored annulus/rotated-instance seeds exercised headers 21,22,25,26; this tests layouts/header handling, not models saved by every application release. |

Missing/unknown versions, 9–12 (not known release families), and future majors
27+ fail with a named unsupported-version error. Missing containers and malformed
archives also return errors carrying the detected version where available. The
reference advertises modern 2021+ and legacy 2013–2020, with Python extensions for
earlier versions; it documents a remaining real V3 parsing gap and legacy dialect
failures. This port does not claim to remove those gaps.

The only format reference was the external MIT OpenSKP checkout at revision
`525093be16d52f64e0f0ccb2e56cc91648eabeb9`, especially its Python `_core.py`,
`legacy.py`, `vff.py` and reader docs. Source code takes precedence over stale
format-table details: coedge orientation bit **0 is forward**, and a placement
uses the row-major 3×3 and translation in the first twelve of thirteen doubles;
the final stored scalar is not multiplied in again. No SDK, application binary,
disassembly, reference assets or branding enter the implementation.
See [MIT attribution](LICENSES.md).

## Imported values

- Shared component/group definitions, nested placements, mirrors, nonuniform
  scales and affine transforms; recursive, invalid and over-deep links warn and
  are removed. One explicit model-root definition avoids duplicating placed
  geometry. Instance paint, per-instance hidden state and instance tags survive.
- Authored outer loops and holes, reconstructed from directed edge uses. Loop
  continuity and closure are checked; a corrupt hole discards its entire face.
  The adapter chooses the outer modern loop by area. No input triangulation or
  coplanar merging is required; file dispatch disables both I1 cleanup options.
- Loose edges and independent soft/smooth/hidden flags. Hidden now has an explicit
  native field with a backward-compatible false default; split/heal and append
  preserve it and the renderer suppresses its line slots.
- Material names, sRGB colors converted to native linear RGB, opacity and embedded
  PNG/JPEG textures. Transparency is gated by the source use flag. Front material
  is preferred, with back-only material used when available. Invalid images warn
  and fall back to color; encoded assets become native content-addressed assets.
- Tag names/visibility. Explicit loose-face or loose-edge tags become tagged child
  containers, consistent with the kernel's container-only tag model. Hidden faces
  likewise become hidden children. Layer0/Untagged inherits the containing tag.
  A face's boundary inherits the face container; a separate boundary-edge tag is
  cosmetic and is not retained independently.
- SketchUp stores **inches regardless of display-unit preference**. Raw positions
  and placement translations remain inches/Z-up until I1 applies exactly
  **0.0254 meters per inch** to vertices and conjugates placements by the same
  unit basis. Source display-unit preferences are not restored.

## Recovery and resource limits

64 MiB source and total expanded ZIP bytes; 4096 ZIP entries; 1 million aggregate
geometry/work items; bounded borrowed TLV trees and traversal work; 64 nested
object/TLV levels. The destination also limits definitions/placements to 100k and
hierarchy depth to 64. Legacy slot-base probing permits at most 32 candidates;
back-reference burn bands use binary search. Loops/kernel rings and aggregate
polygon validation/recovery work are capped. Material XML is limited to 1 MiB,
64 levels and 100k elements, with a quote/comment/CDATA-aware preflight before
DOM parsing; DTD/entity declarations are rejected. Texture decoding uses existing
4096×4096 per-image limits, 127 assets and a 256 MiB aggregate decoded-pixel
budget. Nothing from an archive is extracted to disk or fetched over the network.

Known-length malformed records are isolated where possible. Reader diagnostics
are bounded to 256 messages; adapter polygon repairs/skips are individually
reported without truncation. Invalid nondegenerate polygon recovery stops the
import with an explicit error rather than dropping geometry. Hard container/resource limits
return errors. Invalid TLV lengths stop that sibling run; unknown/unreadable
legacy classes stop the archive walk and return recovered definitions/root
records with an explicit partial-import warning. Neither format safely permits
inventing an unknown record's boundary. A damaged stream can therefore omit
later geometry; the report must be reviewed. Cancellation is checked before/
after binary decoding and during adapter validation and I1 conversion; an active
binary decode finishes its bounded stage before observing cancellation.

## Cross-checks and local fixtures

`tests/fixtures/skp-local/` was created and gitignored; it was empty during I2.
Local-file tests skip gracefully when empty. Tests commit authored byte builders,
not binary reference/proprietary models. Reference-wide MIT licensing was found,
but the mixed real-model fixture corpus has no established model-by-model grant;
none is redistributed. The dev checks read only clearly identified synthetic
reference fixtures in place (their tests describe their authored geometry and
project-generated 8×8 texture), never their SDK validation tooling.

```sh
# Optional dev environment; never an application runtime dependency.
python3 -m venv .agent-out/skp-python
.agent-out/skp-python/bin/pip install -e ../openskp-ref/packages/python
.agent-out/skp-python/bin/python scripts/compare_skp.py
# Or provide explicit files and/or --reference PATH.
```

The script builds the Rust `skp_inspect` release example, runs OpenSKP's Python
reader, and compares unique authored face counts, source placement counts and
placed visible-occurrence bounds in meters (1e-7 absolute / 1e-8 relative
tolerance). Added tag containers are excluded from source placement counts.
Mismatch/error returns nonzero; absent local files print SKIP and return success.
`--no-build` uses an already-built inspector. Both counts and bounds passed on
seven synthetic hierarchy fixtures, four synthetic texture fixtures, and four
authored modern header variants; no mismatch required a geometry correction.
The [15-case comparison output](../benchmarks/2026-10-22-i2-skp-crosscheck.txt) is retained as text evidence.
Authored tests separately verify holes/area, unit conversion, damaged-face
recovery, native round trip, tags/hidden faces, instance paint, cancellation,
version errors, ZIP expansion claims and mutations/deep records. No user-file,
production-model or cross-platform runtime claim is made.

## Performance and remaining gaps

Release workload: 19,600 connected authored quads (4,687,806 model-record bytes,
680,689 compressed file bytes), normal parser/converter/append and a chord edit.
The app workload measures a real kernel edit plus dirty CPU mesh preparation.
Final timings, commands, hardware and exclusions are recorded in
[benchmark evidence](../benchmarks/2026-10-22-i2-skp.txt). These are synthetic CPU
workloads, excluding picking/inference, GPU completion, UI/vsync and entire-frame
latency. They do not guarantee seconds or 16 ms for arbitrary production models,
maximum-size inputs, large affected faces or dense geometry. There were no user
local files available for a representative large-model measurement.

Not imported: scenes/cameras, styles, section planes, dynamic-component behavior
or custom dictionaries, text/dimensions, construction guides, analytic curve
parameters, animation, face-me/billboard behavior, tag colors/folders, source
display units, distinct front/back rendering, authored texture UV pins/projection
or tile sizes. Image-entity definitions can contribute ordinary geometry/images,
but special image/billboard behavior is not reproduced. Textures use native
planar mapping; colorized/hue-shift materials are approximated by ordinary
material tint rather than the source colorization algorithm. Unsupported image
codecs are skipped. Unknown legacy record classes/dialects may yield partial
geometry as described above. No `.skp` writing or export is provided.

### Import summary counts

Vertices are welded positions, edges are unique topological boundary/loose edges,
and faces are kernel polygons after cleanup. Each stored definition (including
root geometry and hidden/unplaced definitions) contributes once, regardless of
how many instances reference it. Instances count stored placement records across
all definitions, including an optional `Imported scene` wrapper and containers
created to preserve per-primitive tags. They are not expanded visible occurrence
counts. Native SKP root promotion preserves components/groups as shared definitions;
`as_component: false` opens the native root directly without flattening it.
The synthetic reader root is removed during promotion. The generic non-SKP
flatten option intentionally expands occurrences.

For the private mybedroom model, raw reader counts are 1760 vertices, 2021 edges,
305 faces and one authored component placement. Before repair, 304 faces survived;
Open produces one instance, while Import as Component produces two (one wrapper).
SketchUp Model Info Statistics were not supplied, so that comparison is unknown.


### Polygon repair policy

Source topology can contain weakly simple loops with retraced bridges. Split
those walks at repeated tolerance-welded vertices into simple outer/hole rings;
preserve all original explicit edges, including bridge edges and their styles.
For a nonplanar polygon, compute the least-squares plane (smallest covariance
eigenvector over all boundary vertices in meters). If its maximum deviation is
at most **0.0254 mm (0.001 inch)**, project onto that plane. This adapter policy
uses the approximate SketchUp tolerance supplied for this task; it is not a
claim about SketchUp's internal algorithm. Kernel validation remains 1e-7 m.
Otherwise earcut the validated projected footprint, including holes, and build
planar triangles from the original 3D positions, preserving material and color.
Verify projected coverage before accepting triangles. Preserve explicit source
edges. Only degenerate outer loops (zero area at kernel precision or fewer than
three distinct vertices) are skipped. Nondegenerate invalid footprints, missing
coordinates and exhausted work budgets fail explicitly without partial loss.
Every adapter repair/skip includes entity identity (or an unavailable-ID marker),
parent definition/placements, tag, local position and reason. Per-face assignments
are replicated onto all resulting triangles before tag/hidden partitioning.

The private model's face 44340 is already planar. Four vertices are revisited
exactly across a retraced bridge, rather than newly collapsed by welding. Simple
boundary decomposition restores it without moving any source position or adding
edges: Open changes 1760/2021/304/1, skipped 1 to 1760/2021/305/1, skipped 0.
