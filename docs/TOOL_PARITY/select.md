# Select parity audit

2026-10-22. Audit only; no production changes or SketchUp execution. Desktop behavior, paraphrased from established knowledge and the official [selection reference](https://help.sketchup.com/en/sketchup/selecting-geometry). Artwork, wording and branding differences are intentional. Evidence paths are repository-relative; line numbers refer to the audited production code. `matches` applies to the stated scope only. Windows Ctrl maps to Ctrl in Cindral Linux; macOS uses Option for selection addition/subtraction. Unknown details are trials below, not defects.

| Behavior | Expected (SketchUp) | Cindral status (matches / differs / missing / unsure) | Evidence (test name, file:line, or read the code) |
| --- | --- | --- | --- |
| A1 Activation | Space activates Select. | matches | `crates/cindral-app/src/lib.rs:422`; `selection_mode_keeps_drawing_shortcuts_and_scroll_zoom`. |
| A2 Cursor | Arrow; modifier indication. | differs | `workspace_ui.rs:432` always requests default arrow; no add/subtract cursor mode. Different art itself is fine. |
| A3 Status hint | Explain selection and modifier actions. | differs | `workspace_ui.rs:678` describes Shift as add; toggle/subtract modes absent. |
| A4 Measurements label/editability | Exact inactive field display needs Q1. | unsure | `workspace_ui.rs:653` always offers editable Dimensions with length hint, even in Select. |
| A5 Numeric input before/after click | No dimensional selection operation. | matches | `modify_ui.rs:130` collects text only during active modify operation; Select release returns without starting one. `select_parity_units_and_clicks_do_not_edit_geometry`. Covers `2m`, `150cm`, `3'6\"`; unit parsing is inapplicable to Select. Field appearance remains A4. |
| F1 Single face/edge | Replace selection with clicked entity. | matches | `selection.rs:42`; `modify_ui.rs:235`; public API and synthetic-egui tests below. |
| F2 Empty-space click | Clear selection. | matches | `modify_ui.rs:237`; `select_parity_empty_click_clears`. |
| F3 First press on empty space | May start a box; no geometry. | matches | `modify_ui.rs:222`; `select_parity_escape_cancels_box_without_model_change`. |
| F4 Box gesture | Hold, drag, release; no two-click box. | matches | `modify_ui.rs:221-243`; two separate clicks remain separate selection actions. |
| F5 Left-to-right window | Include fully enclosed entities. | matches | `Selection::region`; existing `test_left_to_right_window_vs_right_to_left_crossing`. Raw geometry scope only. |
| F6 Right-to-left crossing | Include touched entities. | matches | Same test; edge segment and face-interior overlap checks in `selection.rs:185`. |
| F7 Double-click face | Face plus boundary edges. | missing | `modify_ui.rs:231` handles triple click only; ignored `select_parity_double_face_expands`. |
| F8 Double-click edge | Edge plus adjacent faces. | missing | Same routing; ignored `select_parity_double_edge_expands`. |
| F9 Triple-click raw geometry | Connected geometry, excluding disconnected islands. | matches | `Selection::connected`; `triple_click_routes_connected_selection_and_tape_does_not_edit_history`. Explicit vertex IDs are additionally stored internally. |
| I1 Snaps and axis constraints | Selection uses screen picking, no placement constraints. | matches | Pick hit and region use cursor independently of inferred point (`modify_ui.rs:171,221`). Endpoint/midpoint/on-edge/on-face placement, axis colors, arrow/Shift inference holds are inapplicable. Arrow keys still set an unused global lock and may show a misleading status. |
| K1 Ctrl addition | Add without removing existing items. | missing | Only Shift is read at `modify_ui.rs:220`; ignored `select_parity_ctrl_adds`. |
| K2 Shift toggle | Toggle clicked membership. | differs | Shift only inserts; cannot deselect. Ignored `select_parity_shift_toggles`. |
| K3 Ctrl+Shift subtraction | Remove picked items. | missing | Shift causes addition regardless of Ctrl; ignored `select_parity_ctrl_shift_subtracts`. |
| K4 Alt alone | Version-specific effects need Q2. | unsure | No selection Alt handler; macOS Option addition is represented by K1, not assumed for Linux Alt. |
| S1 Existing selection | Ordinary click replaces; additive mode preserves it. | matches | `Selection::select`; scoped to API `add=true`, not Ctrl parity (K1). |
| S2 Group/component single selection | Select container atomically. | matches | `organization_ui.rs:1023` calls `select_instance` at `:180`; no geometry edit. |
| S3 Multiple containers/mixed selection | Permit a combined selection set. | missing | `OrganizationUi::selected` is one `Option<InstancePath>`; `select_instance` clears raw geometry; ignores modifiers. |
| S4 Box-select containers | Include containers by box rules. | missing | Region scans only current model vertices/edges/faces; no scene-instance iteration. |
| S5 Double-click container/edit context | Enter container; work in its context. | matches | `organization_ui.rs:978`; existing `scripted_three_instances_edit_in_place_exit_undo_reverts_all_and_redo_restores_all`. Nested edit geometry uses `history.model()` and context/world transforms. |
| S6 Select/Deselect All shortcuts | Ctrl+A / Ctrl+T on Windows. | missing | No such selection routing in `lib.rs` or `modify_ui.rs`; ignored `select_parity_ctrl_a_selects_all`. |
| G1 Geometry/model validity | Selection leaves topology and orientation intact. | matches | Read-only model input to selection; app tests snapshot exact model/history. No creation, merge, healing or normal changes applies. |
| G2 Face holes | Exact click/box corner cases need Q3. | unsure | Picking excludes inner loops; region checks hole rings; existing `test_face_occlusion_and_holes`. This establishes Cindral behavior, not all SketchUp boundary cases. |
| G3 Curved/soft geometry | Surface/facet selection details need Q4. | unsure | Picker tests every edge/planar face without soft/smooth filtering; no surface-expansion branch. Do not certify parity. |
| G4 Imported .skp geometry | Exact context/soft-edge handling needs Q5. | unsure | Same model picking after import; no Select-specific import regression or actual-file comparison run. No importer work in this audit. |
| U1 Escape during box drag | Cancel provisional action without editing model. | matches | `modify_ui.rs:97`; `select_parity_escape_cancels_box_without_model_change`. Exact resulting selection needs Q6. |
| U2 Escape with settled selection | Whether it clears selection needs Q6. | unsure | `cancel_modify` clears drag state but preserves selection. |
| U3 Undo/redo geometry and tool state | Selection adds no geometry undo step; geometry undo/redo remains available. | matches | `select_parity_undo_redo_resets_box`; `lib.rs:221` clears selection and pending box. No geometry operation is completed by Select, so one-entry-per-operation applies to edits, not selection clicks. Selection restoration parity remains Q7. |
| B1 Moving selection box | Show box before release. | matches | `modify_ui.rs:387` paints direction-dependent outline; selected membership changes on release. GPU appearance not tested. |
| B2 Selected highlight | Identify selected entities. | matches | `modify_ui.rs:363` outlines selected edges/face rings; `organization_overlay` bounds selected container. No required color/artwork matching. Face-interior distinction needs Q8. |
| B3 Empty/no target feedback | Empty click deselects without error. | matches | `modify_ui.rs:237`; empty click app regression. No invalid geometry command is attempted. |
| B4 Vertex-near click | Independent raw vertices are not ordinary selectable entities. | differs | `pick_section_impl` prioritizes `Hit::Vertex` before edges; Select stores it. Can leave only a vertex selected when clicking a corner. |
| P1 ~20k-face latency | Project target: live feedback <16 ms, release commit/release <100 ms on Ryzen 5 2400G. | unsure | No end-to-end Select timing yet. Release CPU probe added in test step excludes egui, inference, rendering/GPU. SketchUp timing also unmeasured; see Q9. |

## Questions for the user

Please include SketchUp version and platform once. Each trial resolves the listed uncertainty; do not treat a proposed observation as already verified.

1. **Q1 (A4):** Activate Select, inspect the Measurements label/value, click its field and type `2m`, Enter; then select a face and repeat with `150cm` and `3'6"`. Is the field editable, and does its label/value or selection change?
2. **Q2 (K4):** On Windows, select a face, hold Alt and click a second face. Does selection change differently from an ordinary click? Report macOS separately because Option is the known add modifier there.
3. **Q3 (G2):** Make a square with a cut-out square hole. Click the hole center, then drag a small right-to-left box entirely inside the hole, then across its rim. Which actions select the surrounding face or boundary?
4. **Q4 (G3):** Draw a cylinder, click its smooth side once, then show hidden geometry and click one side facet. Is the whole surface selected first and just a facet second? What does double-click add?
5. **Q5 (G4):** Import/open your .skp model, enter one component and click a soft-edged surface, then triple-click it. Does selection stay in that context and match the same actions on newly drawn geometry?
6. **Q6 (U1/U2):** Select a face, begin dragging a box in empty space, press Escape while holding the mouse and release. Is the old selection preserved? Select a face again and press Escape without dragging; does it clear?
7. **Q7 (U3):** Draw a rectangle, select its face, undo, then redo. Is the recreated face selected? Does the next click behave as a fresh selection click?
8. **Q8 (B2):** Select just a face, then deselect and double-click that face. Can you distinguish face-only selection from face-and-edges selection by the highlight? Cindral currently outlines both cases similarly.
9. **Q9 (P1):** In your ~20k-face model, drag a selection box and release in each app's release build. Is feedback immediate or visibly delayed? If available, report frame/release timings and hardware; subjective feedback alone cannot certify 16/100 ms.

## Side-by-side manual script

Use default shortcuts and top view in both apps. Start with two separated, visible square faces A and B with no shared vertices. Also prepare a group G containing a square and a cylinder C. Use each app's own controls to prepare this fixture; no matching icons or text required. Ctrl below means Windows SketchUp/Linux Cindral; use Option for macOS selection modifiers.

1. Hover the viewport; press Space. Expected: Select active, arrow cursor and selection guidance.
2. Click A's interior once, then B's interior once. Expected: only B selected, no geometry changes.
3. Click blank space. Expected: selection clears; no error or geometry change.
4. Click A; hold Ctrl, click B, release Ctrl. Expected: both faces selected. Known Cindral gap.
5. Hold Shift, click A, release Shift. Expected: A deselected, B retained. Known Cindral gap; repeat after restoring A+B if step 4 failed.
6. Select A+B again; hold Ctrl+Shift, click B, release both. Expected: only A remains. Known Cindral gap.
7. Click blank space; double-click A's interior. Expected: A and its boundary edges selected. Then blank-click and double-click the middle of one A boundary edge. Expected: that edge and adjacent face(s). Known gaps.
8. Click blank space; triple-click A's interior. Expected: A and all its connected geometry selected; B and G excluded.
9. Drag left-to-right from blank space to enclose A completely but only half of B; release. Expected: A's face/edges selected, B's face excluded; fully enclosed individual B edges may be included.
10. Drag right-to-left across part of A and B; release. Expected: touched faces/edges selected. Repeat in the opposite direction over identical bounds to compare crossing versus enclosure.
11. Select A; press in blank space and start a box, press Escape before releasing. Expected: no geometry edit and pending box cancelled; record selected-set outcome for Q6. Select A again, press Escape; record Q6.
12. Press Space, type `2m`, Enter over viewport; click A, type `150cm`, Enter; click the Measurements field and type `3'6"`, Enter. Expected: no geometry edits; record label/editability for Q1. Undo the last fixture geometry edit, then redo: exact geometry restored, no stuck drag (selection outcome Q7).
13. Click G once, then double-click G. Expected: whole container first, then entered context. Click its face and triple-click it: connected geometry stays inside G. Exit via each app's context control; Ctrl-click G and another container if available: combined selection expected, Cindral gap.
14. Click C's smooth side once; show hidden geometry and click a side facet. Repeat on an imported surface and run the hole trial Q3. Record Q3–Q5 outcomes; expected details unresolved, not pass/fail. On a ~20k-face document repeat steps 9–10 and record Q9 latency.

## Headless validation

Tests and final measured results are recorded below and in `docs/AGENT_LOG.md` after the test step. Ignored desired-behavior tests document gaps without changing production code. No GPU, SDK, SketchUp binary, assets, new dependencies or Python is used.
