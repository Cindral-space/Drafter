# Push/Pull cleanup audit and fixes

Initial audit: 2026-10-22. Fixes: 2026-10-23, following the reported `.skp` case. The findings below record the original behavior; both now have enabled passing regressions. No SDK or SketchUp execution is used.

## Resolution (2026-10-23)

- Both local and general extrusion now omit the starting cap for attached open surfaces. Isolated profiles retain their bottom closure.
- The local supporting-face path includes compatible adjacent walls and heals their coplanar seams within that patch. It protects border vertices used outside the patch and retains distinct color/material boundaries. It does not rebuild the entire connected object to clean up a local wall extension.
- The general path preserves the selected face ID when the translated cap survives reconstruction intact, maintaining repeated tool previews and later operations.
- Import regressions cover closed boxes, attached planar sheets, folded open surfaces, exact preview cancellation and undo/redo. An opt-in `CINDRAL_SKP_CLEANUP_FILE` test imported the private `SKP/mybedroom.skp` in place, exercised an attached face in each of its two nonempty definitions, verified no identical starting face remained, validated the results and restored the exact original geometry on cancellation. The private file is not copied or staged. This samples two faces; it does not certify every face or arbitrary intersections.
- A connected subdivided-solid regression checks that repeated extension keeps entity counts stable, modifies fewer than twenty faces and preserves a closed manifold with exact undo/redo. A release-only 20,006-face CPU probe covers initial and repeated extrusion; measurements and final checks are in `docs/AGENT_LOG.md`.

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
- Replacing provisional outward distances 0.25/0.5/0.75 m does not accumulate intermediate rings or faces; after the fix, the original 1 m seam is also removed. Cancellation restores the exact starting model and previews add no undo entries.
- Renderer CPU-cache verification compares incremental slots and active geometry against a fresh rebuild after extrusion, successive previews, cancellation, commit, undo/redo and complete deletion. This distinguishes retained kernel entities from stale CPU mesh slots; it does not certify GPU upload, depth artifacts or the user's specific running document.

The original audit had no user-selected file. The follow-up sampled the supplied private file as described above. Both original defects also reproduce without a renderer. Native GPU/window-manager interaction remains unverified.

## Reproduction tests

`crates/cindral-core/tests/push_pull_cleanup_audit.rs`: the original controls and both former ignored regressions are enabled, plus a painted-wall boundary regression.

```sh
cargo test -p cindral-core --test push_pull_cleanup_audit
cargo test -p cindral-render push_pull_cleanup_audit
cargo test -p cindral-io --test pushpull_import
```

Completion validation is recorded in `docs/AGENT_LOG.md`.
