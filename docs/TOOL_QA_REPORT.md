# Tool QA analysis

Date: 21 October 2026 (Asia/Manila)

## Result

Reviewed all 16 drawing and modify tools exposed by the palette. Found four functional issues affecting Rotate, Offset and Follow Me, plus one formatting failure. The existing suite passes despite these issues.

This was an analysis task: production code was not changed. Temporary reproduction tests were removed from the normal test suite after execution; their sources and logs remain in `.agent-out/tool-qa/`.

## Validation

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 245 passed, 0 failed, 0 ignored, including doc tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo fmt --all --check` | Failed: wrapping in `crates/cindral-app/src/lib.rs:303` |
| Additional targeted probes | Five assertions failed, reproducing four functional issues; one issue reproduced at both session and application levels |
| Native OpenGL launch and `capture_viewport --modify` | Succeeded; inspected 1280 × 800 capture |

The native capture showed the modeling workspace, shaded through-hole model and amber selection. An existing recovery prompt overlays the upper left. This is a rendering smoke check, not a manual interaction pass across every tool. Existing recovery data was left untouched.

## Tool coverage

“No additional issue detected” describes the exercised cases; it is not a guarantee that all workflows are defect-free.

| Tool | Coverage and outcome |
| --- | --- |
| Select | Existing tests pass for single/additive selection, connected selection, window/crossing selection, foreground picking and holes. No additional issue detected. |
| Line | Existing tests pass for chained segments, click order, typed lengths, preview consistency, cancel and elevated anchors. No additional issue detected. |
| Rectangle | Existing tests pass for face creation, tilted planes, typed dimensions and canvas undo/redo. No additional issue detected. |
| Circle | Existing tests pass for face creation, radius entry, segment counts, preview and tiny-radius rejection. No additional issue detected. |
| Arc | Existing tests pass for center/start/end order, sweep/radius, typed angles, segment counts and preview. No additional issue detected. |
| Polygon | Existing tests pass for face creation, click order, radius/segment settings and preview. No additional issue detected. |
| Push/Pull | Existing tests pass for target resolution, signed distance, previews, typed commit and kernel modification cases. No additional issue detected. |
| Move | Existing tests pass for absolute updates, live drag, cancellation, typed input and instance transforms. No additional issue detected. |
| Copy | Existing tests pass for copy command generation and transform/kernel behavior. No additional issue detected. |
| Rotate | Existing transform tests pass; application probe confirms axis lock is ignored when choosing the rotation axis. See QA-1. |
| Scale | Existing tests pass for pivot transforms, typed factor/percent and instance previews. No additional issue detected. |
| Offset | Existing typed-distance tests pass; diagonal drag probe confirms incorrect offset magnitude. See QA-2. |
| Follow Me | Existing ordered-path tests pass; new probes confirm typed fallback failure and silent invalid-path fallback. See QA-3 and QA-4. |
| Eraser | Existing tests pass for edge resolution, command generation and topology healing. No additional issue detected. |
| Tape Measure | Existing tests pass for distance, persistent guides, cancellation and no history/geometry edits. No additional issue detected. |
| Paint Bucket | Existing tests pass for resolved targets, color commands and material/render handling. No additional issue detected. |

Camera navigation is also covered by existing orbit/pan routing, cursor zoom, projection, standard views and extents tests. Sections, scenes, organization, files, inference and rendering were included in the full workspace run; this audit's targeted probes focused on the 16 palette tools.

## Findings

### QA-1 — Rotate ignores axis lock (P2)

Location: `crates/cindral-app/src/modify_ui.rs:243` and `:270`.

Reproduction: create an XY rectangle, choose Rotate, hover its face, press Right Arrow to lock X, then press the primary mouse button on the face.

Expected: rotation uses the locked X axis.

Observed: `axis_lock` is `Some(X)` but `ModifySession::axis()` is `(0, 0, 1)`. The application chooses the face/work-plane normal and passes it to `begin` without consulting the lock. The lock influences the inference query, not the rotation axis.

Impact: a requested rotation can act in the wrong plane. Honor the locked axis when establishing the rotation plane and axis, and define how a lock change during a gesture should behave.

Evidence: `app_probes.rs::qa_rotate_honors_x_axis_lock`, `app-probes.log`.

### QA-2 — Offset counts tangential motion as inset distance (P2)

Location: `crates/cindral-tools/src/modify.rs:765`.

Reproduction: on a 2 × 2 m XY square, begin Offset at the left-edge midpoint `(0, 1, 0)` and update to `(0.5, 1.5, 0)`.

Expected: the inset is 0.5 m, the perpendicular distance from that boundary.

Observed: the session emits an inset of `0.7071067811865476` m because it uses the full anchor-to-pointer distance. Moving along the boundary increases the inset even though the perpendicular displacement is unchanged.

Impact: diagonal drags produce unexpectedly large offsets and can reject otherwise valid insets near collapse limits. Resolve a boundary reference and project motion onto its in-plane perpendicular; use that signed distance for both preview and measurement.

Evidence: `probes.rs::qa_offset_drag_distance_is_perpendicular_to_boundary`, `probes.log`.

### QA-3 — Follow Me click-then-type can fail on an XY face (P2)

Location: `crates/cindral-tools/src/modify.rs:914`; application routing in `crates/cindral-app/src/modify_ui.rs:298`.

Reproduction: choose Follow Me on an XY rectangle, click its center without dragging, type `2m`, and press Enter.

Expected: the documented typed normal segment sweeps the profile along its normal and commits one undo step.

Observed: the application reports `invalid topology: sweep must start normal to the profile`. A session-only reproduction emits a path from `(1, 1, 0)` to `(3, 1, 0)`. The measurement fallback chooses X, which lies in the profile plane; the kernel correctly rejects it. The application-level reproduction confirms the failure also occurs through actual egui events.

Impact: the advertised typed workflow fails in a common construction plane. Use the resolved profile normal when no usable normal direction has been established, and check the resulting command through the history boundary.

Evidence: `probes.rs::qa_follow_me_typed_normal_segment_without_hover` and `app_probes.rs::qa_follow_me_click_then_type_normal_segment`; both log files.

### QA-4 — Follow Me silently discards invalid selected paths (P2)

Location: `crates/cindral-tools/src/modify.rs:600`.

Reproduction: select a profile and two disconnected path edges, then begin Follow Me.

Expected: report that the selected edges do not form a valid ordered path and preserve the user's opportunity to correct the selection.

Observed: path resolution returns `None`; `unwrap_or_default()` replaces it with an empty path and `begin` succeeds. Later updates switch to a cursor-generated segment. The same fallback also applies to branching paths rejected by the resolver.

Impact: an explicitly selected path is ignored without explanation, potentially producing a different sweep. Distinguish no selected path from a nonempty invalid path, and reject the latter with a clear error.

Evidence: `probes.rs::qa_follow_me_rejects_invalid_preselected_path`, `probes.log`.

### QA-5 — Formatting gate fails (P3)

Location: `crates/cindral-app/src/lib.rs:303`.

`cargo fmt --all --check` requests rewrapping the `drawing_drag` assignment. This blocks the documented formatting check; no functional defect is implied.

Evidence: `format.log`.

## Evidence and reproduction

Local evidence directory: `.agent-out/tool-qa/` (ignored by Git).

- `workspace-tests.log`: complete passing baseline run.
- `clippy.log`, `format.log`: static-check output.
- `probes.rs`, `probes.log`: three session-level reproduction tests and output.
- `app_probes.rs`, `app-probes.log`: two egui event-level reproduction tests and output.
- `native-capture.log`, `modify.png`, `modify.ppm`: native rendering evidence.

To rerun session probes, copy `probes.rs` to `crates/cindral-tools/tests/qa_audit_temporary.rs`, run `cargo test -p cindral-tools --test qa_audit_temporary -- --nocapture`, then remove the temporary copy. These assertions express the expected behavior and intentionally fail until the defects are fixed.

Application probes require crate-private access. Temporarily add a test-only module to `crates/cindral-app/src/lib.rs` pointing at `../../../.agent-out/tool-qa/app_probes.rs`, name it `qa_audit_temporary`, and run `cargo test -p cindral-app --lib qa_audit_temporary -- --nocapture`. Remove that module afterward.
