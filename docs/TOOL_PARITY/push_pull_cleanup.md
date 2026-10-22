# Push/Pull cleanup audit

2026-10-22. Focused follow-up to the reported retained faces/edges. Audit only; production modeling behavior is unchanged. Reproductions use native drawn geometry and public commands, without imports, SDKs, external geometry or a running-window interaction.

## Confirmed findings

### P1: Local outward extrusion retains obsolete perimeter seams

Draw a 2 × 2 m square, pull it 1 m, then pull its top another 1 m. A uniform rectangular box should have 8 vertices, 12 edges and 6 faces. The actual result has **12 vertices, 20 edges and 10 faces**, including four horizontal perimeter edges at the previous top height. The previous horizontal cap itself is removed; the four original walls remain and four new coplanar wall faces are appended. Further outward pulls can retain additional wall subdivisions.

`crates/cindral-core/src/local_extrusion.rs:98–111` includes only the source boundary in the patch and copies its original edges into the result. Lines 120–125 move the cap, and lines 132–152 add wall faces without extending/healing their existing coplanar neighbors. No subsequent seam cleanup runs on this fast path. `validate()` and closed-volume checks accept these subdivisions, so existing validity/volume tests miss the defect. The reproduced volume remains the correct 8 m³, and undo/redo are exact.

Suggested repair: include affected neighboring wall topology in the local patch, join compatible coplanar walls, remove redundant perimeter edges and heal unused/collinear vertices. Preserve intentional material/style boundaries, unrelated geometry and history. Simply deleting the old perimeter edges would leave the existing face loops invalid. Falling back globally requires performance evaluation because this fast path exists to avoid connected-model reconstruction.

### P1: An attached open sheet retains the starting face beneath the extrusion

Draw two adjoining 2 × 2 m squares in one plane and pull one 1 m. The result has **7 faces**: the untouched square plus a complete six-face prism. The old shared edge has **three incident faces** (the base, neighboring sheet and new wall). `Model::validate()` accepts this open/nonmanifold surface arrangement. This is a concrete retained-face case; unlike an isolated square's useful bottom closure, the attached sheet needs an explicit replacement/healing policy.

`local_extrusion.rs:126–130` unconditionally adds the source face back as a base whenever the input is not closed. The general path has the same base-retention policy: `modification.rs:390–425` builds a capped prism, and lines 701–710 append the remaining open surfaces. The public-command regression exercises the local path; the general-path observation is source inspection, not a separate runtime reproduction.

Suggested repair: distinguish isolated profiles from faces attached to an open surface; replace the selected sheet region without automatically retaining its base when connected-sheet editing calls for removal. Keep a standalone square's bottom closure. Cover partial boundaries, holes and intentionally nonmanifold input before broadening the policy.

## Passing controls and scope

- Simple inward pushes of −0.5 m on the 1 m box remove the old cap and ring and leave 8 vertices/12 edges/6 faces with correct volume. Exact-depth (−1 m) and beyond-depth (−2 m) pushes of the entire top remove the entire box without residual faces, edges or vertices. This does not certify subdivided opposite faces or arbitrary intersections.
- Replacing provisional outward distances 0.25/0.5/0.75 m does not accumulate intermediate rings or faces; the original 1 m seam persists. Cancellation restores the exact starting model and previews add no undo entries.
- Renderer CPU-cache verification compares incremental slots and active geometry against a fresh rebuild after extrusion, successive previews, cancellation, commit, undo/redo and complete deletion. This distinguishes retained kernel entities from stale CPU mesh slots; it does not certify GPU upload, depth artifacts or the user's specific running document.

No user file was provided, so the exact reported scene is unverified. Both defects above independently reproduce without a renderer. No performance claim or benchmark is added.

## Reproduction tests

`crates/cindral-core/tests/push_pull_cleanup_audit.rs`: four ordinary audit/control checks and two ignored desired-behavior regressions. The ignored checks are intentionally failing assertions documenting unresolved defects, not passing fixes.

```sh
cargo test -p cindral-core --test push_pull_cleanup_audit
cargo test -p cindral-core --test push_pull_cleanup_audit -- --ignored --nocapture
cargo test -p cindral-render push_pull_cleanup_audit
```

The ignored run is expected to exit unsuccessfully until the cleanup defects are fixed. Completion validation is recorded in `docs/AGENT_LOG.md`.
