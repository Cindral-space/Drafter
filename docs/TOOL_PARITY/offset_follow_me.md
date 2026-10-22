# Offset and Follow Me parity audit

2026-10-22. Audit only; classic desktop tools, authored loose geometry unless stated. No SketchUp execution, SDK, assets or implementation changes. UI artwork and wording may differ. `matches` covers only the scope stated in a row; `unsure` is not a pass or a defect. Follow Me procedural components are outside this classic-tool audit.

Expected workflows were paraphrased and checked against [Offset](https://help.sketchup.com/en/sketchup/offsetting-line-existing-geometry) and [Follow Me](https://help.sketchup.com/en/sketchup/extruding-follow-me). No documentation text copied. Other expectations use established modeling behavior; uncertain details are explicitly questions below. Evidence paths are repository-relative.

## Parity table

| Behavior | Expected (SketchUp) | Cindral status (matches / differs / missing / unsure) | Evidence (test name, file:line, or read the code) |
| --- | --- | --- | --- |
| O1 Offset activation | F activates Offset. | differs | `crates/cindral-app/src/lib.rs:425`: F fits view. |
| O2 Tool palette activation | Both tools can be explicitly activated. | matches | `crates/cindral-app/src/workspace_ui.rs:80`; read the code. |
| O3 Cursor and status | Identify active tool and next action; artwork may differ. | differs | `workspace_ui.rs:430,676`: crosshair and tool label, but generic drag hint omits face/path workflows. |
| O4 Offset distance display | Show current offset distance. | matches | `crates/cindral-app/src/modify_ui.rs:416`: numeric meters overlay; field named Dimensions, not dynamically relabeled. |
| O5 Offset explicit metric/decimal imperial | Accept lengths such as 2m, 150cm, 42in and 3.5ft after anchoring. | matches | `crates/cindral-tools/src/modify.rs:851`; `units.rs:11`; CPU unit test added in step 2. |
| O6 Offset compound imperial | Accept 3'6\" as a length. | missing | `units.rs:11`: suffix-only parser; ignored regression in step 2. |
| O7 Bare numbers and timing | Template units determine bare-number scale; pre-click entry timing needs a trial (Q1). | unsure | `units.rs:28`: bare values always meters; `modify_ui.rs:128`: viewport text only while active. |
| O8 Offset after completion | Allow changing completed distance by typing again. | missing | `modify_ui.rs:60`; `ModifySession::finish` clears active target. |
| O9 Offset clicks | Click face/edge, move, click to finish. | matches | `modify_ui.rs:242,341`: anchor on press, finish on displaced release; CPU UI regression in step 2. |
| O10 Offset drag variant | Whether press-drag-release also completes needs a trial (Q2). | unsure | `modify_ui.rs:341`: Cindral accepts drag release. |
| O11 Offset double click | Repeat previous offset on another face. | missing | `modify_ui.rs:239`: double-click handling exists only for selection triple click; no repeat distance. |
| O12 Offset empty initial click | With no selection, empty click cannot edit geometry. | matches | `modify.rs:91,562`: NoEditableFace; empty-target test in step 2. |
| O13 Offset pointer distance | Measure perpendicular separation of parallel edges. | differs | `modify.rs:768`: Euclidean anchor-to-pointer length, centroid dot product for sign; ignored diagonal regression. |
| O14 Selected open coplanar chain | Offset a selected connected chain of at least two coplanar edges. | missing | `modify.rs:562`: requires an incident/selected face; `modification.rs:811` accepts only face outer ring. |
| O15 Offset partial face-edge selection | Offset the selected connected subset, rather than the entire face perimeter. | differs | `modify.rs:91,562`: edge resolves entire face; `modification.rs:825` traverses full outer ring. |
| O16 Offset holes | Which rings offset, and which side counts as inward, need a trial (Q3). | unsure | `modification.rs:825`: only outer ring offset; no hole traversal. |
| O17 Offset concavity/overlaps | Default removes overlaps; modifier can retain them. | missing | `modification.rs:839`: rejects some collapsed/outside loops; no overlap-removal mode or toggle. |
| O18 Offset modifiers | Alt on Windows / Command on macOS changes overlap mode. Linux mapping is a product decision; Ctrl/Shift effects need Q4. | missing | `modify_ui.rs:108,220`: no Offset modifier state. |
| O19 Offset ordinary planar result | Add parallel inset edges and bounded planar regions, preserving outer boundary. | matches | `modification.rs:855`: AddPolyline; exact inset geometry/validation test in step 2. |
| O20 Offset soft/curved surface | Facet selection and smooth-surface behavior need Q5. | unsure | `modify.rs:562`: one planar FaceId; `modification.rs:811`; no analytic surface offset. |
| F1 Follow Me preselected path resolution | Use a continuous selected edge path for a profile sweep. | matches | `modify.rs:259,597,794`; `test_followme_with_connected_ordered_edges_and_anchor_fallback` (see actual name in validation below); scope is command path resolution only. |
| F2 Automatic completion | Clicking profile completes preselected-path sweep. | differs | `modify_ui.rs:305,341`: Follow Me is not an immediate action; displaced release or Enter needed. |
| F3 Manual path following | Trace existing edges interactively. | missing | `modify.rs:794`: unselected fallback always two points; `modify_ui.rs:313` constrains pointer to face normal. |
| F4 Manual completion convention | Exact drag/release versus final-click behavior needs Q6. | unsure | `modify_ui.rs:341`: commits on >3px displacement, not path end. |
| F5 Measurements and double click | Follow Me numeric label/editability, before/after click entry, repeat double click need Q7. | unsure | `modify.rs:919`: accepts a length, but ignores length for preselected path; `modify_ui.rs:416`: displays pointer distance, not path length. |
| F6 Follow Me shortcut | Default shortcut presence needs Q8; explicit palette activation available. | unsure | `lib.rs:408`: no Follow Me key binding. |
| F7 Follow Me straight result | Normal-aligned straight sweep creates valid closed sides and caps. | matches | `modification.rs:718`; `sweep_straight_and_bent_profiles_and_paint_are_undoable`; new straight sweep test. |
| F8 Follow Me bends and closed loops | Corner cross section, seam closure and twist need Q9; topology validation alone is insufficient. | unsure | `modification.rs:747`: rotates profile at stations; no bisector intersection or closed-loop seam branch. |
| F9 Oblique profile | Projection/rejection of nonperpendicular starting profile needs Q10. | unsure | `modification.rs:739`: rejects nonnormal start. |
| F10 Holes, normals, intersections | Hole walls, cap orientation and interaction with adjacent solids need Q11. | unsure | `modification.rs:732,776,800`: hole stations/caps and surface rebuilding; no equivalence certification. |
| F11 Path feedback | Show which existing edges the manual sweep follows; color can differ. | missing | `modify_ui.rs:355`: selection outline only; no traced-path state. |
| F12 Invalid selected path | Disconnected/branching selection response needs Q12. | unsure | `modify.rs:259,601`: silently substitutes empty path and then straight fallback. Product should explain invalid selections regardless of parity answer. |
| I1 Snaps/locks for both | Endpoint, midpoint, on-edge, on-face; axis arrow locks and Shift hold applicability need Q13. | unsure | `modify_ui.rs:180,194`: inference and arrow locks; no Shift hold; Follow Me overrides inferred point with normal-axis projection. |
| I2 Inference feedback | Which snap highlights and axis colors appear for these tools needs Q13. | unsure | `workspace_ui.rs:392`: modify mode uses modify overlay; `modify_ui.rs:355` omits inference markers. Red/green/blue axis convention exists globally, no per-tool colored guide. |
| S1 No preselection | Offset face and Follow Me profile can be chosen directly. | matches | `modify.rs:91,562,597`; direct-hit fixture tests. |
| S2 Edit context | Edits stay in current group/component context, shared definitions propagate. | matches | `crates/cindral-app/src/lib.rs` History working-model usage; `crates/cindral-core/src/organization.rs`; read the code. Scope: existing History context contract, not tool-specific imported/group selection certification. |
| S3 Closed groups/components | Tools operate on editable internal geometry, not closed container geometry. | matches | `modify.rs:91`: requires local face; `modify_ui.rs:175`: local model picking. Enter context first; read the code. |
| S4 Selected face, empty click | Whether selected face acts despite click in space needs Q14. | unsure | `modify.rs:147`: falls back to first selected face, for both tools. |
| C1 Cancel preview | Escape restores geometry without a committed edit. | matches | `modify_ui.rs:17,93`; exact History preview/cancel test and synthetic-input test in step 2. |
| C2 Undo/redo | One completed operation is one undo entry, redo restores it. | matches | `modify_ui.rs:37`; exact History test in step 2. |
| C3 Undo tool reset | Undo/redo cancels active operation before history navigation. | matches | `modify_ui.rs:103`: cancel_modify before history_step; synthetic-input test in step 2. |
| C4 Invalid input/geometry | Report error, preserve valid committed model. | matches | `modify_ui.rs:26,60,697` (error rendered by `workspace_ui.rs:697`); `History::preview` rollback; invalid input tests. |
| C5 Live preview | Show prospective edited geometry before commit. | matches | `modify_ui.rs:26,325`: provisional History geometry rendered via refresh_mesh; CPU preview/cancel tests. |
| P1 20k-face performance | Project requirement: live preview <16ms and commit <100ms on Ryzen 5 2400G, release. SketchUp timing unknown. | unsure | CPU probe and limits below; no full picking/inference/mesh/GPU timing from this audit. Q15. |
| P2 Imported .skp geometry | Equivalent editable planar topology should behave consistently; import-specific parity unverified. | unsure | No importer changes or imported fixture certification; Q16. |

## Questions for the user

1. **Q1 (O7):** On a centimeter template, activate Offset, type `50` before clicking anything, then click a face. Is the value used? Repeat by typing `50` after the first click; what distance results?
2. **Q2 (O10):** Press Offset on a square edge, hold the button, move inward and release. Does this finish the offset or keep it active?
3. **Q3 (O16):** Draw a square with a square hole, Offset the annular face inward by `20cm`. Do both boundaries move, and in which directions?
4. **Q4 (O18):** During an ordinary Offset preview, tap Ctrl, then Shift separately. Does either change behavior? Record platform/version; Linux equivalents are not assumed.
5. **Q5 (O20):** On a cylinder side with softened vertical edges, activate Offset and click its side. Does it target one facet, the whole surface, or refuse?
6. **Q6 (F4):** With no path selected, Follow Me a profile around a two-edge corner using press-drag-release. Does release finish, or must you click the endpoint?
7. **Q7 (F5):** Select a straight path, activate Follow Me, type `2m` before clicking the profile, then repeat after clicking it. What does the measurements label show, and does either entry affect the sweep? Immediately double-click a second profile: does anything repeat?
8. **Q8 (F6):** In a fresh default desktop shortcut setup, find Follow Me in shortcut preferences. Is any key assigned?
9. **Q9 (F8):** Sweep a rectangular profile round a closed square path. At the first corner and closing seam, are cross sections mitered without a twist or gap? Show Hidden Geometry to inspect.
10. **Q10 (F9):** Tilt a square profile 30 degrees away from perpendicular to a straight path, then use preselected-path Follow Me. Is it rejected, projected, or rotated?
11. **Q11 (F10):** Sweep a square annulus along a straight normal path into an existing box. Does the center remain hollow, do faces intersect/merge, and which cap sides face outward with back-face coloring enabled?
12. **Q12 (F12):** Select two disconnected edges, activate Follow Me and click a profile. Does it refuse, choose one edge, or do something else? Note feedback.
13. **Q13 (I1/I2):** For each tool, start a preview and hover an endpoint, midpoint, edge, face and axis alignment in turn; press Right/Left/Up/Down then hold Shift. Which snaps/locks affect the preview, and which highlights or colored guides appear?
14. **Q14 (S4):** Select a face, activate each tool separately and click empty space away from it. Does an operation begin on that face?
15. **Q15 (P1):** On your ~20k-face model on the Ryzen 5 2400G, perform one Offset and one Follow Me sweep in release Cindral and SketchUp. Is either visibly delayed? Instrumented presented-frame/commit timing is still needed to establish the numeric budgets.
16. **Q16 (P2):** Import the same permitted .skp with a planar face, hole and softened curved path into Cindral; run one inset and one path sweep beside the original in SketchUp. Which result or rejection differs?

## Side-by-side manual script

Use classic tools, meter units, a fresh model, top view for Offset. In Cindral use the Modeling palette to activate named tools; in SketchUp use Tools menu. Toolbar placement/artwork is not graded. Use Ctrl+Z/Ctrl+Y (macOS Command equivalents) for history. Set up two disjoint 10m squares, a 1m square XY profile with a connected normal path `(0,0,0) → (0,0,3) → (3,0,3)`, and an annulus in a separate area. Save this starting model. Coordinate placement may use existing drawing tools; setup is outside the tool audit.

1. Hover viewport, press `F`. Expected: SketchUp activates Offset; Cindral fits view (O1). Choose Offset explicitly afterward.
2. Clear selection, click empty space, move and press Escape. Expected: no geometry/history change; Cindral explains missing editable face.
3. Click midpoint of the left edge of square A, release; move inward and along the edge, then click. Expected: parallel boundaries, distance measured perpendicular to source edge. Cindral's diagonal travel overestimates it (O13). Undo.
4. Click square A edge, move inward, type `2m`, Enter. Expected: 6m inner square, 2m band, valid coplanar faces. Undo once; redo once restores exact result.
5. Undo; repeat step 4 with `150cm`. Expected: 7m inner square, 1.5m band. Escape during another preview: restored geometry from before that preview.
6. Immediately type `1m`, Enter after completing step 5. Expected: completed inset changes to 1m in SketchUp; Cindral has no completed-operation measurement target (O8).
7. Complete a 1m inset on A, double-click square B with Offset. Expected: same inset repeated in SketchUp; Cindral has no repeat operation (O11). Undo back to setup.
8. Click A edge, hold button, move inward and release. Expected Cindral: drag commits. Record SketchUp behavior for Q2. Undo; start again, type `3'6"`, Enter: expected valid imperial inset; Cindral rejects format.
9. Select only two connected perpendicular edges of square A, choose Offset, click selected edge, move and click. Expected: selected chain offset; Cindral offsets entire face (O15). Undo.
10. Click annular face with Offset, move inward, type `20cm`, Enter. Record both ring changes for Q3; Cindral changes outer loop only. Undo.
11. In perspective, select both edges of the prepared bent path (use Select then Shift-click each in Cindral). Activate Follow Me, click/release profile once, move pointer away. Expected SketchUp: entire selected path completes; Cindral requires additional movement/release or Enter (F2). Escape, then restore setup if needed.
12. With nothing selected, activate Follow Me, click profile then move over first path edge and around corner, click endpoint. Expected: follows both existing segments; Cindral only sweeps a straight fallback (F3). Escape must cancel the pending edit.
13. Select only first normal path edge, activate Follow Me, click profile, then drag/release in Cindral if required. Expected: valid straight extrusion. Undo once restores profile/path, redo restores result; start another preview and press Ctrl+Z: active tool resets before undo.
14. Restore setup, put profile/path in a group, double-click group to edit, repeat step 13. Expected: result stays inside context. Exit context; neither tool may directly alter internal faces through the closed group.
15. Restore setup; start an Offset preview and then a manual Follow Me preview separately. Hover snaps, press arrow keys and Shift as in Q13; then Escape. Expected Cindral: axis query exists, no Shift-held inference or snap overlay; record SketchUp result rather than treating unknown lock behavior as a pass/fail.

## CPU tests and performance limits

Completion validation, test names, raw release measurements and reproduction are recorded below after execution. GPU-free tests establish specific state/geometry contracts, not visual parity. No real SketchUp timing or imported .skp certification is claimed. Existing bent-sweep kernel test checks valid closed topology, not correct corner dimensions.
