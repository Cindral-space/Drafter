# Select → Transform tools parity audit

Scope: Move/Copy, Rotate and Scale, desktop workflow, 2026-10-22. Select itself is audited in `select.md`. Audit only: no production fixes. SketchUp was not run. Visual styling, icon artwork and branding are intentionally independent. Windows Ctrl is the reference modifier; macOS usually uses Option for copy/center. Cindral Linux has no transform modifier equivalent implemented.

Expected behavior is paraphrased from established desktop behavior and checked against official [Move](https://help.sketchup.com/en/sketchup/moving-entities-around), [Rotate](https://help.sketchup.com/en/sketchup/flipping-mirroring-rotating-and-arrays), [Scale](https://help.sketchup.com/en/sketchup/scaling-your-model-or-parts-your-model) and [shortcuts](https://help.sketchup.com/en/default-keyboard-shortcuts) references. These sources do not settle the questions below. No documentation text or assets copied. “Matches” applies only to the narrow behavior named, not whole-tool certification. Performance limits are the user's targets, not measured SketchUp timings.

Evidence paths below are relative to the repository. `MU` = `crates/cindral-app/src/modify_ui.rs`; `OU` = `crates/cindral-app/src/organization_ui.rs`; `MS` = `crates/cindral-tools/src/modify.rs`; `WU` = `crates/cindral-app/src/workspace_ui.rs`. Test names without a path belong to `crates/cindral-tools/tests/transform_parity.rs` unless explicitly qualified. Source line references were recorded at audit time.

| Behavior | Expected (SketchUp) | Cindral status | Evidence |
| --- | --- | --- | --- |
| A1 Activation shortcuts | M Move, Q Rotate, S Scale. | missing | `crates/cindral-app/src/lib.rs:411`: only drawing keys and Space routed. |
| A2 Tool indication/cursor | Active transform is identifiable; artwork need not match. | matches | WU:424–440 crosshair/tool label, grabbing for instance drag; read the code. |
| A3 Status guidance | Explain current pick/transform stage. | differs | WU:679 generic drag hint for all three; Rotate stages and Scale handles absent. |
| A4 Measurement meaning | Move distance, Rotate angle, Scale factor/direction. | differs | WU:655–674 always labels field Dimensions; MU:416 overlay reports m/deg/x, no directional Scale label. |
| A5 Move/Copy explicit metric input | `2m` and `150cm` override units. | matches | MS:859; `crates/cindral-tools/src/units.rs:10`; typed geometry tests. |
| A6 Architectural units | Move accepts `3'6"`. | missing | `units.rs:20` suffix parser only; ignored compound-unit regression. |
| A7 Unitless template values | Bare Move distance uses document units. | differs | `units.rs:31` always meters; no document unit passed to ModifySession. |
| A8 Move coordinates | Absolute bracketed and relative angle-bracket coordinates accepted. | missing | MS:861 calls scalar `parse_length`; no coordinate parser. |
| A9 Rotate typed degrees | Signed numeric angles rotate precisely. | matches | MS:884; existing `test_rotate_convention_affine_around_anchor_and_measure`; new geometry tests. |
| A10 Scale typed factor | Numeric 2 doubles selected geometry uniformly when appropriate. | matches | MS:905; existing `test_scale_affine_around_anchor_and_typed_scale`; new geometry tests. |
| A11 Scale dimensional entry | Grip direction supports target size and multi-axis entries. | missing | MS:345 only scalar/%/x; no grips/directional scale. |
| A12 Before-first-click typing | Exact response to typing before any anchor/grip is uncertain. | unsure | Q1; MU:128 only accepts viewport text while active; focused field can submit but session returns NotReady. |
| A13 Immediately-after Move typing | Last displacement can be revised after placement. | missing | MU:60/finish_modify, MS:963 clears session; ignored remeasurement regression. |
| I1 Move two-click placement | Pick reference, move pointer, click destination. | matches | MU:247–349 keeps initial press and commits on later release away from start; headless app test. |
| I2 Move drag-release | Whether drag-release is interchangeable with two clicks in user's version/settings needs a trial. | unsure | Q2; existing `live_move_drag_commits_once_and_escape_restores_geometry_and_mesh` proves Cindral drag behavior only. |
| I3 Rotate pivot/reference/end | User places pivot, baseline, then final angle. | differs | MU:262/291 substitutes selected-vertex centroid and first-hit reference; no three-stage pivot workflow. |
| I4 Rotate custom axis | Dragging an axis defines a fold/rotation axis. | missing | MU:258/291 derives axis from hit face/work plane, no axis-drag stage. |
| I5 Scale grips/anchor | Box grips choose affected directions; opposite grip anchors by default. | missing | MU:262 centroid; MS:748 isotropic radius scaling only; no grip hit testing. |
| I6 Double click | Tool-specific repeat or completion behavior needs confirmation. | unsure | Q3; MU has triple-click only in Select; transform path has no special double-click branch. |
| I7 Empty first click, no selection | Exact idle/error behavior needs confirmation. | unsure | Q4; MS:582 fails NotReady with no targets; MU:249 explicit plane error if no point. |
| N1 Move endpoint/midpoint/edge/face | Inferences can position the reference point. | matches | MU:173–230 feeds shared engine result to update; `cindral-inference/src/lib.rs` SnapKind and inference tests; read the code. |
| N2 Move axis colors | Red X, green Y, blue Z communicate axes. | matches | `cindral-inference/src/lib.rs:9`; `crates/cindral-app/src/lib.rs` inference overlay axis colors; read the code. |
| N3 Move arrow locks | Right X, left Y, up Z constrain displacement. | matches | MU:113–124/194; shared hard-lock query; app regression. |
| N4 Shift inference hold | Hold a previously acquired movement inference. | missing | MU:235 reads Shift for selection only; no inference-hold state. |
| N5 Rotate plane locking | Arrow/Shift locks choose the rotation plane. | differs | MU:113/258 sets query axis lock but rotation axis still comes from face/work plane; no Shift plane hold. |
| N6 Scale snapping | Exact endpoint/edge inference interaction with a grip is uncertain. | unsure | Q5; MU sends snapped point into radius scale; no grips. |
| M1 Move/Rotate copy modifier | Ctrl toggles copy during either operation. | missing | MS:693/719 hardcode copy=false; separate Copy translation kind at 707; no modifier handling. |
| M2 Scale Ctrl and Shift | Toggle center anchor and proportional scaling. | missing | MU has no such toggles; Scale is always uniform about centroid. |
| M3 Move Alt/Rotate Alt | Move offers autofold/grip cycling; Rotate offers plane release/grip cycling where applicable. | missing | MU and OU transform routing have no Alt branch; no functional grips. |
| M4 Copy arrays | Linear/radial copy can be repeated/divided by typed array counts. | missing | MS:859/884 scalar parsing only; Copy supports translation only. |
| S1 Raw selection vs hover | With a preselection, reference picking must not add unrelated geometry to the transform. | differs | MS:154 unions hit vertices into selection; regression with two disconnected squares. |
| S2 No selection, face hit | Move can target geometry under pointer. | matches | MS:154/580 resolves hit; typed geometry/no-selection tests. |
| S3 Groups/components | Transform a selected instance without flattening its definition. | matches | OU:669–759 uses instance transforms/duplicate; existing `container_drag_cancel_typed_commit_and_shared_context_dimming`. Narrow scope: one instance. |
| S4 Editing context | Geometry edits apply to active definition; shared instances update. | matches | `organization_ui::tests::scripted_three_instances_edit_in_place_exit_undo_reverts_all_and_redo_restores_all`; history world/context transform. |
| S5 Mixed/multiple containers | Transform several selected objects together. | missing | OU:1073 uses one `organization.selected` InstancePath; raw Selection does not hold instance paths. |
| G1 Isolated raw geometry | Rigid transforms/positive scale produce valid geometry; Copy keeps original. | matches | `cindral-core/src/modification.rs:184`; new real-history geometry tests. |
| G2 Connected stretching/autofold | Exact face splitting/healing after nonplanar connected edits requires a fixture trial. | unsure | Q6; `modification.rs:260` updates touched face planes and validates; no general autofold stage in tool. |
| G3 Faces with holes | Rigid transform should carry inner loops and preserve hole area. | matches | `modification.rs:227–245` maps hole vertices; new holed-face transform contract. |
| G4 Normals/reflections | Winding/front-back behavior under reflections needs comparison. | unsure | Q7; `modification.rs:276` recomputes plane; loose Scale rejects negative factor, instance Scale permits it. |
| G5 Curved/soft surfaces | Smooth/soft selection, deformation and seam behavior need comparison. | unsure | Q8; kernel copies edge flags but no analytic curved deformation workflow established. |
| G6 Imported .skp geometry | Same authored operation should work on supported imported entities; exact fixture behavior unverified. | unsure | Q9; no importer changes/test certification; historical Move-only profiling is not parity proof. |
| U1 Escape raw preview | Cancel restores model without history entry. | matches | MU:92; `History::cancel_preview`; new four-kind cancellation contract. |
| U2 Commit/undo/redo | One operation is one undo entry, redo restores result. | matches | MU:36; real History preview/commit contract for all four kinds. |
| U3 Undo resets raw tool | Undo cancels provisional operation and clears active state before history traversal. | matches | MU:102–110; synthetic app regression. Axis lock persistence assessed separately in Q10. |
| U4 Lock state after cancel/undo | Whether explicit axis locks persist between operations needs confirmation. | unsure | Q10; MU:92/108 does not clear `axis_lock`. |
| F1 Live geometry preview | Pointer movement previews prospective geometry. | matches | MU:27/315 and OU:746/931; existing live Move test plus new preview contracts. |
| F2 Highlight/measurement feedback | Selected geometry and current values remain visible. | matches | MU:357–435 edges/face rings/vertices and measurement overlay; OU:1110 instance bounds; read the code. |
| F3 Cannot act/invalid value | Invalid operations show feedback and preserve document. | matches | MU:29/79/249/307; error field WU:698; invalid measurement regression. |
| P1 Full preview <16 ms, commit <100 ms | User's release budget on Ryzen 5 2400G, approximately 20k faces. SketchUp timing unknown. | unsure | Q11; CPU probe below excludes viewport work. `docs/REAL_FILE_PROFILE.md` already reports loose Move above 16 ms on a different fixture; no new full GPU run. |

## Questions for the user

Every unsure row is covered here. Please record SketchUp version, desktop platform and interaction settings with each trial.

1. **A12:** Select a rectangle, activate Move, type `2m` and Enter before clicking. Does anything move, is the input retained, or is it ignored? Repeat with Rotate `90` and Scale `2` before placing a pivot/grip.
2. **I2:** Select a rectangle, activate Move, press on a corner, drag two meters and release without a second click. Is the move committed? Repeat with click-release, move, click.
3. **I6:** Complete one Move, then double-click a different face using Move. Is the last move repeated? Repeat once with Rotate and once with Scale, undoing between trials.
4. **I7:** Clear selection, activate Move and click empty background. Does it stay idle, show a hint, or start an operation? Repeat Rotate and Scale.
5. **N6:** Scale a rectangle from a corner grip toward a separate edge endpoint. Does the grip snap exactly to the endpoint, and does an arrow key constrain that scale?
6. **G2:** Move one upper corner of a box off the planes of its adjacent faces. Are diagonals created automatically? Undo, then repeat with Alt held; report which faces split and whether coplanar edges heal on undo.
7. **G4:** Scale a grouped box by `-1` using a corner grip, then repeat on loose geometry. Are front/back faces reversed or preserved, and does the anchor stay fixed?
8. **G5:** Turn hidden geometry off on a cylinder, move one visible curved patch upward, and undo. Which facets move, and do soft/smooth seams remain unchanged? Repeat Rotate and Scale on the same patch.
9. **G6:** In the same small .skp model opened/imported into both programs, select one group, Move `2m`, Rotate `90`, Scale `2`, then undo three times. Do geometry, holes, materials and instance sharing survive identically? Name the fixture; do not send proprietary assets unless authorized.
10. **U4:** Lock Move to red using Right, start a move, press Escape, start another move and move away from red. Is the lock retained? Repeat after Ctrl+Z during a provisional move.
11. **P1:** On your Ryzen 5 2400G with a roughly 20k-face model, run the three transforms with recording/timing enabled. Are full preview frames below 16 ms and completed operations below 100 ms? Report geometry selection size and model structure; visual smoothness alone cannot certify either bound.

## Manual side-by-side script

Start a fresh meter-unit model in each app, Top view, orthographic projection. Prepare two separate 2m × 2m rectangles at (0,0)–(2,2) and (6,0)–(8,2). Select all of the first rectangle using a left-to-right selection box. Use Cindral's labeled toolbar/menu buttons when a shortcut is missing. Ctrl means Windows/Linux Ctrl; use macOS Option for copy. Undo each independent trial to restore the fixture. Expected outcomes are behavioral, not pixel comparisons.

1. With pointer over viewport, press **M**, **Q**, **S** in turn. Expected: Move, Rotate, Scale activate; Cindral currently fails these shortcuts. Activate Move using its button to continue.
2. Click-release (0,0); move toward +X; type **2m**, **Enter**. Expected: selected rectangle translates exactly 2m along inferred red, with one undo entry. Press **Ctrl+Z**, **Ctrl+Y**: original and translated states return. Undo again.
3. Repeat step 2 with **150cm**. Expected: 1.5m displacement. Undo. Repeat with **3'6"**: expected 1.0668m; Cindral rejects the compound format. Press **Escape** if still active.
4. Click-release (0,0), move to (3,0), then click-release there. Expected: one 3m translation; Cindral supports this two-click path. Immediately type **2m**, **Enter**: expected final displacement becomes 2m, not 5m; Cindral has no completed-operation remeasurement. Undo.
5. Activate Move; click-release (0,0), move to (3,1), press **Right**, move toward (3,2). Expected: preview lies on red X. Press **Escape**: exact original geometry, no extra undo entry. Trial Q10 separately checks whether the lock persists; press **Down** in Cindral to clear it.
6. Start Move at (0,0), move toward (3,0), tap **Ctrl**, click-release destination. Expected: original and one translated copy. Cindral requires the separate **Copy** tool; verify that fallback too. Undo.
7. With only the first rectangle selected, use Move and click a corner of the second rectangle as the reference, then type **2m**, **Enter** while pointing red. Expected: only the preselection moves. Cindral unions the clicked geometry into targets. Undo.
8. Activate Rotate. Click-release (0,0) for pivot, click-release (2,0) for baseline, move toward (0,2), type **90**, **Enter**. Expected: quarter turn around (0,0). Cindral uses centroid instead and has no independent pivot/baseline stages; cancel if still active. Undo any completed operation.
9. Repeat Rotate with a visible blue plane, tap **Ctrl** before entering **90**, **Enter**. Expected: original plus rotated copy; Cindral lacks Rotate copy mode. Cancel and undo any change.
10. Activate Scale; click a corner grip, move outward, type **2**, **Enter**. Expected: uniform doubling about opposite corner. Cindral has no grips and scales around centroid; use its corner drag plus typed `2` only to compare resulting size. Undo.
11. Activate Scale; click a side-center grip, type **2**, **Enter**. Expected: only the selected axis doubles. Undo; repeat while tapping **Shift** to toggle uniform behavior. Cindral lacks side grips and this toggle.
12. Activate Scale; pick a corner grip, tap **Ctrl**, type **2**, **Enter**. Expected: centered proportional scaling. Undo; repeat without Ctrl to see opposite-corner anchoring. Cindral always uses centroid and cannot toggle anchors.
13. With first rectangle selected, start each tool using its button, move pointer until preview changes, then press **Escape**. Expected: exact starting geometry, same undo depth, tool no longer provisional. Repeat with an invalid typed value **2m trailing**, **Enter**: error appears and document stays valid; Escape restores original.
14. Make the first rectangle a group in each app via its context menu. Select the group, Move **2m**, Rotate **90**, Scale **2**, using the pivot/grip procedures above (Cindral's existing centroid workflow for its controls). Expected: instance stays a group, each completed operation adds one undo entry. Press **Ctrl+Z** three times, then **Ctrl+Y** three times: exact geometry states restored. Double-click group, select its face, repeat a Move and undo: operation belongs to editing context.
15. Run Q1–Q11 separately, then repeat steps 2, 8 and 10 on a roughly 20k-face model in **release mode**. Expected budget: preview <16ms, commit <100ms; record actual frame measurements. A CPU-only probe cannot establish this full-frame result.

## Validation and performance

Final test names, release CPU measurements and completion checks are recorded in the completion step below. All automated checks are GPU-free; frontend tests use synthetic egui frames. Imported .skp parity, complex folds, reflection orientation and GPU timings remain unverified. No dependency or non-test behavior changes are authorized by this report.
