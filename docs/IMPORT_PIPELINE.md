# General import pipeline (I1)

`cindral-io::ImportedScene` separates parsers from model construction. Indexed polygons
(including holes), styled edges, reusable definitions, local instance transforms,
materials, encoded texture assets and tags enter one converter. Native coordinates
are meters, right handed, Z up. The neutral DTO defaults to meter/Z up; glTF uses
meter/Y up; COLLADA without metadata uses meter/Y up. Unit and axis conversion
conjugates instance matrices as well as transforming local vertices, preserving
nested translations, mirrors and shared geometry.

Default cleanup joins disjoint coplanar triangle pairs into polygons and marks
remaining compatible coplanar seams soft/smooth (hidden in ordinary shaded styles).
It does not compute maximal coplanar regions. Material and color boundaries
are preserved. Source UV sets and their seams are not represented by the current
planar texture mapping model. The two cleanup switches are independent.
A scene may remain a single wrapping component with nested shared definitions or be
flattened into loose root geometry. Flattening is bounded by expanded geometry limits.

`BulkGeometry::build_cancellable` welds positions within `TOLERANCE` (1e-7 meters)
using neighboring spatial-hash cells, then constructs unique boundaries directly.
No per-triangle edit commands, intersection splitting or planar-region reconstruction
run during import. Surface mesh validation checks positions, boundary identity,
planarity, winding, triangulation/area and runtime indices. Unsplit crossing edges are rejected during the final validation with a bounded
intersection-candidate budget; they are never split during construction. Intersecting
source surfaces should be separate components.
The history uses ordinary entity/organization deltas with one global undo entry;
later edits and their redo stacks survive undo/redo across the import.

Limits: 64 MiB primary/combined resources, one million source vertices/faces,
four million polygon indices, three million explicit edges, 100,000 structural nodes,
64 nesting levels and 127 PNG/JPEG textures. Local assets must remain inside the
source directory after canonicalization. Absolute/network/traversal paths and symlink
escapes are rejected. Encoded and decoded texture limits are also enforced. No format
SDK or external modeling application is used. Independent SketchUp `.skp` compatibility
is permitted by project policy but is outside I1; no affiliation is implied.

The release regression is explicitly invoked with:

```sh
cargo test --release -p cindral-core --test import_performance -- --ignored --nocapture
```

It constructs 100,000 triangles in a connected 250×200-cell sheet, welds and cleans
it, constructs import history, then draws a chord on a corner face. Budgets are
10 seconds for construction/history and 16 ms for the local kernel edit. Parsing,
GPU uploads, UI event handling and complete frame latency are separate workloads.
For a chord confined to one convex face, the edit cavity follows loose wires and
collects adjacent face boundaries without walking the complete sheet perimeter.
Other edits retain exterior-boundary traversal so remote loops close correctly.

File > Import appends to the current editing context and retains previous history.
Parsing, conversion, append validation, texture decoding and initial CPU mesh caches
run in a worker. UI installation consumes the prepared caches and schedules GPU
updates. Cancellation discards even completed channel results. New/Open and a changed
document revision invalidate pending results so an import cannot overwrite intervening
edits. Source snapshot cloning and GPU uploads remain foreground work.

The File > Import OBJ path also emits an intermediate scene without constructing a
runtime history first; historical `import_obj`/`import_obj_str` bundle APIs retain
their M6 behavior. Sparse glTF accessors, compressed meshes, animation, skins and
non-triangle primitive modes are unsupported. Required unknown extensions fail;
optional unsupported features produce bounded warnings. Alpha MASK is approximated
with blending. COLLADA supports triangles, polylist and simple polygons; controllers,
animations and other unsupported sections are reported. XML is preflighted before
DOM parsing, with DTD/entities disabled and size, element and depth limits enforced.

End-to-end and CPU cache regression commands:

```sh
cargo test --release -p cindral-io --test import_performance -- --ignored --nocapture
cargo test --release -p cindral-app --lib imported_100k_interactive_mesh_update -- --ignored --nocapture
```

The IO workload embeds 200,000 source vertices and 300,000 indices in glTF JSON,
then parses, welds/cleans, validates, records history and appends to a document. The
app workload installs worker-prepared caches and measures a connected corner chord
through the real `DrafterApp::execute` path, including dirty CPU mesh preparation.
These tests enforce a 10-second import budget and a 16-ms edit budget. They exclude
GPU completion, picking/inference, UI event dispatch, vsync and whole-frame latency.
Triangle pair cleanup yields approximately 50,000 faces; exact pair count depends on
mesh connectivity, so the regression checks workload/geometry ranges rather than a
maximal-region face count. Original explicit mesh edges cannot restore a removed
cleanup diagonal or clear a softened seam.

Final release measurements on the local Ryzen 5 2400G machine, sequentially without
concurrent builds/agents during measured execution:

| Workload | Result |
| --- | ---: |
| glTF 100k triangles: parse + conversion + validation/history | 3.609 s |
| Same import including append to destination/history | 5.707 s |
| Connected chord edit in that imported component (kernel) | 0.212 ms |
| Imported sheet edit + dirty CPU mesh preparation (app) | 2.123 ms |

Raw evidence: [glTF import](benchmarks/2026-10-21-i1-gltf-import.txt) and
[app edit/CPU mesh](benchmarks/2026-10-21-i1-import-edit.txt). These are single
regression samples, not distribution or full-editor frame-rate claims. The final
ID-preservation correction affects loose append only; these component workloads
are unchanged. Loose append retains existing geometry identities and welds incoming
vertices into the indexed model using its allocator high-water mark.


## I2 SketchUp extension

`.skp` dispatch uses the independent `cindral-skp` crate and retains authored
polygons/holes, so it disables both coplanar cleanup options. The adapter applies
inch-to-meter conversion through the same scene basis boundary. Tags on loose
geometry and hidden faces become child containers. Imported placements can now
carry instance paint and hidden state; explicit edge smooth and hidden fields
are independent throughout conversion/append/persistence. Open routes `.skp` to
the asynchronous append worker, with a read-only source note and `.cdraft` save.
See [the reader contract and limitations](SKP_IMPORT.md).
