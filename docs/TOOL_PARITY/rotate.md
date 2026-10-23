# Rotate behavior parity

Behavior implementation from the user's 2026-10-23 requirements and conventional choices where uncertain. No reference application, SDK, code, icons or assets were used. Independent Cindral artwork. No affiliation implied. Earlier transform audit is historical; this table supersedes its Rotate findings.

`RU`: `crates/cindral-app/src/rotate_ui.rs`; `MS`: `crates/cindral-tools/src/modify.rs`; `RK`: `crates/cindral-tools/tests/rotate_kernel.rs`; `DH`: `crates/cindral-core/src/document.rs`.

| Expected | Status | Evidence |
| --- | --- | --- |
| 1. Q, Rotate cursor, Angle caption | Implemented; native interaction trial pending | Q/pivot regressions in modify_ui; shared cursor/status/readout |
| 2. Preselection required, no hover-target expansion | Implemented | RU `rotate_hint`, explicit selected IDs/paths; `locks_copy_toggle_and_no_hover_target` |
| 3. Center/reference/end clicks, typed after center | Implemented for loose and occurrence targets | RU shared route; `three_clicks_center_only_typing_reentry_and_escape` |
| 4. Escape clears text, then exact cancel; clean restart | Implemented, including focused Angle field | Shared input capture/cancel; RK preview cancellation and RU scripted Escape stages |
| 5. Camera-side protractor, ticks, axis/neutral colors | Implemented; visual comparison pending | RU projected 72-segment ring, ticks, center/reference overlay |
| 6. Cursor/face orientation, horizontal fallback | Implemented conventional edge/endpoint fallback | RU active/child face inverse-transpose normal; world Z for empty/edge/point |
| 7. Arrow toggle plane locks; Shift orientation hold | Implemented | RU pre-center world locks; scripted arrow toggles |
| 8. Shared inference and tooltip at center/reference | Implemented | Existing Engine query, including picked child occurrence frame; existing overlay enabled |
| 9. Document increment; near-ring snap, far free; wedge/live angle | Implemented conventional threshold | `rotation::snap`; RK threshold tests; persisted `angle_snap_degrees`, default 15 |
| 10. Signed degrees/slopes, invalid preservation, completed re-entry | Implemented | Shared parser/state and RK actual kernel repeated-entry tests |
| 11. Documented/tested sign | Implemented | `rotation::angle/matrix`, RK sign/arbitrary-plane/inverse properties |
| 12. Ctrl / macOS Option toggle copy; cursor + | Implemented; macOS runtime pending | RU modifier rising edge and shared Transform copy/placement preview |
| 13. Nx/*N//N; replace array; one undo | Implemented with count choice below | RK radial-array coordinates/history; shared Batch/DuplicateMany/TransformSet |
| 14. Rigid loose/group/component/mixed targets; definitions preserved | Implemented on existing selection paths | MS Transform; DH shared placement helper; mixed mirrored/two-component and nested-context app tests |
| 15. Shared Move connectivity; reject invalid partial result | Implemented conservative rejection; no autofold | RK connected-box partial test; RU invalid live preview cancels with reason |
| 16. Active context isolation and world protractor plane | Implemented | RU transforms rays/anchors/normals; nested scaled-context geometry/definition/undo assertions |
| 17. Per-step hints | Implemented | `rotate_hint`: center/start/sweep and selection-required idle hint |
| 18. Release preview target <16ms, minimum 33ms | Measurements recorded below; workload-specific | Real-file replay and synthetic whole-definition kernel workload, both externally bounded |
| 19. One undo; exact undo/redo; no preview entries/topology rebuild | Implemented for supported operations | Existing local/graph deltas; cached whole-definition transform; RK/RU history tests |

## Chosen conventions

The normal faces the camera at center placement and stays fixed throughout the operation. Positive angles are right-handed: +90° sends +X toward +Y about +Z, counterclockwise when looking at the plane from the normal-tip/camera side. A typed angle immediately after center uses the projection of world +X onto the plane (world +Y if X is nearly normal) as baseline. Later entries replace the absolute angle from the original baseline.

The protractor aims for 55 logical pixels along its most visible projected basis. Snap applies when the planar pointer radius is between 0.75 and 1.25 times its world radius, inclusive; outside that annulus rotation is free. Perspective and an oblique plane can make the screen-space band elliptical. The document setting is an integer degree increment from 1 to 90 (default 15), available by right-clicking the Angle caption. Old documents default to 15.

`3x` and `*3` mean **three additional copies**, at one/two/three times the established angle. `/3` means **three additional copies** at one-third/two-thirds/all of that angle. The original remains; the prior destination is one of the copies, not an extra fourth copy. Retyping replaces that entire array in its one undo slot. Counts are bounded by the shared parser (256). Connected raw copies that overlap may be rejected by the existing kernel; they are never accepted as invalid geometry.

Shift-select containers and loose geometry to retain mixed targets. Multiple occurrence targets must be direct children of the active edit context; a container carries its complete nested subtree. Instances change placement only. Partial raw selections retain Move's connectivity behavior: unsupported nonplanar/crossing results cancel with a message, rather than adding autofold behavior.

## Questions for the user

1. With nothing selected in SketchUp or SketchUp for Web, press Q and click a face, then try a rotation: does it rotate that hovered face, or require selection? Cindral requires selection.
2. In empty space press Q, then hover an edge midpoint and an endpoint: does the protractor stay horizontal, or choose another plane? Cindral uses horizontal world Z unless a face or lock supplies a plane.
3. Set a Rotate center/reference in SketchUp, sweep near its ring, then move radially outward without changing direction: where does 15° snapping stop? Cindral uses the inclusive 0.75–1.25 radius annulus.
4. Copy-rotate a selected group 30°, then type `3x` Enter: are there three copies plus the original, or three objects total? Cindral creates three copies plus the original.
5. Repeat the 30° copy and type `/3` Enter: count the copies and their angles. Are they 10°, 20°, 30° plus the original? That is Cindral's choice.
6. Select just one top corner of a box in SketchUp and rotate it 37° about a lower corner: are diagonal fold edges added automatically, and does undo remove them? Cindral rejects any result that its shared Move connectivity path cannot keep planar and valid.

## Side-by-side manual script (12 steps)

Use meters, decimal point, Top view, and a fresh 2m × 1m loose rectangle for each independent step. Call its lower-left A=(0,0,0), lower-right B=(2,0,0), upper-left C=(0,1,0). Select the entire rectangle before each trial unless told otherwise. Q means Rotate; Ctrl means copy on Windows/Linux, Option on macOS. Restore/reselect after undo. These are behavioral comparisons, not artwork comparisons.

1. Clear selection, press **Q**, click A. Expected Cindral: “Select objects to rotate,” no geometry/history change. Select the rectangle, press Q: expect Angle caption, Rotate cursor and “Click to set rotation center.” Record question 1 in the other app.
2. Before clicking, hover the face interior, then B, then empty space. Press **Up**, **Up**, **Right**, **Right**, **Left**, **Left**, **Down**, **Down**. Expect blue/red/green plane locks toggling on/off, Down holding/releasing the inferred plane. Hold **Shift**, move to another surface, release: orientation stays held only while Shift is down. Record question 2.
3. Press Q, hover A until its endpoint tooltip appears, click-release A; hover the midpoint of AB for its midpoint tooltip, click-release there. Expect fixed center A, baseline along AB, no model change and the sweep hint. Hover other endpoints/edges to compare shared inference feedback.
4. Press Q, click-release A, then type **45 Enter** before placing a baseline. Expect one 45° rotation from the default planar baseline. Type **-45 Enter**, then **22.5 Enter**: the same operation changes absolutely. Undo once restores the original; redo once restores 22.5°.
5. Press Q, click-release A, click-release B, move toward +Y without holding a button, click-release at C. Expect a live quarter-turn preview before the last click and one committed +90° operation. Undo/redo exactly once. Drag-release alone must not complete the three-click operation.
6. Press Q, click-release A, click-release B. Sweep just beside the ring near 29°: expect a 30° wedge/readout with the default setting. Move farther than 1.25 ring radii at the same bearing: expect a free angle. Right-click Angle, set **10°**, repeat: near-ring snapping uses 10°. Record question 3.
7. Press Q, click-release A; type **123**, press **Escape**: text clears, center stays active. Press Escape again: exact original geometry. Repeat after baseline B and a visible sweep preview; repeat with no center. Restart Q/A/B afterward: no stale angle, copy or lock remains.
8. Press Q, click-release A, type **1:2 Enter**, then **8:12 Enter**, then **-1:2 Enter**. Expect approximately 26.565°, 33.690°, -26.565° as absolute replacements. Type **1:0 Enter** and **nonsense Enter** using the Angle field: expect error/highlight and unchanged geometry/history. Escape twice ends eligibility.
9. Press Q, tap **Ctrl/Option**, click-release A, click-release B, type **45 Enter**. Expect original plus rotated copy and a + cursor before completion. Tap the modifier twice between clicks in another trial: mode returns to its starting value. Undo one step restores only the original.
10. Use a grouped rectangle positioned away from its rotation center to avoid overlapping raw copies. Q, Ctrl/Option, click center 5m left of A, click A as baseline, type **30 Enter**, then **3x Enter**, **/3 Enter**, **2x Enter**. Expect respectively three copies at 30/60/90°, three at 10/20/30°, then two at 30/60°. One undo removes the entire array; redo restores it. Record questions 4–5.
11. Create a component rectangle and a second occurrence. Mirror/nonuniformly scale one occurrence; put it inside a rotated group. Enter that group, select its child, perform Q/center/reference/**37 Enter**. Expect rigid world rotation of the occurrence with definition geometry unchanged. Exit; Shift-select both occurrences plus a separate loose rectangle, rotate once, undo once: all selected targets restore together; unrelated objects remain unchanged.
12. Select just one upper box corner, Q, click a lower corner as center, click another lower corner as reference, type **37 Enter**. Expect either a valid shared-connectivity result or a rejection restoring the preceding model, with a status reason. Record question 6. Finally run the release replay below on the private file and compare timed sweeps; visual smoothness alone is not a timing measurement.

## Validation and timing

Targeted tests and final workspace results are recorded in the agent log. The release CLI imports the supplied private file through the existing production importer, selects a visible top-level container, and runs the actual three-click input route. Its 60 sweep samples include inference, CPU scene synchronization, full egui layout and tessellation. GPU-completed rendering is included only when `CINDRAL_BENCH_GPU=1` successfully obtains an adapter; an unavailable adapter prints a skip. It does not time vsync, native presentation, loading or first-placement setup as sweep frames.

```sh
cargo build -p cindral-app --features kernel-benchmark --example rotate_benchmark --release
timeout 90s target/release/examples/rotate_benchmark /home/earl/Desktop/Cindral/Drafter/SKP/mybedroom.skp
cargo test -p cindral-core --features bench-fixtures --release --test rotate_performance --no-run
 timeout 60s cargo test -p cindral-core --features bench-fixtures --release --test rotate_performance -- --ignored --nocapture
```

The synthetic workload is explicitly **19,600 faces, all vertices selected**, 40 absolute Rotate updates. It measures the kernel/history subset, not the full viewport, and asserts a 33ms maximum. It also validates geometry, exact undo/redo, a subsequent indexed edit and undo across that edit. Cached whole-definition transforms reuse Move's original-value deltas and spatial/render frames. Partial selections and copies retain the general shared transactional path and can be slower.

Real-file GPU-completed sweep maxima were **9.619ms** initially and **5.108ms** with strengthened actual-placement assertions; report **9.619ms**, the maximum across both runs. CPU-only maximum was **1.075ms**. Synthetic 19,600-face whole-definition kernel mean/max were **1.867/8.114ms**, commit **0.005ms**. See [raw bounded summaries](rotate-release.txt). These results establish these specific workloads, not every loose/partial/copy selection or native presented frame. No reference-application validation was performed.
