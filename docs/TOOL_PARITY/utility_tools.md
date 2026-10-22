# Utility tools parity audit: Eraser, Tape Measure, Paint Bucket

Audit only, 2026-10-22. Desktop SketchUp baseline; shortcuts/modifiers use Windows conventions (Cindral Linux equivalents must be wired explicitly). No SketchUp execution, SDK, binaries, copied assets, production changes or importer work. A match describes only the stated scope; UI styling is deliberately independent. Unknowns are questions, not inferred bugs. Evidence paths below are repository-relative.

Expected behavior is paraphrased from established workflows and checked against [Erasing and Undoing](https://help.sketchup.com/en/sketchup/erasing-and-undoing), [Measuring Distance](https://help.sketchup.com/en/measuring-distance), [Using Guides](https://help.sketchup.com/en/using-guides), [Applying Materials](https://help.sketchup.com/en/sketchup/applying-materials), [softening](https://help.sketchup.com/en/sketchup/softening-smoothing-and-hiding-geometry), and the [desktop reference card](https://download.sketchup.com/QRC2025/HC-QRC2025-en-SU-Windows.pdf). Recent help describes three Tape modes; older versions may differ. No documentation text reproduced.

Totals: **52 rows: 15 matches / 8 differs / 16 missing / 13 unsure**. Abbreviated sources: `modify.rs` and `units.rs` are under `crates/cindral-tools/src/`; `modify_ui.rs`, `workspace_ui.rs`, `organization_ui.rs` and `lib.rs` are under `crates/cindral-app/src/`. Existing CPU evidence includes `chord_splits_edges_and_erasure_heals_midpoints`, `midpoint_and_on_edge_snapping_respect_priority_and_perspective`, `locked_axis_snaps_compatible_collinear_and_crossing_edges_and_partially_clipped_segments`, `splitting_and_healing_painted_faces_preserves_material_and_history`, and `construction_guides_stay_local_to_their_definition`.

| Behavior | Expected (SketchUp) | Cindral status (matches / differs / missing / unsure) | Evidence (test name, file:line, or read the code) |
| --- | --- | --- | --- |
| All: activation E/T/B | Activate Eraser/Tape/Paint respectively | missing | `crates/cindral-app/src/lib.rs:412` shortcut list lacks all three; toolbar works |
| All: cursor communicates mode | Cursor indicates active tool and modifier state; artwork need not match | differs | `crates/cindral-app/src/workspace_ui.rs:435`: shared crosshair and tool label; no modifier cursor states |
| Eraser/Paint: useful hint and measurements | Action/modifier instructions; no numeric geometry entry required | differs | `workspace_ui.rs:666`: generic Dimensions/2m field; generic drag/type hint inaccurate for these single-press tools |
| Tape: field and hint | Distance/length display plus guide/measure instructions | differs | `workspace_ui.rs:674`, `modify_ui.rs:416`: generic Dimensions plus overlay in meters; no guide-mode hint |
| Eraser: edge click | Remove hit edge and unsupported incident face | matches | `modify.rs:807`; CPU tests in `crates/cindral-tools/tests/utility_parity.rs` |
| Eraser: click face interior | Do not erase the face via Eraser | differs | `modify.rs:202`: face hit resolves every boundary edge |
| Eraser: multi-edge drag | Accumulate highlighted edges; erase on release as one operation | missing | `modify_ui.rs:294`: commits initial target on press and immediately finishes; no stroke accumulator |
| Eraser: empty click | With no target, leave geometry unchanged | matches | `modify.rs:605`: NotReady; app reports error |
| Eraser: Ctrl/Shift modes | Soften/smooth or hide edges rather than delete | missing | `modify_ui.rs:112`, `modify.rs:807`: no modifier routing or style command |
| Eraser: Ctrl+Shift recovery | Restore softened edges to hard visible edges | missing | Same modifier/command path; no recovery action |
| Eraser: preselection plus unrelated hit | Exact selected-versus-hit scope requires trial Q1 | unsure | `modify.rs:188`: unions selection with hit (also vertex-incident edges) |
| Eraser: coplanar divider healing | Erasing a shared coplanar divider merges regions | matches | Kernel EraseEdges; `utility_parity` chord test |
| Eraser: guides | Clicking a guide removes it | missing | `Guide` is session-local, `Hit` has no guide variant; `modify.rs:187` |
| Eraser: holes/curved surfaces/normals | Exact surviving faces and orientation on nontrivial erase require Q2 | unsure | Rebuilds topology; modifier smoothing absent; no equivalent curved/holed SketchUp trial |
| Eraser: double-click | Exact extra behavior requires Q3 | unsure | App only handles presses; no Eraser double-click branch |
| Tape: measure two points | First click anchors, pointer reports distance, second click ends | differs | `utility_parity_tape_two_click_distance_and_escape` fails: stationary first release inferred 0.000000542294m and finished with a tiny guide; `modify_ui.rs:330` |
| Tape: measure-only mode | Measure without creating construction geometry | missing | `modify.rs:974` always calls add_guide; Ctrl has no mode toggle |
| Tape: edge hover length | Before anchoring, hovering an edge displays its length | missing | `modify_ui.rs:416` only current/last measurement; no hovered-edge length |
| Tape: edge-offset guide | Guide is parallel to source edge at entered perpendicular offset | differs | `modify.rs:513`: guide is through original anchor in pointer direction, independent of source edge |
| Tape: guide points | Distinct point construction aid | missing | `modify.rs:46`: only infinite Guide lines |
| Tape: guide numeric entry after anchor | Accept explicit 2m, 150cm, decimal imperial values | matches | `modify.rs:944`, `units.rs:9`; CPU numeric tests (numeric distance only, placement differs) |
| Tape: imperial compound entry | Accept 3'6" as 1.0668 m | missing | `units.rs:9`: single numeric value/suffix parser; ignored regression |
| Tape: bare values | Use model display units | differs | `units.rs:29`: always meters; no document-unit conversion |
| Tape: typing before first click | Exact result requires Q4 | unsure | Session rejects inactive measure; viewport captures text only while active |
| Tape: typing after measured endpoints | Offer proportional model resizing after measuring | missing | `modify_ui.rs:60`: inactive NotReady; no resize workflow |
| Tape: drag vs two clicks | Exact drag completion semantics require Q5 | unsure | `modify_ui.rs:330` finishes on any release after nonzero measurement |
| Tape: first click empty | Free-space anchor/guide behavior requires Q6 | unsure | Uses work-plane intersection even with no hit |
| Tape: double-click edge | Exact special action requires Q7 | unsure | No Tape-specific double-click handling |
| Tape: geometry snaps and axes | Snap to endpoints, midpoints, edges, faces and axes; pointer measurement follows inferred point | matches | `modify_ui.rs:177` Query/infer -> update; shared inference tests; visual overlay excluded |
| Tape: arrow locks | Right/Left/Up constrain X/Y/Z | matches | `modify_ui.rs:114` -> Query.axis_lock; UI arrow-lock test |
| Tape: Shift inference hold | Hold current inference while moving | missing | Shift only additive selection; no stored held inference |
| Tape: inference feedback | Identify snap and axis direction visually, including axis colors | missing | `lib.rs:326` suppresses paint_snap_overlay in modify mode; no measuring segment in modify_overlay |
| Tape: guide undo/redo | Construction aids participate in undo/redo | missing | `modify.rs:974`: Vec push, no history command; ignored UI regression |
| Tape: cancel before completion | Escape removes pending interaction without adding guide or changing model/history | matches | `modify_ui.rs:93`; CPU/UI cancel tests |
| Tape: guides in edit context | Keep construction aids in their definition context | matches | `organization_ui.rs:1403` construction_guides_stay_local_to_their_definition |
| Paint: single face color | Apply chosen color without changing topology or winding | matches | `modify.rs:816`; color/topology/history CPU test |
| Paint: material and object click | Assign chosen material to face or group/component | matches | `organization_ui.rs:1031`: PaintInstance/PaintFaces; existing organization material tests |
| Paint: selection scope | Apply selected-face batch consistently when palette material is active | differs | `organization_ui.rs:1045` intercepts material click with only hit face; fallback `modify.rs:218` unions selection |
| Paint: Alt sampler | Pick an existing material for reuse | missing | Organization/modify input paths contain no Alt sample action |
| Paint: Ctrl/Shift/Ctrl+Shift scopes | Adjacent matching / all matching / connected replacement scopes | missing | `modify.rs:816`: fixed target faces; no material-scope traversal |
| Paint: empty click | No target and no selection changes nothing | matches | `modify.rs:613`: NotReady; errors surfaced by app |
| Paint: edge material | Paint an edge as an entity | missing | `modify.rs:229` resolves adjacent faces; no PaintEdge command |
| Paint: front/back material | Independent side assignment and default restoration exact behavior require Q8 | unsure | One face material/color representation; no front/back click mapping |
| Paint: holes and soft surfaces | Hole remains empty; exact paint scope across softened seams requires Q9 | unsure | Face assignment does not edit rings; no soft-surface traversal in resolver |
| Paint: double-click/drag | Exact behavior beyond one click requires Q10 | unsure | Immediate commit per press; no dedicated double-click/brush path |
| All: groups/context boundaries | Edit only appropriate active context; shared definitions affect occurrences | matches | `organization_ui.rs:1031`, `1064`; existing context/material tests; Eraser object-shell action excluded |
| Eraser: object shell | Whether Eraser can erase unopened group/component requires Q11 | unsure | App rejects inactive geometry; no instance deletion from Eraser |
| Eraser/Paint: cancel before commit | Cancel prepared operation without model/history changes | matches | CPU cancellation test; Eraser UI commits on press so pending stroke cancellation is missing above |
| Eraser/Paint: history and reset | One completed click, one undo; redo restores result; reset pending tool on undo | matches | CPU exact history tests; `modify_ui.rs:102`; synthetic-egui reset test |
| All: impossible action feedback | Invalid numeric input/uneditable targets are reported and preserve model | matches | `modify_ui.rs:286`, `:79`; CPU invalid-input and no-target tests |
| All: imported .skp geometry | Tool behavior on private imported fixtures requires Q12 | unsure | No .skp implementation or certification in this audit; native topology tests do not establish importer parity |
| All: ~20k faces on Ryzen 5 2400G | Project budget: live preview <16 ms, operation commit <100 ms, release | unsure | CPU subset probe only; full picking/inference/render/presentation and connected-model timings unverified; Q13 |

## Questions for the user

Every unsure row maps to a trial; observations should include SketchUp version/platform. These are not known defects.

1. Q1: Select one loose edge, activate Eraser, then click a different edge. Which edges disappear? Repeat clicking a shared endpoint: are all incident edges erased?
2. Q2: Draw an inner loop on a face; erase one inner edge. Which regions survive and do front/back orientations change? Repeat erasing one seam of a faceted curved surface with no modifier.
3. Q3: Double-click a loose edge with Eraser. Does anything beyond that edge get erased?
4. Q4: Activate Tape Measure, type `150cm`, Enter before clicking. Does it set anything or get ignored?
5. Q5: In Tape Measure's Measure mode, press at an endpoint, drag to another endpoint, release. Is measuring finished, or does it still await a click?
6. Q6: In Tape guide mode, click empty space, move, click again. Does it produce a line, point, or nothing?
7. Q7: Double-click a known edge using Tape Measure. Does it measure, create a guide, or do something else?
8. Q8: Paint the front red and back blue, then apply Default on only the back. Which sides change?
9. Q9: Paint an annular face beside an unpainted inner face, then paint one facet of a softened cylinder. Which faces receive paint in each case?
10. Q10: Double-click one painted face with Paint Bucket, then separately drag across two faces while holding the mouse. Which faces change and how many undos restore them?
11. Q11: Click an unopened group with Eraser on a visible edge. Does it erase the object, ignore it, or enter it?
12. Q12: Open your own .skp with a holed face, softened seam and nested component. Erase one seam, measure an edge, and paint one inner face in edit context; do results match the same native-model operations?
13. Q13: In a roughly 20k-face model on your Ryzen 5 2400G, measure a long diagonal, erase one edge and paint one face. Record release interaction/commit latency for Cindral and observed responsiveness in SketchUp; do previews stay below 16 ms and commits below 100 ms? Timing instrumentation is needed; visual impressions alone cannot certify these bounds.

## Side-by-side manual script

Use desktop default shortcuts in SketchUp, Cindral toolbar buttons Eraser/Measure/Paint when shortcuts fail. Start a fresh meter model in top view, no selection, and choose a plain color. Save a copy before destructive trials. Each numbered step includes its own setup; undo its changes before continuing. Q trials intentionally have an observational expected result.

1. Draw a 10m by 10m square with a diagonal and two separate loose lines outside it. Hover viewport and press `E`, `T`, `B` separately. Expected: matching tool activation; Cindral currently needs toolbar buttons.
2. Activate Eraser, click middle of a loose line. Expected: just that edge disappears. Press Ctrl+Z then Ctrl+Y: restore then delete exactly once; Ctrl+Z restores fixture.
3. Eraser: click center of the square away from the diagonal/edges. Expected: no erase in SketchUp; Cindral currently erases face boundaries. Undo if changed.
4. Eraser: press on first loose line, drag across second, release. Expected: both highlighted during stroke and erased on release, one undo restores both. Cindral commits first target on press.
5. Eraser: tap Ctrl, click diagonal; undo. Hold Shift, click diagonal; undo. Expected: soften/smooth then hide while preserving surface, respectively. Cindral lacks modes; stop/undo if it deletes.
6. Select Eraser, click diagonal with no modifiers. Expected: one square face replaces the two triangular regions; undo restores divider.
7. Activate Tape Measure; Ctrl-cycle to Measure mode in SketchUp. Click first square corner, move to diagonal opposite corner, click. Expected: approximately 14.142136m; no guide in Measure mode. Cindral may finish immediately with a tiny guide on the first release; otherwise it measures and adds an infinite guide.
8. Tape: click midpoint of bottom horizontal edge; move straight up, type `2m`, Enter. Expected in guide mode: horizontal guide 2m above bottom. Cindral creates vertical line through original anchor. Repeat from same edge with `150cm`, then `3'6"`; expected offsets 1.5m and 1.0668m. Cindral rejects compound imperial.
9. Tape: click first corner, press Right arrow, move diagonally; then Escape. Expected: X constrained live distance; cancellation adds no guide or history entry. Repeat using Left/Y and Up/Z on a suitable 3D view.
10. Tape: click first corner, move toward opposite corner, hold Shift and move away. Expected: held inference; Cindral does not hold it. Escape.
11. Tape: complete a guide, Ctrl+Z then Ctrl+Y. Expected: guide disappears and returns. Cindral guides currently bypass undo. Click guide with Eraser: expected deletion; unsupported in Cindral.
12. Tape in Measure mode: measure a square edge, type `5m`, Enter. Expected: resize confirmation in SketchUp; decline it to preserve fixture. Cindral has no resize action.
13. Paint: choose a plain red color, click a square face interior. Expected: color changes without changing edge count or shape; Ctrl+Z/Ctrl+Y reverses/restores once. Start Tape then press Ctrl+Z: pending tool resets before undo.
14. Paint: choose blue; Alt-click red face, release Alt, click a different face. Expected: sampling red then applying it. Ctrl-click a matching-material adjacent face and Shift-click a matching-material distant face in separate restored trials: expected corresponding fill/replace scopes; Cindral modifiers are absent.
15. Enter a component shared by two occurrences; paint one face, then undo/redo. Expected: shared definition occurrences reflect same change, unrelated root geometry stays intact. Run Q8/Q9 separately for face sides/holes/soft surfaces; results remain unclassified until observed.

## Automated validation and performance limits

CPU tests: `cargo test -p cindral-tools --test utility_parity`; synthetic app tests use filter `utility_parity`. Ignored tests preserve confirmed missing/different contracts and must fail when explicitly run; they do not implement fixes. Existing organization tests provide material/context coverage. No tests assert the Q1–Q12 unknown SketchUp outcomes.

Release probe and final measured results are recorded after execution below. All CPU probes exclude GPU, mesh rebuilding, UI presentation, picking and inference. A passing CPU subset cannot establish the full project latency budget. No performance claim is made for imported geometry or whole-model painting/erasing.

Final validation: seven new ordinary tests pass (four tool/kernel, three synthetic-egui); nine known desired-behavior regressions fail on explicit ignored runs. The additional ignored test is the opt-in release probe. Keyboard/cancel/history app tests establish a session directly where necessary: they do not disguise the separate, ignored first-click routing failure. Completion checks: formatting, strict workspace/all-target/all-feature Clippy and workspace/all-feature tests pass (390 passed, 0 failed, 58 ignored). Only tests and documentation changed.

Release CPU probe reproduction:

```sh
cargo test --release -p cindral-app --features kernel-benchmark utility_parity_release_cpu_probe -- --ignored --nocapture --test-threads=1
```

Ryzen 5 2400G; 20,000 authored faces in 10,000 disconnected triangulated squares, ten synthetic pointer/command updates per tool, one edge/face target, one guide completion for Tape. Fixture creation, begin/target resolution and validation setup are outside timing. Command generation plus kernel provisional application are inside update timing for Eraser/Paint; Tape only updates distance. Commit is the prepared history commit for Eraser/Paint, session guide completion for Tape. This is not the app's immediate-click path or a connected/full-selection workload.

| Tool | Isolated max CPU update (ms) | Prepared commit/finish (ms) |
| --- | ---: | ---: |
| Eraser | 0.126181 | 0.001100 |
| Paint Bucket | 0.051161 | 0.000700 |
| Tape Measure | 0.000900 | 0.000910 |

All measured CPU subsets pass their bounds. Earlier sample, concurrent with compilation/workspace checks, update/commit ms: Eraser 0.198492/0.001330; Paint 0.100241/0.001290; Tape 0.002270/0.001080. Isolated output: [utility-release.txt](utility-release.txt). No full viewport or SketchUp latency measurement; performance row remains unsure.
