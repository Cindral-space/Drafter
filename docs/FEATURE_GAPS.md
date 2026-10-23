# Feature gaps for house, interior and furniture visualization

**Inventory: 116 scoped rows — 25 Have, 42 Partial, 47 Missing, 2 Unsure.**

Audit date: 2026-10-23. Baseline: the current checkout, including the completed measurements-box and Eraser fixes. Purpose: make rooms and furniture accurately, compare ideas, and explain them to other people. This is an audit only; no feature implementation or runtime SketchUp comparison was performed. SketchUp feature descriptions below are independently paraphrased. No SDK, application binaries, vendor code, icons or assets were used.

## How to read this audit

**Have** means the named capability is implemented and exposed, within the stated scope; it does not certify complete SketchUp parity. **Partial** identifies an implemented foundation and the missing user workflow. **Missing** means no implementation/action was found in the relevant registry, menu, panel, schema or format dispatch. **Unsure** means reference behavior or comparable capability could not be established; questions follow. Absence claims are scoped to inspected application surfaces and their underlying types, not a claim that every source file was read.

Priorities are personal: **P0** daily modeling or showing a plan; **P1** useful; **P2** occasional; **Skip** outside this use case. A P0 existing capability is worth keeping, not a request to rebuild it. Complexity estimates apply to the **remaining gap**, including meaningful validation: **S** under a day, **M** a few days, **L** a week or more, **XL** design/new subsystem needed. Existing capabilities use **—**. Estimates assume familiarity with this Rust workspace and are not promises that M/L/XL work fits one usage window.

Tier notation: **F/G/P** = Free web, Go web/iPad, Pro desktop (and the applicable lower-tier products). **P verified; web unsure** means the desktop feature is established but its exact current web availability was not verified. It does not mean Free lacks it. An extension is explicitly distinguished from a built-in feature. Platform differences matter, especially PDF and web versus desktop controls.

### Reference checks

Current official sources were checked on the audit date. Where sources disagree, the discrepancy is retained rather than guessed:

