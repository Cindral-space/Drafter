# Draw tools parity audit — Line, Rectangle, Circle, Arc, Polygon

2026-10-22; audit only. No production changes or SketchUp execution. Arc means the **center-based Arc**, not 2 Point Arc, Pie or 3 Point Arc. Default desktop shortcuts/preferences are version dependent. Appearance, icons and branding are intentionally different.

Expected behavior is paraphrased from established modeling behavior and checked against official [drawing basics](https://help.sketchup.com/en/sketchup/introducing-drawing-basics-and-concepts), [basic shapes](https://help.sketchup.com/en/sketchup/drawing-basic-shapes), [arcs](https://help.sketchup.com/en/sketchup/drawing-arcs), and [drawing preferences](https://help.sketchup.com/en/sketchup/drawing). No source text or assets copied. Uncertain subcases have trial questions below. “matches” is limited to the named scope, not certification of the complete UI.

Evidence paths below are repository-relative. S = crates/cindral-tools/src/session.rs; U = crates/cindral-tools/src/units.rs; A = crates/cindral-app/src/lib.rs; W = crates/cindral-app/src/workspace_ui.rs; I = crates/cindral-inference/src/engine.rs. Function names identify code read where a test is not named.

## Behavior table

| Behavior | Expected (SketchUp) | Cindral status (matches / differs / missing / unsure) | Evidence |
| --- | --- | --- | --- |
| Line activation | L | matches | A:412 |
| Rectangle activation | R | matches | A:413 |
| Circle activation | C | matches | A:414 |
| Arc activation shortcut | Which arc variant A activates needs confirmation (Q1). | unsure | A:415 activates center-based Arc |
| Polygon activation shortcut | Default shortcut for user's desktop version needs confirmation (Q2). | unsure | A:416 assigns P |
| Tool cursor and status instructions | Visible tool identity and placement guidance. | matches | W:404 paint_tool_cursor; W:680 tool-specific prompts; visual art intentionally distinct |
| Dynamic measurement feedback, all five | Length, dimensions, radius or angle appropriate to phase. | missing | W:653–674 always “Dimensions”; only input buffer/hint, no calculated readout |
| Explicit lengths after anchor: Line/Circle/Polygon | Metric and explicit imperial units accepted. | matches | U:parse_length; S:measure_line/measure_circle/measure_polygon; test_typed_unit_measurements_and_invalid_errors_and_cancel |
| Rectangle dimension pair | Two comma-separated dimensions. | matches | S:measure_rectangle; same existing typed-unit test |
| Compound imperial lengths | Accept 3'6". | missing | U:parse_length only parses one numeric suffix; ignored regression |
| Bare length units | Use model's chosen length unit. | differs | U:parse_length always meters; no document-unit argument |
| Typing before first placement: Line/Rectangle/Arc | Exact behavior needs trial (Q3). | unsure | A:537 captures viewport text only after anchor; S:measure rejects idle state; focused Dimensions field can accept text earlier |
| Circle/Polygon pre-placement side count | Set sides before center click. | missing | S:measure requires anchor; W:365 has separate Segments control (3–256), not measurement workflow |
| Rectangle partial dimension | Allow one dimension to remain unchanged. | missing | S:measure_rectangle requires two nonempty parseable lengths |
| Re-enter dimensions after completion: Line/Rectangle/Circle/Polygon | Revise latest primitive before next operation. | differs | S clears closed-shape anchors; Line advances start so measure appends; A:apply_measurement only executes new command |
| Arc click order and result | Center, start, end; open segmented curve. | matches | S:click_arc; test_each_tool_click_order; test_closed_face_creation |
| Arc typed angle after second click | Angle commits sweep. | matches | S:measure_arc; test_arc_radius_sweep_and_segment_settings |
| Arc radius typing after center | Whether typed radius sets second point needs trial (Q4). | unsure | S:measure_arc NotReady with only one anchor |
| Arc negative sweep and third-click direction | Exact clockwise and long-sweep semantics need trial (Q5). | unsure | S:measure_arc accepts signed degrees; click_arc uses positive sweep up to 360 |
| Arc segments after completion / Ctrl +/- | Adjust tessellation without redrawing. | missing | S clears anchors; W Segments only changes session setting; A has no Ctrl +/- handling |
| Line click-move-click and continuation | Endpoint starts next segment under continuation preference. | matches | S:click_line; test_chained_line_behavior |
| Rectangle click-move-click | Opposite corners create planar face. | matches | S:click_rectangle; test_each_tool_click_order |
| Circle/Polygon click-move-click | Center then radius point/vertex. | matches | S:click_circle/click_polygon; test_each_tool_click_order |
| Line/Rectangle click-drag | Drag release completes primitive. | matches | A:565–589; read the code (new routing regression below) |
| Circle/Arc/Polygon drag preferences | Which tools support drag in user's preference mode needs trial (Q6). | unsure | A:566 only enables drawing drag for Line/Rectangle |
| Double-click during drawing | Per-tool chain ending/repeat behavior needs trial (Q7). | unsure | A drawing branch has no double-click-specific logic; organization handler may intercept |
| First click on empty ground | Start geometry without a selection. | matches | I:default_engine_and_plane_fallback_work; S first-click paths |
| Empty sky / ray parallel to work plane | Exact fallback and feedback need trial (Q8). | unsure | I fallback can return None; A:596 ignores click when snap absent |
| Endpoint/midpoint/on-edge/on-face snaps | Infer existing geometry. | matches | I:endpoints_and_grade_three_plus_intersections_are_distinguished; midpoint_and_on_edge_snapping_respect_priority_and_perspective; on_face_respects_holes_and_picks_nearest_face_hit |
| Line arrow locks | Right=X, Left=Y, Up=Z. | matches | A:460–480; I:hard_axis_lock_works_on_z_from_ground_plane_and_rejects_incompatible_geometry |
| Shape arrow locks | Lock drawing plane rather than collapse radius/corner to an axis. | differs | A assigns the same query.axis_lock to every tool; S projects onto unchanged plane; no arrow-induced plane update |
| Shift inference hold | Retain inferred direction/plane while pointer moves. | missing | A:viewport_input has no draw Shift inference state; Shift only used for navigation/redo |
| Ctrl/Alt additional modes | Per-tool center/corner/circumscription modes vary by version; trial required (Q9). | unsure | A drawing branch has no tool modifier modes |
| Preview geometry | Moving pointer previews proposed path. | matches | S:preview; test_preview_command_consistency; A:752 |
| Axis color feedback | Convey axis of constrained preview. | differs | A:752–762 uses same turquoise preview for all directions; snap label names axis but guide is uniform blue |
| Rectangle square/golden-ratio hints | Provide proportion inference. | missing | I SnapKind and S:preview_rectangle contain no proportion candidates |
| Existing loose selection vs none | Drawing adds geometry independently of selection. | matches | A:click_tool_at/execute do not use selection to form draw command |
| Editing group/component context | Edits belong to active definition; shared instances reflect edits. | matches | A query transforms local coordinates; organization_ui.rs:148 enter_context; History active model execution; read the code (context UI trial still recommended) |
| Snapping across closed contexts | Cross-context inference eligibility needs trial (Q10). | unsure | A:engine.infer only reads history.model(), excluding inactive definitions |
| Closing a planar line boundary | Creates a face. | matches | test_closed_face_creation |
| Rectangle/Circle/Polygon face creation | Closed boundary creates face. | matches | test_closed_face_creation; test_arbitrary_tilted_planes |
| Curve identity: Circle/Arc/Polygon | Segments retain collective curve identity. | missing | S emits ordinary AddPolyline only; core Edge has no curve owner/radius metadata; selection cannot select curve as unit |
| Line intersection splitting / coplanar region healing | Shared geometry splits into valid regions. | matches | core transactional AddEdge/AddPolyline; core tests chord_splits_edges_and_erasure_heals_midpoints; read architecture kernel contract (specific holes case below remains unverified) |
| Faces with holes / crossing hole boundaries | Exact face fill/reconstruction behavior needs trial (Q11). | unsure | kernel supports holes and rebuilds bounded regions; no side-by-side hole drawing certification |
| Face normals/front-back | Exact orientation rules for newly closed loops need trial (Q12). | unsure | S plane_basis_for; kernel canonical normal is not outward-solid orientation |
| Curved/soft surfaces | Exact hidden-edge intersection and surface drawing result needs trial (Q13). | unsure | curves are planar polylines; no projection/wrapping across soft surface in session |
| Imported .skp drawing | Same user workflow on supported imported geometry; fixture-dependent outcome (Q14). | unsure | no importer changes or private fixture use in this audit; generic active-model path only |
| Escape before completion | Discard preview without changing model. | matches | S:cancel; A:449; new GPU-free cancellation regression |
| One undo entry and redo per completed primitive | Restore exact geometry; Line segment is one operation. | matches | A:commit; new all-tool command/history regression |
| State after successful undo | Reset unfinished drawing. | matches | A:history_step calls tool.cancel on changed history; new app regression |
| Undo with pending input / empty history | Exact pending-text and no-op undo semantics need trial (Q15). | unsure | A:history_step does not clear measurement/axis_lock; cancels tool only on Ok(true) |
| Invalid lengths / degenerate geometry | Reject without partial geometry and show feedback. | matches | S validates paths/points; A:error/commit rollback; W:702 error; new invalid-measurement regression |
| Live preview <16 ms; commit <100 ms, ~20k faces | Project budget on Ryzen 5 2400G, not a claimed SketchUp timing. | unsure | No complete presented-frame measurement for these five draw tools; CPU probe below does not establish viewport budget (Q16) |

## Questions for the user

1. **Q1:** In your SketchUp desktop version, press A in a new file, then click three points: does it ask for center/start/end, or endpoint/endpoint/bulge?
2. **Q2:** In a new SketchUp file with default shortcuts, press P: which tool activates? Can Polygon be activated without a custom shortcut?
3. **Q3:** Select Line, type 2m Enter before clicking anywhere, then click the origin: what happens? Repeat with Rectangle and center-based Arc.
4. **Q4:** Choose center-based Arc, click the origin, move along red, type 2m Enter: does that set the start/radius, or reject/ignore it?
5. **Q5:** Choose center-based Arc, set center and a red-axis start, then type -90 Enter: which way does it sweep? Repeat by clicking an end direction clockwise from the start; is the result short or long?
6. **Q6:** In Drawing preferences choose Auto Detect, then press-drag-release a Circle; does it complete? Repeat Polygon and each placement phase of center-based Arc.
7. **Q7:** Draw one Line segment then double-click the next endpoint: does chaining stop? Repeat a completion double-click for Rectangle, Circle, Arc and Polygon and report whether it starts a new shape or repeats dimensions.
8. **Q8:** In front view, aim above the horizon with Line and click empty sky: where does the first point land and what feedback appears? Repeat Circle.
9. **Q9:** With each of these five tools active, hold Ctrl, Shift and Alt separately before the first click: which alternative placement modes does the status bar offer? Include your platform/version.
10. **Q10:** Make a group, exit it, activate Line and hover its edge midpoint: can you snap there while drawing outside the group? Repeat while editing another group.
11. **Q11:** On a flat face with a square hole, draw a rectangle crossing two hole boundaries: which regions fill, and which boundaries remain? Repeat a circle crossing the hole.
12. **Q12:** In monochrome view draw a ground rectangle in each drag direction, then a closed clockwise/counterclockwise Line loop: which side faces upward?
13. **Q13:** On a softened cylinder, draw a Line across two hidden facets with hidden geometry off: does it create projected surface edges, one spatial edge, or no edge? Repeat Rectangle.
14. **Q14:** Open your usual .skp model, enter a component and draw a 2m line snapped to an imported endpoint: does it split an existing edge and update all shared instances? Report any import-specific difference seen in Cindral.
15. **Q15:** Start a new file, click Line start, type 2 without Enter, then Ctrl+Z: do the anchor and typed text clear even when history is empty? Repeat after committing one segment.
16. **Q16:** In Cindral on your Ryzen 5 2400G, load a representative ~20k-face model and record preview/commit frame timing for all five tools in release mode: are worst frames below 16/100 ms? This question validates the project budget; no claim about SketchUp speed.

## Side-by-side manual script (15 steps)

Use a fresh meter-based file, top view, red right/green up. “Click” means press and release left mouse. Choose center-based Arc explicitly from SketchUp's menu. Use the tool palette in Cindral when shortcut assignments differ. Undo isolated trials to avoid overlap. Expected column is the shared contract or a named known gap; questions are observations, not pass criteria.

1. Press L; click origin; move along red; type 2m Enter; press Esc. Expected: one 2m edge, no dangling preview.
2. Press L; click an empty ground point; move along green; type 150cm Enter; Esc. Expected: 1.5m edge.
3. Press L; click a new point; move along red; type 3'6" Enter. Expected: 1.0668m edge in SketchUp; Cindral rejects compound units (gap). Esc.
4. Press L; click a new point; click a point along red; type 2m Enter without another click. Expected: revise latest edge in SketchUp; Cindral appends another edge (gap). Esc.
5. Press R; click an empty ground point; move diagonally; type 2m,150cm Enter. Expected: four edges, one 3m² face. Ctrl+Z then Ctrl+Y: remove and restore exactly one rectangle.
6. Press R; press left mouse on empty ground, drag diagonally, release. Expected: one rectangle. Press R; click first corner, move, Esc: no additional geometry.
7. Press C; before center placement type 12 Enter. Expected: 12 sides in SketchUp; viewport typing is ignored by Cindral (gap). In Cindral set Segments to 12 to continue.
8. With Circle active, click a new center; move along red; type 2m Enter. Expected: radius 2m closed face with configured segments. Immediately type 150cm Enter: SketchUp revises radius; Cindral has no completed-circle remeasurement (gap).
9. Choose Polygon by menu/palette; set sides to 6 (SketchUp type 6 Enter; Cindral Segments 6); click center; move red; type 2m Enter. Expected: six-sided face with center-to-vertex radius 2m.
10. Choose center-based Arc; click empty center; click red-axis start; move toward green; type 90 Enter. Expected: open quarter arc, no sector face. Ctrl+Z/Ctrl+Y removes/restores all its edges as one operation.
11. Press L; hover a rectangle endpoint, then edge midpoint, then face interior. Click midpoint; move red; press Right; move off red; click; Esc. Expected: named snaps and red-axis-constrained line.
12. Press L; click a free start; hover along red; hold Shift; move far toward green. Expected: held direction in SketchUp; Cindral does not hold inference (gap). Esc. With Circle, press Right before center placement: compare plane orientation (known lock gap).
13. Press R; click first corner on an existing vertical planar face; move diagonally on that face; click opposite corner. Expected: rectangle remains on face plane and subdivides it. Repeat with Circle or Polygon. Existing loose selection should not change draw placement.
14. Select one primitive, create a group using each app's grouping UI, enter the group by double-click, then draw a short line inside it. Exit context and undo/redo that edit. Expected: geometry belongs to group definition; no changes to unrelated root geometry. Shared components should update all instances.
15. For each tool start an unfinished operation (Arc: center and start), move pointer, then Esc. Expected: no geometry/history change. Complete one primitive, start another and Ctrl+Z: previous primitive disappears and unfinished preview clears; Ctrl+Y restores it. Record Q15 separately if typed text/locks remain.

## Automated evidence and limits

Existing GPU-free coverage: crates/cindral-tools/tests/tool_session.rs (nine tests) and cindral-inference engine tests named above. New tests: crates/cindral-tools/tests/draw_parity.rs and crates/cindral-app/src/draw_parity_tests.rs. Ignored desired-behavior regressions deliberately retain known gaps; do not interpret ignored as passing. Exact final validation/results are recorded in docs/AGENT_LOG.md.

CPU performance probe is opt-in, release-only, under the existing kernel-benchmark feature. It uses 20,000 authored triangles in disconnected coplanar tiles, times 30 ToolSession pointer/preview updates and one History execute per tool. It excludes inference, picking, egui, mesh construction, upload, GPU and display latency. It must never turn the full-frame performance row into “matches.” No .skp fixture certification, native dependency, Python, SDK, or SketchUp binary is involved.


### Completed validation and measurements

53 scoped rows: **25 matches / 4 differs / 8 missing / 16 unsure**. Top workflow gaps: completion remeasurement, shape-plane arrow locks, and Shift inference hold. Cursor art and branding differences are not defects.

Eight new ordinary tests pass (five public tool/kernel tests, three synthetic-egui routing tests). Six ignored behavioral tests were explicitly run and all failed as expected: compound imperial input, Line remeasurement, pre-center Circle sides, partial Rectangle dimensions, completed Circle remeasurement, and Circle plane lock. Arc radius-before-start remains a question, not a failing contract. The synthetic input harness was corrected to initialize hover and supply egui's platform command modifier; no production fix was made.

Completion checks: targeted tools/app checks; formatting; strict workspace/all-target/all-feature Clippy; one full workspace/all-feature test run: **368 passed, 0 failed, 26 ignored**. The new release probe is the seventh added ignored test and passes when explicitly invoked. Existing ignored tests were not enabled in the normal workspace run.

Measured on AMD Ryzen 5 2400G with Radeon Vega Graphics, release profile, CPU only. Fixture construction excluded. Isolated repeat after all checks/builds completed:

| Tool | Max pointer + polyline preview, 30 samples (ms) | One kernel commit (ms) |
| --- | ---: | ---: |
| Line | 0.000610 | 0.174 |
| Rectangle | 0.001720 | 0.107 |
| Circle | 0.013000 | 0.760 |
| Arc | 0.003640 | 0.383 |
| Polygon | 0.001890 | 0.155 |

First sample, possibly overlapping other checks: preview/commit ms = Line 0.000710/0.072, Rectangle 0.001050/0.099, Circle 0.014450/0.970, Arc 0.003850/0.318, Polygon 0.001630/0.144. Both runs pass only the isolated CPU bounds; **the complete live-preview and presented commit budgets remain unsure**. This fixture tests an isolated new drawing outside existing tiles; it does not measure editing through densely connected geometry.

Reproduce: `cargo test --release -p cindral-app --features kernel-benchmark --lib draw_parity_tests::draw_20k_cpu_budget -- --ignored --exact --nocapture`. Raw isolated output: [draw-release.txt](draw-release.txt).