- [Web features and tier gates](https://help.sketchup.com/en/sketchup-web/web-features): paid web unlocks Outliner, tag folders, material/style authoring and the full solid tool set; Free has Outer Shell. Dynamic Components and extensions require desktop. Free can use Live Components; modification requires a subscription.
- [Web expanded toolset](https://help.sketchup.com/en/sketchup-web/sketchup-web-expanded-toolbar): confirms the additional drawing, annotation and navigation tools listed below. A listed tool is not itself proof that every submode has the same tier entitlement.
- [Interoperability by subscription](https://help.sketchup.com/en/sketchup/using-sketchup-data-other-modeling-programs-or-tools) and [web file management](https://help.sketchup.com/en/sketchup-web/file-management): distinguish formats and direction. The web feature matrix says STL-only export, while file management lists PDF and the same feature matrix separately allows PNG. Accordingly PNG is established, PDF availability needs a trial. File-list typos are not treated as format specifications.
- [GLB workflow](https://help.sketchup.com/en/sketchup/working-gltf-files): current Pro documentation describes binary GLB import/export. This supersedes an assumption that all glTF support requires extensions.
- [Camera controls](https://help.sketchup.com/en/sketchup/viewing-model), [scenes](https://help.sketchup.com/en/sketchup/creating-scenes), [materials and environments](https://help.sketchup.com/en/sketchup/adding-colors-and-textures-materials), [photo matching](https://help.sketchup.com/en/sketchup/matching-photo-model-or-model-photo), and [terrain](https://help.sketchup.com/en/sketchup/sculpting-and-fine-tuning-terrain) support the corresponding presentation/view rows.
- [Dynamic-component authoring](https://help.sketchup.com/en/sketchup/making-dynamic-component), [welding edges](https://help.sketchup.com/en/sketchup/dividing-splitting-and-exploding-lines-and-faces), and [model cleanup](https://help.sketchup.com/en/3d-warehouse/minimize-tag-usage) support the organization/modify/productivity comparisons. Established everyday desktop behavior also informs the table; unresolved details are explicitly placed under Unsure.

### Drafter evidence key

Paths are repository-relative. `:number` identifies a starting line; section/function names also locate evidence. References to earlier parity audits use their behavior IDs, since source line numbers in those audits predate recent changes.

| Key | File / inspected scope |
| --- | --- |
| W | `crates/cindral-app/src/workspace_ui.rs`:70 drawing/modify registry; :101 menus; :325 construction planes; :528 inspector; :651 measurements/status |
| A | `crates/cindral-app/src/lib.rs`:404 `viewport_input`, especially :449 shortcuts and :496 constraints/history |
| O | `crates/cindral-app/src/organization_ui.rs`:35 single-container selection state; :126 create; :154 context entry; :237–590 Outliner/Tags/Materials; instance input/transform handlers |
| M | `crates/cindral-app/src/modify_ui.rs`: input, previews, selection at :240, guides at :438; `crates/cindral-tools/src/modify.rs`:19 `ModifyKind`, :46 Guide, transform/offset/sweep adapters |
| D | `crates/cindral-tools/src/session.rs`:10 `ToolKind`, drawing state and completed-operation adapters |
| K | `crates/cindral-core/src/command.rs`:7 `Command`; `crates/cindral-core/src/document.rs`:33 `OrganizationCommand`; `crates/cindral-core/src/model.rs`:48 Edge/:65 Face; `crates/cindral-core/src/document_data.rs`:103 Instance/:121 Tag/:129 Material |
| V | `crates/cindral-render/src/camera.rs`:40 fixed `FOV_Y`, :90 Projection, :141 Camera, navigation methods |
| P | `crates/cindral-app/src/presentation_ui.rs`: scene capture/restore and :220 panel; `crates/cindral-core/src/presentation.rs`:6 styles, :52 camera, :66 scenes |
| S | `crates/cindral-app/src/section_ui.rs`:82 panel, cap-cache functions; `crates/cindral-app/src/shadow_ui.rs`: shadow panel; `docs/ARCHITECTURE.md` M7b |
| F | `crates/cindral-app/src/files_ui.rs`: file menu/actions, :410 import dispatch, :423 exports, :887 texture loading; `crates/cindral-io/src/lib.rs`:178–191 public format modules |
| E | `crates/cindral-io/src/export.rs`:143 OBJ, :339 STL, :460 glTF; `crates/cindral-io/src/export_dae.rs`; `docs/DAE_EXPORT.md` |
| I | `docs/IMPORT_PIPELINE.md`; `docs/SKP_IMPORT.md`; `crates/cindral-io/src/import_scene.rs`; `crates/cindral-io/src/import_gltf.rs` |
| B | `docs/MEASUREMENTS_BOX.md` and `crates/cindral-app/src/measurements_ui.rs`; current structured input supersedes old scalar-input findings |
| T | `docs/TOOL_PARITY/` audits: `draw_tools.md`, `select.md`, `transform_tools.md`, `utility_tools.md`, `offset_follow_me.md`, `push_pull.md`, `push_pull_cleanup.md` |
| R | `docs/ARCHITECTURE.md`, `docs/FILE_FORMAT.md`, recent `docs/AGENT_LOG.md`; `docs/REAL_FILE_PROFILE.md` and parity release measurement files |

## Feature table

### Drawing and precision

SketchUp's basic drawing and annotation tools are established in its [web toolset](https://help.sketchup.com/en/sketchup-web/sketchup-web-expanded-toolbar); the distinctions here are workflow/functionality, not icon or wording parity.

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Line, planar Rectangle, Circle, Polygon, center Arc | F/G/P | Have — five tools, typed geometry, automatic planar faces | W:70; D; B | — | Kernel, inference | P0 |
| Rotated Rectangle | F/G/P tool; exact modes unverified | Missing — only opposite-corner Rectangle | W:70; D:10 | M | Plane basis, snapping | P1 |
| 2-point / 3-point Arc and Pie | F/G/P tools | Missing — only center/start/end Arc | W:70; D:10 | L | Shared curve construction, input stages | P1 |
| Freehand | F/G/P tool | Missing | W:70; D:10 | M | Stroke sampling, simplification | P2 |
| Editable curve identity / radius and segment properties | P verified; web property behavior unsure | Partial — tessellated edges and immediate re-entry; no persistent Curve entity or later radius edit | D emits AddPolyline; K Edge; B | XL | Curve metadata, topology/history/persistence | P1 |
| 3D Text | F/G/P tool | Missing | W registry; D; K schema | XL | Font outlines, tessellation, holes, extrusion | P2 |
| Text / leader labels | F/G/P tool | Missing — no annotation entities | W; K; P schema | XL | Text rendering, anchors, persistence | P0 |
| Persistent linear dimensions | F/G/P tool | Missing — measurement readout is not a dimension annotation | W; K; B | XL | Annotation subsystem, units, entity anchors | P0 |
| Protractor / angular construction guides | F/G/P tool | Missing — typed Rotate angles are not a protractor | W; M:19/:46 | M | Guide entities, plane inference | P1 |
| Relocate/orient drawing axes | F/G/P tool | Partial — preset work planes and local contexts; no arbitrary user axes tool | W:325; A; D `set_plane` | L | Coordinate frames, snaps, persisted axes | P1 |
| Sandbox terrain creation / sculpt / stamp / drape | P desktop; no native web extensions | Missing — no terrain tool family | W registry; K | XL | TIN editing, surface projection, spatial queries | P2 |
| Metric/imperial/fractional entry and document units | F/G/P | Have — units, fractions, locale lists, precision settings | B; W:665 | — | Measurements settings | P0 |
| Live measurements and immediate re-entry | F/G/P; exact phase details Q1 | Have — supported drawing/modify adapters replace one undo operation | B; D; M | — | Completed-operation snapshots | P0 |
| Endpoint/midpoint/edge/face/direction snapping | F/G/P | Have — active-context geometry, axes, parallel/perpendicular hints | R M3; `crates/cindral-inference/src/engine.rs` | — | Inference engine | P0 |
| Shift inference hold and shape-plane arrow locks | P verified; web exact keys Q2 | Partial — directional arrow locks exist; no held inference; shapes retain their plane | A:496; T draw I/shape rows, transform N4/N5 | M | Shared constraint state, plane selection | P0 |
| Snap to geometry outside active editing context | P verified; exact eligibility Q3 | Partial — current inference queries active local model; closed-context targets absent | A `viewport_input`; O context; T draw cross-context row | L | Scene queries, world/local transforms, visibility | P0 |
| Tape edge-offset guides, guide points and measure-only mode | P verified; web modes unverified | Partial — two-point measure/line guides and confirmed resize exist; parallel offsets, point guides and mode toggle absent | M Guide; B; T utility Tape rows | L | Persistent guide model, inference | P0 |
| Guides persist, undo and erase like construction aids | P verified; web details unverified | Partial — session lines render; not serialized/history-owned or guide-picked | M:438; R M6; T utility guide undo; B limits | L | Guide IDs, commands, archive migration, picker | P0 |
| Rectangle proportion / acquired curve-tangent hints | P verified; exact hints Q4 | Partial — in-progress tangent support; no square/golden-ratio hints or analytic existing-curve tangents | T draw rows; D; R M3 | M | Inference candidates, curve metadata | P2 |

### Selection and modification

Solid operations below are separate from intersecting raw faces: the [tier matrix](https://help.sketchup.com/en/sketchup-web/web-features) explicitly gates the paid solid set. Welding creates selectable curve structure, as described in [edge editing](https://help.sketchup.com/en/sketchup/dividing-splitting-and-exploding-lines-and-faces).

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Click/window/crossing/connected selection of loose geometry | F/G/P | Have — raw-geometry scope | `crates/cindral-tools/src/selection.rs`; M; T select | — | Picking | P0 |
| Selection add/toggle/subtract, double-click expansion, Select All | P verified; web modifier mapping Q5 | Partial — Shift adds only; missing toggle/subtract, face/edge double-click expansion and Select All | M:240; selection.rs:42; T select F7/K/S6 | M | Shared selection operations/input | P0 |
| Multiple groups/components and mixed box selection | F/G/P everyday workflow | Missing — one container path; raw selection is separate | O:35; selection.rs; T select S3/S4 | L | Unified selection, instance picking, transform batches | P0 |
| Divide an edge into equal segments | P verified; web menu availability unverified | Missing — no Divide action; adding lines can split edges manually | W; K Command; O/M input | M | Edge splitting, preview and transaction | P1 |
| Lasso selection | Web tool; desktop version/tier details unverified | Missing | W; M selection input | M | Unified selection, polygon selection region | P2 |
| Push/Pull and through-cuts | F/G/P | Have — extrusion, holes, inward cuts, transactional preview; cleanup fixes included | K; R M4; T push_pull_cleanup; recent log | — | Planar topology/solid extrusion | P0 |
| Push/Pull repeat-last and retained-start-face mode | P verified; web modifiers Q6 | Partial — extrusion exists; no double-click repeat or Ctrl start-new section | M; K PushPull fields; T push_pull F3/K1 | M | Tool repeat state; extrusion mode kernel support | P1 |
| Move / Copy / Rotate | F/G/P | Have — loose geometry and individual containers; cancellation and typed entry | M; O instance transforms; B | — | Selection and transactions | P0 |
| Overlapping coplanar loose-copy healing / connected autofold | P verified; autofold cases Q7 | Partial — disjoint copies work; overlapping coplanar copies rejected; general nonplanar autofold absent | T transform G2/G7; B limits; R M4 | XL | Local topology reconstruction, face splitting | P1 |
| Linear copy arrays, multiply and divide | F/G/P; counting details Q1 | Have — shared parser, loose/container arrays, max 256 copies | B | — | Copy batch transactions | P1 |
| Rotate-copy / radial arrays / Move copy toggle | P verified; web modifiers Q5 | Partial — separate translation Copy and linear arrays; no rotate-copy or radial array workflow | M Rotate/Copy adapters; B; T transform M1/M4 | M | Copy mode, angular transform batching | P1 |
| Scale factors, per-axis numbers, target length and reflection | P verified; exact target-axis semantics Q1 | Have — typed factors/lengths and signed reflections; bounded valid transforms | B; M and O scale adapters | — | Measurements, affine transforms | P0 |
| Graphical Scale grips / opposite-anchor / center toggle | P verified; web grip modes unverified | Partial — numerical axes supported; pointer scaling uses radius/center, no selectable grips | B limitations; M; O; T transform I5/M2 | L | Grip picking, pivot/axis state | P0 |
| Offset | F/G/P | Partial — numeric planar offsets; pointer distance uses anchor distance; no repeat-last or open edge-chain workflow | M offset; T offset O11/O13/open-chain row; B fixes only entry | L | Perpendicular-distance solver, chain offset topology | P0 |
| Follow Me with preselected path / interactive path tracing | F/G/P | Partial — ordered selected-edge sweep and straight fallback; no manual traced path; bend/closed-loop parity Q8 | M FollowMe; T offset_follow_me F rows | L | Ordered path state, sweep corner/seam solver | P1 |
| Intersect Faces with selection/model/context | P verified; web scope unverified | Missing — planar line splitting/extrusion CSG is not a general face-intersection action | W; K commands; R M2/M4 | XL | Cross-face intersections, context conversion, surface splitting | P1 |
| Reverse Faces / Orient Faces | P verified; web entitlement unverified | Missing — no exposed normal-edit command | K Face/commands; W | L | Stable face orientation, rendering/export conventions | P1 |
| Weld and Explode Curve | P verified; web entitlement unverified | Missing — no persistent curve ownership to weld/explode | K Edge; D; W | XL | Curve identity, selection, persistence | P2 |
| Soften / smooth edges by selection, angle or Eraser modifier | P verified; web controls unverified | Partial — flags/import/render support; no user editing panel/modifier workflow | K SetEdgeStyle; P display toggles; T utility Eraser modes | M | Edge command UI, angle queries; normals for real smoothing | P1 |
| Explode group/component to parent geometry | F/G/P | Missing — no organization command/menu action | K OrganizationCommand; O:237–590 | L | Transform baking, material/tag inheritance, ID remapping | P1 |
| Hide / unhide objects and loose geometry | F/G/P | Partial — container Hidden checkbox/tags; imported hidden edges; no loose-face/edge authoring or Unhide All/Last | O:398; K Hide/Edge; I SKP visibility | M | Visibility commands, hidden selection, scene restore | P1 |
| Lock/unlock containers | P verified; web availability unverified | Missing — no lock field or action | K Instance; O controls | M | Persisted lock state, all mutation/pick guards | P1 |
| Align faces/objects / mirror by chosen axis | P Flip established; dedicated Align unsure Q9 | Partial — numeric negative scale mirrors; no Flip Along action or alignment workflow | B; W; M; O transforms | M | Bounds, local/world pivots, selection | P1 |
| Solid Union / Intersect / Subtract | G/P; Free has only Outer Shell | Missing — internal extrusion union/subtract is not an object solid tool | W MODIFY; K Command/OrganizationCommand; R M4 | XL | Manifold checks, multi-object selection, CSG/attribute transfer | P1 |
| Solid Trim / Split / Outer Shell | G/P; Outer Shell also Free | Missing — no object-level actions | W; K | XL | Solid operand workflow, robust CSG, result ownership | P2 |
| Eraser click / stroke / soften-hide modifiers | P verified; web exact modes unverified | Partial — current click removes one face or edge; no edge stroke accumulation or style modifiers | M Eraser; T utility; R Eraser targeting/recent log | M | Stroke transaction, edge styles, hit rules | P1 |

### Organization and materials

[Component workflows](https://help.sketchup.com/en/sketchup/adding-premade-components-and-dynamic-components) include reuse and replacement. [Dynamic authoring](https://help.sketchup.com/en/sketchup/making-dynamic-component) is a Pro desktop capability. [Material workflows](https://help.sketchup.com/en/sketchup/adding-colors-and-textures-materials) now also include physically based appearance and environments.

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Groups, shared components, nested edit, Make Unique | F/G/P basic organization | Have — private groups, shared definitions, edit-in-place, reparenting | O; K OrganizationCommand; R M5 | — | Definition/instance graph | P0 |
| Component library/browser, replace/reload, reusable collections | F/G/P reuse; reload details P verified | Partial — duplicate/import/create available; no reusable library browser, replace or source reload | O controls; F import; K | L | Asset metadata, placement preview, definition replacement | P0 |
| Component insertion axes, glue/cut-opening behavior | P verified; web details unverified | Missing — instance transforms exist; no authored insertion/glue/cut semantics | K Instance/DefinitionData; O | XL | Component metadata, host-face links, opening topology | P1 |
| Authored component connection points (Snaps) | P documented; other platform/tier availability unverified | Missing — no connection-point metadata or attach-and-orient tool | K DefinitionData/Instance; O; [official Snaps](https://help.sketchup.com/en/sketchup/snaps) | L | Persisted local frames, placement/orientation solver, picking | P1 |
| Dynamic/attribute-driven components | P desktop authoring; not web Dynamic Components | Missing — no attributes/formula/interaction engine | K document schema; O; R M5 | XL | Attributes, dependency evaluator, parametric regeneration | P2 |
| Live Components | Free use; G/P paid configuration per web matrix | Missing — no connected/configurable live-component system | O; F; K | XL | Parametric engine or external integration | P2 |
| Tags and visibility | F/G/P | Have — named tags on containers, scene capture | O:426; P; K | — | Container organization | P0 |
| Tag folders / dashed tag styles | G/P folders; P dashed styles verified | Missing — flat tags, no folder/dash schema | K Tag; O Tags; P styles | L | Tag tree, visibility inheritance, line renderer | P2 |
| Color / opacity / image materials and paint | F use; G/P author/edit custom materials | Have — create/update/delete, paint faces/containers, PNG/JPEG textures | O:478; F:887; R M6 | — | Material assets/rendering | P0 |
| Texture position / rotation / physical size / projected mapping | P verified; exact paid-web controls Q10 | Partial — dominant-axis planar UVs; no per-face mapping editor; authored import UVs lost | E planar_uv; I; R M6; docs/DAE_EXPORT.md limits | XL | Persisted UV mapping, interactive handles, render/import/export | P0 |
| Material sampler / connected-or-matching paint scopes | P verified; web has Sample Material tool | Missing — palette paint exists; no sampler/modifier scope routing | O paint handler; M Paint; T utility Paint rows | M | Material picking, surface/adjacency traversal | P0 |
| Independent front/back materials and monochrome face checking | P verified; web side controls Q11 | Partial — single face paint, no explicit monochrome/back-face palette; SKP prefers front or back fallback | K Face; P StyleMode; I SKP materials | XL | Two-sided face/material schema, renderer and exporters | P1 |
| Photoreal materials, roughness/metal/normal maps, environments/AO | P current desktop verified; web tier/version Q12 | Missing — color/opacity/diffuse texture only; ordinary shading/shadows | K Material; P style; I import; R M6/M7b | XL | PBR renderer/assets, lighting, import/export | P1 |
| Outliner hierarchy and object naming | G/P; Free web lacks Outliner | Have — tree, edit, rename, reparent and visibility | O:237–425 | — | Instance graph | P0 |
| Outliner search/filter and multi-selection | G/P Outliner; exact search behavior P verified | Partial — virtualized tree and reveal selection; no search, filter or multi-selection | O state/:237–425 | M | Tree filtering; unified selection for multi-select | P1 |
| Entity Info: editable properties, lengths/area/solid volume | P verified; web property coverage unverified | Partial — counts, name/tag/material controls; no consolidated dimensional/area/volume inspector | W:568; O; K model area API | M | Selection summary, units, solid validation | P0 |
| Classifications / attribute reports | P | Missing | K document schema; O panels; F exports | XL | Attribute schema, classifiers, report export | Skip |
| Saved named selections | Native SketchUp capability not established Q13 | Unsure — no Drafter named selection store either | K DocumentData; selection.rs; P scenes store visibility, not selection | XL | Stable selection references, archive/UI | P2 |

### View, navigation and presentation

[Viewing controls](https://help.sketchup.com/en/sketchup/viewing-model) establish adjustable FOV and two-point perspective; [scene documentation](https://help.sketchup.com/en/sketchup/creating-scenes) establishes saved views/properties. Drafter's implemented styles, sections and sunlight are not counted as missing.

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Orbit / pan / cursor zoom / fit model | F/G/P | Have | V; A:404; W Camera | — | Camera | P0 |
| Standard views and parallel projection | F/G/P | Have — seven views, perspective/orthographic | W:139; V:90/:102 | — | Camera | P0 |
| Position Camera / eye height / Look Around / Walk | F/G/P tools | Missing — orbit camera only | W Camera; V Camera/methods | L | Eye/target navigation, height/ground conventions, input | P0 |
| Adjustable field of view / focal length | P verified; web control Q14 | Missing — fixed 45-degree vertical FOV | V:40/:276; P SceneCamera | M | Camera projection state, scene/archive migration | P0 |
| Zoom Window | F/G/P tool | Missing | W Camera; V navigation; A | M | Rectangle gesture, projection fitting | P1 |
| Two-point perspective | P verified; web availability Q14 | Missing — only perspective/orthographic | V Projection; P SceneCamera; W | L | Off-axis projection, ray/pick math, scenes | P1 |
| Section planes, section views and fills | P verified; web tool exists, detailed tiers unverified | Have — place/flip/move, active cut, cached closed-shell fills; one active document plane | S; R M7b | — | Clip-aware render/pick/inference | P0 |
| Nested/context sections and customizable fill/line styling | P verified; web exact scope unverified | Partial — one global active section; no per-context active sections or fill-style editor | P SectionState; S panel/caps | XL | Context section state, multi-plane clipping, style UI | P1 |
| X-ray / wireframe / hidden-line / textured views | F/G/P preset views; paid custom style editing | Have — five modes including shaded; profiles | P:220; R M7a | — | View style uniforms/pipelines | P0 |
| Back Edges / dedicated front-back diagnostic display | P verified; web display choices Q11 | Partial — X-ray/soft-edge display; no distinct occluded dashed Back Edges or monochrome side colors | P StyleMode; renderer mesh edge flags | L | Edge visibility/dash pipeline, face sides | P1 |
| Fog | P verified; web tier unverified | Missing | P ViewStyle; W/P panels | M | Depth-based shading, persisted settings | P2 |
| Scenes: save/update/rename/restore view and visibility | F/G/P | Have — camera, style, tags/objects, section and shadow settings | P SavedScene/panel; R M7a/M7b | — | Presentation snapshots | P0 |
| Scene sequencing, reorder/timing and animation export | P verified; web export entitlement unverified | Partial — named scene selection and fixed 0.25s transition; no timeline/reorder/movie export | P transition/panel; F export actions | L | Scene sequence UI, capture/encoding, timing | P1 |
| Shadows and manual sun/date/location settings | F/G/P shadows; exact UI varies | Have — solar model, lat/lon/time, face/ground controls and quality | S shadow panel; R M7b | — | Shadow map, solar direction | P1 |
| Geolocation map / imagery / site context | Free basic location; G/P expanded site context | Partial — numerical coordinates drive sun; no map, imagery or terrain acquisition | S; F; W | XL | Geographic coordinates, imagery/terrain providers, terms/cache | P2 |
| Sketchy edges / custom line patterns, widths and dash styles | P verified; G/P custom web styles, exact options unverified | Partial — silhouette profiles and visibility toggles; no sketchy strokes or user dash/width controls | P ViewStyle/panel; R M7a | XL | Stroke renderer/style schema | P2 |
| Background / sky / ground colors | F presets; G/P custom web editing | Have — editable colors and horizon | P:243; R M7a | — | Presentation uniforms | P1 |
| Match Photo / camera matching | P verified; web availability unverified | Missing | W; P; F | XL | Reference image, vanishing-point calibration, camera/UV mapping | P2 |
| Image export of current view/scenes | Free PNG (watermarked web); G/P expanded options | Missing — no end-user 2D image export; developer capture examples are not a workflow | F export menu; E 3D exporters; R M1 screenshot example | M | Offscreen render/readback, size/alpha, file dialog | P0 |
| PDF / vector 2D view export | P desktop platform-dependent; Free/Go PDF conflicting docs Q15 | Missing | F/E format actions | XL | Page units, vector/text composition or raster PDF, print settings | P1 |
| LayOut: scaled sheets, viewports, dimensions, references | P companion application ([inclusion](https://sketchup.trimble.com/en/products/layout)) | Missing — no paper-space/documentation subsystem | W; F; P | XL | Annotations, view exports, page layout, reference updates | P2 |
| Browser/mobile share-and-review model link | F/G/P connected sharing; limits differ | Missing — local file exchange only | F; W menus; R architecture | XL | Viewer packaging/hosting, access controls, mobile UX | P1 |

### Files and interoperability

The following directions follow [official format/tier information](https://help.sketchup.com/en/sketchup/using-sketchup-data-other-modeling-programs-or-tools), checked against [web file directions](https://help.sketchup.com/en/sketchup-web/file-management). Pro's [GLB import/export](https://help.sketchup.com/en/sketchup/working-gltf-files) is explicit. A supported extension is not a promise to preserve all semantics.

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Native save/open and read-only SKP import | F/G/P SKP save/open | Partial — native .cdraft save/recovery; independent bounded SKP reader, no SKP writer or full fidelity | F:313/:336; I SKP version/limits | XL | Independent format coverage, fidelity fixtures; writer separately designed | P1 |
| DWG / DXF import and export | G/P | Missing | F format dispatch; IO lib format exports | XL | Independent parser/writer or suitable licensed library, CAD units/layers/curves | P1 |
| IFC import/export | P | Missing | F; IO modules; K has no BIM classifications | XL | IFC schema, placements, object semantics, geometry conversion | Skip |
| FBX export / import | G/P export; native import not established | Missing — neither direction in Drafter; compare export only | F; E; official format page links Exporting FBX | XL | Independent encoding/library, materials/transforms | P2 |
| 3DS import/export | G/P | Missing | F; E | L | Binary format parser/writer, limits/materials | Skip |
| STL import/export | Free export; G/P web import; P desktop import/export | Partial — binary STL export only | E:339; F import filters | M | Triangle-soup importer, unit selection, welding/validation | P2 |
| OBJ export / import | G/P native export; native import not established | Have — Drafter imports and exports OBJ; UV fidelity separate | F:410/:425; IO import_obj; E:143 | — | Mesh IO/material sidecars | P1 |
| COLLADA DAE import/export | G/P | Have — hierarchy/instances/materials within documented limits | I; E; docs/DAE_EXPORT.md | — | Scene converter, assets | P1 |
| glTF / GLB | P current native GLB import/export | Partial — glTF/GLB import, self-contained .gltf export; no binary .glb export or PBR fidelity | F:410/:427; E:460; I; K Material | M | GLB container writer; PBR is separate XL work | P1 |
| KMZ import/export / geographic packaging | G/P; direction/platform details vary | Missing | F; E | L | ZIP/KML, DAE assets, geographic transform | P2 |
| PNG/JPEG reference images and floor-plan tracing | F/G/P image import | Partial — texture asset loading only; no placed image entity with handles/calibration | F:887; O Materials; K schema | L | Image planes, aspect/scale UI, selection, archive | P1 |
| Linked external model/reference reload | P component/LayOut references; generic CAD-style xref unsure Q16 | Missing — imports are snapshots with no live source link | I ImportedScene; F workers; K schema | XL | Source identity, replacement/merge policy, dependency tracking | P2 |
| Templates / reusable starting documents | F/G/P starter choices; custom templates P verified | Partial — a saved .cdraft can be reopened/copied; no template chooser/save-template workflow | F New/SaveAs; B units; P styles | M | Template metadata, fresh document identity, file UI | P1 |
| Autosave / crash recovery / unsaved changes protection | F/G/P product-specific mechanisms | Have — periodic snapshots, explicit recovery, outgoing backups | F; R M6; docs/FILE_FORMAT.md | — | Atomic native archive | P0 |
| Source units / origin / basis options and import report | P verified; web format-specific | Have — common conversion, options, warnings/progress/cancel; format limits apply | I; F import UI; docs/IMPORT_PIPELINE.md | — | ImportedScene validation | P1 |
| Authored UV seams / smooth normals / two-sided import fidelity | P capabilities; exact source round trip Q10/Q11 | Partial — hierarchy/materials retained, but native planar mapping/face normals replace source UV/normal sets | I; R M6/I1; E limits | XL | UV and shading schema; full importer/exporter integration | P0 |

### Productivity, platform and everyday workflows

[Shortcuts](https://help.sketchup.com/en/default-keyboard-shortcuts) and [purging unused data](https://help.sketchup.com/en/3d-warehouse/minimize-tag-usage) are established everyday workflows. Pro extensions are separate from native features; web does not run desktop extensions.

| Feature | In SketchUp (tier if known) | Drafter status (Have / Partial / Missing / Unsure) | Evidence | Complexity | Depends on | Priority for me |
| --- | --- | --- | --- | --- | --- | --- |
| Tool/file/history keyboard shortcuts | F/G/P | Partial — drawing, Select, camera, file/history keys; missing M/Q/S/E/T/B and usual P/F mapping | A:449; F input; W shortcut labels; T | S | Central action routing; preserve/customize current mappings | P0 |
| Custom shortcuts / preferences / toolbars | P customization; F/G web shortcuts/tool slots, detailed settings vary | Partial — inspector/view/unit controls only; no action rebinding or global preference system | W menus; A hardcoded keys; B document settings | L | Action IDs, settings persistence, conflict handling | P1 |
| Geometry/object context menus | F/G/P | Missing — measurements caption has a context menu, model entities do not | W:665; M/O input; W Edit | M | Shared actions, selection/target rules | P0 |
| Clipboard copy/cut/paste, Paste in Place and cross-file reuse | P verified; web clipboard scope unverified | Missing — modeling Copy is an in-document transform only | W Edit; A input; F; O/K | L | Selected-subgraph serialization, asset/ID remap | P1 |
| Undo / redo transactions and cancel previews | F/G/P | Have — geometry and organization, exact committed history; guides/presentation limitations separate | K histories; M; B | — | Transaction deltas | P0 |
| Searchable history / history panel | Native SketchUp searchable timeline not established Q17 | Unsure — Drafter only undo/redo depths; no panel | W Edit; K History | XL | Named history events, retained snapshots/index | P2 |
| Model Info / statistics / solid diagnostics | P verified; web Entity Info/settings differ | Partial — active-context edge/face counts; no document-wide statistics, solid inspector or repair UI | W:568; K validate/face_area; R | M | Definition/occurrence aggregation, manifold diagnostics | P1 |
| Purge unused components/materials/tags | P verified; web cleanup available, tier details unverified | Missing — individual deletes exist; no reachability-based purge | K OrganizationCommand; O; F; R native schema | M | Graph reachability, scene/material references, atomic deletion | P1 |
| Native performance controls versus proxies/LOD | P fast styles/visibility; general native proxy/LOD unsure Q18 | Partial — styles/tags/instancing/incremental caches exist; no user proxy/LOD controls; no speed-superiority claim | R benchmarks; P; docs/REAL_FILE_PROFILE.md | XL | Profiling, budgets, alternate mesh/pick representations | P1 |
| Extensions / user scripting API | P desktop Ruby extensions; unavailable in web | Missing — internal Rust APIs are not an end-user extension host | W menus; R crate boundaries; K typed commands | XL | Stable plugin API, sandbox/lifecycle, compatibility | P2 |
| Ready-made furniture discovery / Warehouse-like workflow | F/G/P model-library access; permissions/content limits vary | Missing — file import exists; no search/browser/download/placement catalog | F; O; W | XL | Licensed asset catalog/API, thumbnails, caching, placement | P1 |
| Reusable component placement without redrawing | F/G/P | Partial — duplication/import/shared instances; no definition palette with insertion-point preview | O duplicate/create; F import; K AddInstance | M | Definition browser, placement tool, insertion metadata | P0 |
| Browser/iPad editing and native desktop platform choice | Free browser; Go browser/iPad; Pro adds Windows/macOS desktop | Partial — native desktop Rust app; no browser/iPad build or touch workflow; Linux is an extra | Cargo workspace; R validation/platform limits | XL | Platform frontend/render/input adaptation | P2 |
| Smooth-surface selection across softened facets | P verified; exact selection rules Q19 | Partial — soft/smooth flags exist; raw picker operates on facets/edges without semantic surface expansion | selection.rs; K Edge; T select G3 | L | Surface adjacency, soft-edge traversal, selection UI | P1 |
| Tool discoverability: mode hints, hover targets and repeat feedback | F/G/P | Partial — tool names/status exist; generic hints and missing Push/Pull hover highlight/mode cursors | W status; M overlay; T push_pull B2 and utility | M | Action/tool states, cached hover picking | P0 |

## Top 10 gaps for this use case

1. **Image export from the current view and saved scenes (P0, M).** A clean image is the quickest way to show someone a room or furniture plan without installing a model viewer.\
   Scenes/styles/sections already exist, so user-facing capture connects substantial finished work to its intended audience.
2. **Persistent dimensions and text labels (P0, XL).** Room sizes, clearances and furniture names make a plan understandable without a live explanation.\
   Build one annotation foundation for labels and dimensions; start with linear measurements attached to stable entities.
3. **Texture size, rotation, placement and UV preservation (P0, XL).** Floorboard/tile scale and wood-grain direction strongly affect how an interior looks.\
   The existing texture loader cannot correct planar mapping or recover imported authored seams; shared UV work helps native and imported furniture.
4. **Eye-level camera placement and adjustable FOV (P0, L overall).** Orbit views can obscure what standing inside a room will feel like.\
   Add persisted FOV and a simple eye/target placement first; full Walk navigation can follow independently.
5. **Reliable selection and inference controls (P0, M/L).** Toggle/subtract selection, Shift hold and correct shape-plane locks reduce repeated picks and accidental placement.\
   Start with input semantics and shared constraints; cross-context inference and unified multi-object selection are separate larger increments.
6. **Move/scale several furniture objects together (P0, L).** Testing a dining set or grouped arrangement needs multiple container selections and one transform.\
   The current single-container state blocks this even though instance transforms already work; unify selection before adding more object tools.
7. **Persistent, perpendicular construction guides (P0, L).** Wall offsets, aligned cabinets and consistent clearances need repeatable construction aids.\
   Make guides history/file-owned before adding parallel offsets, guide points and protractor construction.
8. **Graphical Scale grips and explicit Flip Along actions (P0/P1, L).** Sizing furniture by one side or mirroring a layout should be clear from the selected handle.\
   Typed per-axis/negative scaling already exists; add pivot/grip control and reuse the same transform math.
9. **Component placement palette and replacement (P0, M/L).** Doors, chairs and cabinets should be reused and swapped while comparing arrangements.\
   Start with existing local definitions and placement; searchable online catalogs introduce licensing/network scope and can wait.
10. **Material sampling and proper soften/smooth controls (P0/P1, M plus shading work).** Reusing a finish and removing unwanted facet lines make furniture much easier to present.\
    Surface-aware sampling/painting and edge flags can reuse current infrastructure; interpolated normals need their own verified renderer increment.

DWG/DXF, IFC, full solid operations, terrain and LayOut are real gaps, but they are less central than communicating a room plan. No construction-documentation or BIM requirement was assumed. Imported-model fidelity and selection cost may outrank visual polish if actual furniture assets expose those limits.

## Suggested roadmap: eight bounded usage windows

These are future implementation suggestions, not work authorized by this audit. Each session has a concrete stop point and should make **one bounded increment**, including targeted validation and its completion checks. A session is not a promise to finish an XL subsystem. If investigation reaches its limit, commit the agreed completed slice and record follow-up scope. The eight windows intentionally leave full UV editing, Walk, advanced sections, robust general CSG and annotation breadth for later.

| Session | Deliverable that fits one bounded window | Dependency/order | Main risks and explicit stop point |
| --- | --- | --- | --- |
| 1 — quick actions | Central tool action map; add missing common activation keys without silently stealing current shortcuts; simple selected-material sampler. | Reuse existing tools/material picker. | P currently means Polygon and F means Fit. Define mapping/preset behavior first. Stop at key activation and one-face sampling; custom shortcut UI and paint scopes deferred. |
| 2 — shared selection/constraint rules | Add raw-selection toggle/subtract, Select All and double-click expansion; implement a bounded Shift-held direction state. | Shared actions from 1; existing raw selection/inference. | Focus/modifiers/Escape must not affect text fields or stale previews. No multi-container or cross-context inference claim in this slice. Shape-plane locks can move to a follow-up if they exceed the window. |
| 3 — presentation output | Export current view to PNG at one explicit size with existing styles, sections and shadows. | Existing render target/capture example and file UI; independent of annotations. | GPU readback, color/alpha, overlay inclusion and memory caps. Stop at one reliable current-view export; batch scenes, PDF and print layout deferred. |
| 4 — camera foundation | Persist adjustable FOV through camera, scenes and archive migration; add numeric eye height/target placement if the bounded slice permits. | Current camera and scenes; test exports from 3 reflect the chosen projection. | FOV must agree across rendering, rays/picking and fit behavior. Stop at FOV if eye/target conversion is larger than expected; Walk and two-point projection deferred. |
| 5 — annotation design and first vertical slice | Define anchored annotation IDs/schema/history; add one plain label type that saves, loads and appears in PNG output. | Units, entity identity, capture policy from 3. | Deleted/moved anchors, world versus screen text size and migration. This is the first XL-subsystem slice, not full dimensions. Stop after one tested label workflow. |
| 6 — first linear dimension | Reuse 5 for a two-anchor linear dimension with units, placement offset, editing and export. | Annotation foundation from 5; existing measurements settings. | Anchor updates and undo/redo across contexts. Limit to active-context linear dimensions; angular, chain, paper-scale and cross-instance anchors deferred. |
| 7 — selection graph foundation | Replace the single selected-container option with a set; support selecting two sibling objects and one batched translation with exact undo. | Session 2 selection semantics; existing instance transforms. | Shared-definition ownership, mixed contexts and visibility. Stop at sibling object translation; arbitrary mixed loose geometry/containers, box selection and multi-scale need later slices. |
| 8 — guide foundation | Persist existing line guides with stable IDs and command history; expose deletion and native save/load. | Reuse identity/migration lessons from 5; existing guide rendering. | Guides currently live outside the document; snapping/cache/context ownership must stay consistent. Stop at line-guide lifecycle; perpendicular offsets, point guides and protractor follow later. |

**Next backlog after these windows:** UV schema and one-face size/rotation editor (design first, then render/import/export slices); graphical Scale grips and Flip actions; local component palette/replacement; soften/smooth UI and normals; perpendicular Offset and Follow Me corner/path repairs. These reuse shared selection/identity/coordinate systems above. Full general solid tools, dynamic components, BIM, online catalogs and paper-layout software remain separate projects. Do not squeeze them into the eight windows or present the roadmap as full parity.

## Unsure: small reference trials

Record product (desktop/web), version, subscription, OS and the result. Each numbered item can be answered by doing the named operation; do not infer the result from Drafter's implementation.

1. **Measurements phase semantics:** finish a center Arc and retype a radius; separately scale a 2×3×4 box to `8m`, and copy then enter `3x` or `/3`. Which result changes, and how many copies remain? These resolve the independent trials in `docs/MEASUREMENTS_BOX.md`; compound entry and re-entry are already implemented here.
2. **Shape-plane locks:** choose Circle, press Right before placing its center, then hold Shift after acquiring a plane. Which plane/direction remains locked in your product?
3. **Cross-context inference:** exit a group, draw a Line and hover that group's midpoint; repeat while editing a second group. Which targets snap?
4. **Drawing hints:** start a rectangle near a square proportion, then draw a Line tangent to an existing arc. Which hints and constraints appear?
5. **Selection/copy modifiers:** with two boxes, test Shift-click, Ctrl-click (Option on Mac), Ctrl+Shift-click, and Rotate with copy modifier. Record membership/copy behavior; do not assume desktop keys equal browser keys.
6. **Push/Pull modes:** extrude a face, double-click another, then repeat while toggling the start-new modifier. Which original/intermediate faces remain?
7. **Autofold/overlap:** copy a loose square halfway across itself; separately move a connected box corner off its face plane. Are edges split and are new fold triangles created?
8. **Follow Me corners:** sweep a rectangular profile around a closed rectangular path using preselection; repeat by tracing manually. Inspect the final seam and inner corners. This also distinguishes current Drafter sweep support from equivalent results.
9. **Align versus Flip:** select a cabinet and inspect its native menu/context menu for alignment. Is there a built-in Align command, or only Flip, inference-based Move, or an extension? Do not count an extension as native Align.
10. **Paid-web texture editing:** right-click a textured face and try setting physical tile size, rotating the texture and pin positioning. Which controls exist? Separately reimport a UV-mapped furniture model and inspect its seams.
11. **Face sides:** paint the front and back different colors, enable Monochrome/Back Edges, and view from each side. Which controls exist in the tier you use, and what is displayed?
12. **Modern appearance:** open material settings and try enabling roughness/metalness and an environment/AO view. Which are available in your desktop/web version and tier?
13. **Saved selections:** select two objects and look for a native command that saves that membership under a name. Is it actually a selection set, a scene/visibility workaround or an installed extension? Native support remains unestablished.
14. **Camera in web:** use Zoom and type a FOV angle, then look for Two-Point Perspective. Which controls are available in your tier? Desktop support is established.
15. **PDF direction/platform:** in a fresh Free web file, try Export/Print to PDF and PNG. Is PDF a real export, browser printing, gated or absent? Report watermark/resolution. Official feature/file pages disagree; Mac desktop PDF import/export is a separate case.
16. **References:** insert a component from a local model file, change that file outside the current model, and try Reload. Does the model keep a source association? This establishes component reload, not arbitrary live CAD xrefs.
17. **History:** inspect Undo and the available panels. Is there a native named/searchable history timeline, or only undo/redo? A third-party extension is not evidence of native support.
18. **Proxies/LOD:** inspect native model/performance settings for a switch that swaps an object for lower-detail geometry. Is it native, a renderer proxy or an extension? No general native proxy claim is made here.
19. **Soft surfaces:** click a softened cylinder once, then show hidden geometry and click a facet. Does selection expand across soft seams? Repeat double-click to record its exact scope.

The saved-selection and searchable-history rows are **Unsure**, not established SketchUp advantages. Dedicated Align, native FBX/OBJ import, general xrefs, and native proxies/LOD are similarly not asserted as built-in gaps without verification. Benchmarked Drafter timings and imported-file limits do not establish any comparative SketchUp speed claim.

## Things Drafter already has extra, or offers differently

Only concrete, scoped differences are claimed:

- **Native Linux desktop application.** The Rust/eframe app runs locally on Linux; SketchUp Pro desktop targets Windows/macOS. Browser SketchUp still offers a Linux-accessible alternative, so this is a native-platform extra, not exclusive Linux usability. Evidence: `Cargo.toml`, `crates/cindral-app/src/main.rs`, R local Linux GPU validation; [SketchUp system requirements](https://help.sketchup.com/en/sketchup/system-requirements).
- **OBJ import alongside export.** Drafter exposes both directions directly; the current official SketchUp interoperability list documents native OBJ export. This is an extra relative to that verified native workflow, not proof no extension can import OBJ. Evidence: F:410, `crates/cindral-io/src/import_obj.rs`; [official OBJ export](https://help.sketchup.com/en/sketchup/exporting-obj-files).
- **Display-free native-to-DAE CLI.** Drafter's documented `--export-dae` path includes selection/context, units, hidden-content options and cancellation without opening the modeling UI. No claim is made that SketchUp has no automation equivalents. Evidence: `docs/DAE_EXPORT.md`, `crates/cindral-io/src/dae_cli.rs`.

No blanket advantage in robustness, visual quality, format fidelity or performance is established by this audit.

## Audit record and limitations

Read AGENTS.md first, skimmed architecture, all requested parity Markdown audits and the recent agent log, then inspected bounded source ranges and searched the tool registry, menus, panels, schema and format dispatch. Old measurements/cleanup/Eraser findings were reconciled with the current implementation. One read-only Antigravity task (`gemini-3.8-flash-high`, high effort, 60-second print deadline/80-second external limit) returned no findings before timeout; no delegated claims were accepted and the audit was completed locally. OpenCode remained disabled.

Only this document is added/committed, following the explicit task restriction instead of updating AGENT_LOG.md. The pre-existing `.gitignore` modification is preserved and excluded. No application code, features, fixtures or dependencies were changed. No app restart, remote push, SketchUp execution or new benchmark was performed. Source inspection and existing tests establish implementation evidence, not native UI certification or vendor parity. Ignored parity regressions remain evidence of unresolved behavior, not passing functionality.

Validation: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `git diff --check` pass. One `cargo test --workspace --all-features --quiet` run completed all 52 targets: **450 passed, 0 failed, 45 ignored**. No new tests or performance measurements were added for this documentation-only audit.

**Inventory: 116 rows — 25 Have, 42 Partial, 47 Missing, 2 Unsure.** Counts are per scoped row, not per atomic subfeature; some rows group related variants. The priority list ranks remaining work, not every item included in a grouped row.
