# Paint / Materials UX review

2026-10-25 · review only · branch `agent/paint-ux-review`

## Current state and evidence

**Recommend D:** keep a reorganized Materials workspace in the inspector, with Browse / Edit / Library views, and turn the existing active-material header display into a quick picker. The right side is a good home for persistent properties; the current long, shared stack is a poor home for frequent paint choices. Useful foundations already exist: search, usage sorting, virtualized document rows, live edits, undo, an Eyedropper alternative to Alt, and modifier-sensitive status text.

The actual inspector tabs are **Model / Appearance / Recovery**. Materials is an open-by-default collapsing section under Model, after Outliner and Tags (`organization_ui.rs:483`; `workspace_ui.rs:724`). MATERIALS.md's “Materials tab” description does not match this checkout. Advanced mapping controls already appear only for textured materials (`materials_ui.rs:475`); preserve that behavior.

Capture method: rebuilt the existing `capture_viewport` example, then used an equivalent temporary native eframe wrapper with the same `DrafterApp::update` and native `ViewportCommand::Screenshot` readback. The wrapper lived under `/tmp`, forced theme/window size, supplied synthetic pointer input, and seeded three simple faces, two colors and one original generated texture. Temporary settings supplied a two-image folder and three Recent swatches. No private model or external vendor assets were used. “Default” means no named paint material selected, not an empty document. Screenshots are full-window PNGs, without resizing; textured views are scrolled to expose mapping controls and library views show the folder thumbnails. These are layout captures, not representative room geometry or a user study.

Screenshot paths (all under `docs/ui-review/`; columns are the five requested states):

| Size / theme | Default | Flat color | Textured editor | Folder open | Bucket hover |
| --- | --- | --- | --- | --- | --- |
| 1366×768 light | [default](ui-review/1366x768-light-default.png) | [flat](ui-review/1366x768-light-flat.png) | [textured](ui-review/1366x768-light-textured.png) | [library](ui-review/1366x768-light-library.png) | [hover](ui-review/1366x768-light-hover.png) |
| 1366×768 dark | [default](ui-review/1366x768-dark-default.png) | [flat](ui-review/1366x768-dark-flat.png) | [textured](ui-review/1366x768-dark-textured.png) | [library](ui-review/1366x768-dark-library.png) | [hover](ui-review/1366x768-dark-hover.png) |
| 1920×1080 light | [default](ui-review/1920x1080-light-default.png) | [flat](ui-review/1920x1080-light-flat.png) | [textured](ui-review/1920x1080-light-textured.png) | [library](ui-review/1920x1080-light-library.png) | [hover](ui-review/1920x1080-light-hover.png) |
| 1920×1080 dark | [default](ui-review/1920x1080-dark-default.png) | [flat](ui-review/1920x1080-dark-flat.png) | [textured](ui-review/1920x1080-dark-textured.png) | [library](ui-review/1920x1080-dark-library.png) | [hover](ui-review/1920x1080-dark-hover.png) |

Code references below use MU = `crates/cindral-app/src/materials_ui.rs`, OU = `crates/cindral-app/src/organization_ui.rs`, WU = `crates/cindral-app/src/workspace_ui.rs`, at the reviewed checkout. Evidence distinguishes visible capture results from code-derived behavior.

At 1366×768 the inspector body runs approximately y=164–702 (538 px); the unedited Materials section starts near y=349 and reaches the folder heading near y=694, using about 345 px. At 1920×1080 the body reaches y=1014 (850 px), but the list reserves more height: the default section reaches the folder heading around y=819 (470 px). A selected texture adds a long editor, requiring another scroll position at both sizes. Expanded builtins add roughly another screen or two. Controls are mostly stacked rather than overlapping; the texture import row forces width growth and can clip at the right edge while scrolled. These are capture coordinates at 100%, not scaled-device guarantees.

## New-user walkthrough

Counts are **analytical action counts**, informed by the capture interactions, not timed human trials. Start on Model, with Materials expanded, Outliner/Tags collapsed, no selection, and the bucket inactive unless stated. Existing-material tasks assume the named materials exist. A drag counts as one pointer action; typing and Enter are separate keyboard actions, not clicks. A scroll is one directed excursion to another control group, not a wheel notch. Scroll estimates are lower bounds for compact / large windows; nested list scrolling and expanded Outliner add more. Confusion counts are predicted distinct decision points, not measured mistakes. Native file-picker navigation was not walked.

| Task | Clicks / pointer actions | Scroll excursions 768 / 1080 | Confusion moments | Walkthrough and friction |
| --- | --- | --- | --- | --- |
| a. Two walls, same solid color | 5 | ≥1 / ≥0 | 2 | Open Builtin collection → choose first visible swatch → Paint → wall → wall. Finding the collection and identifying unnamed swatches are the two decisions. Existing listed color reduces this to 4 clicks. |
| b. Switch among three colors | 6, bucket already active | ≥1 / ≥0 | 2 | Select each existing material → paint, repeated three times. The short list can clip a row at 768 px. Recent looks like the faster route, but edits the current material rather than choosing three separate paint materials (F02). |
| c. Wood floor, realistic plank width | Builtin path blocked; existing texture: 4 | ≥2 / ≥1 | 3 | Pattern thumbnail cannot activate as authored (F01). With an existing wood material: choose it → width field, type e.g. `180mm`, Enter → Paint → floor. Need to identify width, understand repeat size versus a single plank, and interpret the aspect lock; an image containing several planks needs a correspondingly larger repeat. |
| d. Recolor every user of a material | 3 + typing | ≥1 / ≥0 | 2 | Choose material → hex field → Set. Shared editing is supported; neither the editor nor Recent announces the number of affected users. Exact hex field starts with a placeholder, not the selected color. |
| e. Identify a face's material and edit | 5 + typing | ≥1 / ≥0 | 3 | Paint → header Eyedropper → face → hex field → Set. Ordinary face selection does not synchronize the editor; sampling nested geometry may pick a container assignment instead (F05). |
| f. Replace matching material | 3 | ≥0 / ≥0 | 2 | Source row's ⋯ → Replace → target row. The target click applies immediately. Faster bucket alternative: choose target → Paint → Shift-click source (3); its implementation is scoped differently from a whole-document replacement (F03). |
| g. Remove unused materials | 1 | ≥1 / ≥0 | 1 | Purge unused. Count and hover undo text help; “unused” may include the currently chosen paint material if it has not been applied. |
| h. Own texture folder → image | ≥5 attempted, blocked at thumbnail | ≥1 / ≥1 | 2 | Folder library → Add collection → navigate/select folder → confirm → thumbnail. Picker costs vary. Folder scanning/thumbnails were captured using temporary settings; image has hover sensing, so the final click handler cannot fire as written (F01). Load PNG/JPEG is an alternative on an already selected material. |
| i. Discover Ctrl / Shift / Alt | 1 + hover, modifier holds | 0 / 0 | 2 | Activate Paint. Plain status hint lists Ctrl, Shift and Alt; hold each to read its action. Ctrl+Shift is absent from the initial hint and requires guessing the combination. Scope/selection consequences remain unclear. No paint click is required to inspect hints. |

## Findings

Effort: S = under a day; M = a few days; L = a week or more. Severity refers to the affected workflow; an alternative workflow may still exist.

| ID | Problem | Evidence (capture, action count, code) | Severity | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- |
| F01 | Pattern and folder images look actionable but do not opt into click sensing. | Tasks c/h blocked. MU:1030–1033,1116–1119 use `Image` then `.clicked()` without `.sense(Sense::click())`; pinned egui 0.33.3 `widgets/image.rs:81` defaults to hover. Library captures show the targets. | blocks work | Use accessible image buttons or explicit click sense, visible focus and names; verify actual activation. | S |
| F02 | Browsing a color and editing shared paint are conflated. Recent recolors the selected material; builtins create new materials. | Task b: 6 safe list clicks versus a misleading Recent shortcut. MU:840–872,1132–1143. Flat captures show Recent directly under shared properties. | slows work | Label Edit with affected-user count; make quick-pick entries choose material IDs. Keep any “recent color values” editor explicitly separate. | S |
| F03 | Hover and hint do not reliably describe the click's scope. | MU:1441–1454 paints the whole selected-face set, while 1532 previews one face; Ctrl+Shift on a loose face produces no region at 1508, yet paints at 1441. Alt is absent from hover key at 1501. Container preview stores `instance`, but 1578 returns when faces are empty. Shift uses the active model at 1447, not every definition. Plain hover captures establish only the simple-face case. | slows work | Share one action/scope calculation between click, preview and hint; show target/context and count, including selection, sampling and container cases. Avoid claiming whole-model Shift scope until it matches. | M |
| F04 | Long nested scrolling and content-dependent width displace the modeling view. | WU:713–718 gives nominal 320 px, 40 px padding; MU:221 list nested in WU:754 outer scroll. Compact flat editor expands inspector to about 354 px; texture fields and folder controls require separate scroll positions. Builtin colors are 40 vertical allocations, not a grid (MU:1001–1017), roughly 1,080 px for swatches alone. | slows work | Give Materials its own inspector destination with Browse/Edit/Library subviews, stable width, one main scroll area per view, and a wrapped named swatch grid. | M |
| F05 | Selection, sampled appearance and active paint are different states without enough explanation. | Task e: 5 clicks. OU:1108–1126 changes selection without material sync; MU:1327–1341 samples raw container/face assignment. No mixed-material inspection state in this panel. | slows work | Show “Selected face: …” separately from “Painting: …”, including effective/inherited material and a mixed state; explicit Use for painting action. | M |
| F06 | Texture form exposes an asset slot and ambiguous physical dimensions. | Textured captures; MU:434–460 exposes internal slot; 506–525 stacks unlabeled width, ×, height and a link glyph. Reset's 96-DPI explanation is hover-only (610). Task c has 3 interpretation points. | slows work | Width/Height labels with units; “Size of one image repeat”; preview with plank-count explanation; named aspect lock. Put asset key under Advanced. Keep mapping conditional on texture. | S |
| F07 | Destructive/shared actions have weak consequence cues. | Task f/g. MU:934 Delete applies immediately; 979 choosing replacement applies immediately; 887 Purge applies immediately. Delete is beside Duplicate. Purge has count and one-undo-step tooltip, but no persistent Undo cue; Delete/Replace omit scope/count in the action UI. | slows work | Separate destructive actions; show source → target and affected users/context before Replace; confirm used-material Delete, and offer a clear Undo result. Counted unused-only Purge can stay one click with Undo. | S |
| F08 | Several targets miss 24 px; row sizing disagrees with panel styling; keyboard list navigation is missing locally. | MU:108 row height 22 vs WU:77 interact height 30 plus 9 px spacing; Compact Default captures clip the fourth row despite four logical rows; the large view instead reserves substantial blank list space. Swatches: 14 px list, 16 px Recent, 18 px builtins (277,850,1007). ⋯ tooltip says only Rename although it opens all actions (297). MU list has no arrow-key handling. | slows work | Align virtual row stride with rendered height; make full rows and swatches ≥24 px targets; name ⋯ “Material actions”; add arrows/Enter and focus indication. Preserve existing Tab behavior where widgets support it. | S |
| F09 | Library/builtin view does avoidable work on an integrated GPU laptop. | MU:1023–1028 generates six patterns and calls `load_texture` each open frame; 1102–1123 builds every folder image (up to documented 2,000). In-model list is already virtualized, so this is a separate weakness. No new performance benchmark was run. | slows work | Cache pattern previews and virtualize a bounded thumbnail grid; reuse texture handles and preserve asynchronous scans. Measure before adding a second always-open panel. | S |
| F10 | Empty, disabled and invalid states provide uneven guidance. | Default captures explain clearing materials, but no direct Create/Import CTA. MU:308 returns before Recent; 891 disables panel Eyedropper with Default selected despite header sampling being available. Invalid mapping returns false silently (659–678); hex errors reach status (426). Search retains Default even for no matches (219). | slows work | Offer Create/Import and enabled sampling from Default; explicit no-results message; local invalid-value hint; retain status details and progress/error feedback. | S |
| F11 | Theme text is generally readable, but weak labels are tiny and slider tracks have little visual separation. | Both-theme textured captures; WU:17–40 palette, 80–83 13 px body/buttons; MU small/weak usage, mapping and Recent labels. Normal text colors give approximately 9:1 or better against their panel backgrounds; this does not validate every composited state. | polish | Keep the shared palette; increase essential labels/counts, strengthen slider tracks/focus/disabled distinction; inspect at laptop scaling. | S |
| F12 | Active paint already has two cues, but neither provides quick switching or texture identity. | WU:510–528 header has name + 14 px hover-only swatch and Eyedropper; WU:620–635 also draws a colored cursor dot, contrary to MATERIALS.md's “not attached to pointer” statement. Header swatch shows base color even for textures. MU:1562 lists three modifiers; status label truncates with hover expansion (WU:913). | slows work | Extend the existing header into a clickable named/thumbnail chip and quick picker, with discoverable shortcut and complete modifier help. Keep concise click-action text in status. | S |

## Placement decision

| Option | App-specific assessment |
| --- | --- |
| A: unchanged inspector | Lowest implementation cost, but keeps panel competition, scroll travel and viewport movement. Not recommended. |
| B: reorganized inspector | Good baseline: uses established panel conventions and preserves viewport space. Still makes frequent color changes require travel to the right edge. |
| C: dockable/floating Materials | Useful later for multi-monitor specialists; on a 1366×768 laptop it adds occlusion, focus/window management and another live view. No demonstrated GPU advantage; thumbnail work still needs fixing. Defer. |
| **D: B + active chip + quick pick** | **Recommended.** Persistent editing stays consistent with Model/Appearance panels; an on-demand picker supplies frequent paint choices without keeping a library open. Extend the current header rather than duplicating it. Keep the inspector width stable at both resolutions. |
| E: viewport-only palette/radial menu | Less travel, but image browsing and precise texture-size edits need labels and stable space. Could supplement D later; unnecessary now. |

Proposed 1366×768 layout (logical pixels at 100%; illustrative, not implemented). Inspector ≈320 px, tool rail ≈132 px, viewport ≈914 px. Reserve header/status and keep browsing within the remaining ≈600 px.

```text
+---------------------------------------------------------------+
| menus / file                                                 |
| Undo  Fit   Paint   [wood thumbnail Oak v]  Eyedropper  [?]     |
+------+---------------------------------------+----------------+
|tools |                                       |Model Appearance|
|      |                                       |Materials       |
|      |                                       |[Browse] Edit Lib|
|      |                 viewport              |Search... Sort v|
|      |                                       |Recent materials|
|      |  click chip / assigned shortcut:      |[A] [B] [C]     |
|      |  +---------------------------+        |In model        |
|      |  | Search...                 |        |[ ] Oak    8 ...|
|      |  | Recent / In model         |        |[ ] Clay  12 ...|
|      |  | thumbnail + name + usage  |        |  bounded list  |
|      |  | arrows choose; Enter uses |        |                |
|      |  +---------------------------+        |Purge unused (2)|
+------+---------------------------------------+----------------+
| Paint 8 matching faces in Room > Floor with Oak | Measurements |
+---------------------------------------------------------------+
Edit view: name / color / opacity / "Used by 8"; texture-only
Width [180mm] Height [...] [Lock aspect]; Rotation; Advanced v.
Library view: Builtins / Folders; cached, virtualized thumbnail grid.
```

Do not automatically replace the active paint when selecting geometry. Synchronize a separate inspected-material readout, with an explicit “Use for painting” action. This prevents inspection from unexpectedly changing the bucket. Quick pick should reuse virtualized in-model data, omit shared-edit controls, close on Escape, and restore viewport focus. Choose its shortcut through the existing shortcut system after checking conflicts; none is implemented by this review.

## Next coding sessions (maximum eight)

Each item is a bounded session deliverable; a complete placement redesign spans multiple sessions. Keep model/schema changes out of the UI tasks unless separately justified.

1. **S — Restore thumbnail activation and target sizing:** image buttons with names/focus; ≥24 px swatch targets and correct material-row height. Fixes F01, target/row portion of F08. Validate picking a pattern and a folder image.
2. **S — Make editing consequences explicit:** label shared Edit with usage count; separate Recent color-value editing from material selection; add Default Create/Import and enabled sampling. Fixes F02, Default portion of F10.
3. **S — Repair active-context paint preview:** derive face-selection, Ctrl+Shift and Alt preview/hints from the existing click scope; clear preview in sampling mode. Fixes the active-context portion of F03. Defer nested-definition behavior to its own session.
4. **S — Add Materials inspector destination and Browse/Edit/Library navigation:** retain existing views initially; one scroll owner per view, stable width, wrapped builtins. Fixes layout portion of F04; preserves consistency with the rest of the inspector.
5. **S — Clarify texture editing:** labeled repeat dimensions/units, aspect control, local invalid-input message, Advanced asset slot. Fixes F06 and mapping portion of F10.
6. **S — Add selected-material readout:** single face / mixed selection / inherited material, with explicit Use for painting; leave the active slot stable. Fixes F05.
7. **S — Extend existing active chip into a minimal quick picker:** material IDs, recent/in-model tabs, search, arrows/Enter/Escape, texture thumbnails and shortcut registration. Fixes F12, keyboard portion of F08 and remaining frequent-switching part of F04.
8. **S — Add action safeguards:** correct ⋯ wording, source/target/count cues and Undo result, with used-material Delete confirmation. Fixes F07 and the action-label portion of F08.

F09 caching/virtualization and F11 visual polish remain follow-ups outside this eight-session list. Complete nested-context scope parity separately after item 3; the M-sized findings span several bounded sessions.

## Limits and questions for a real screen

I can view the captured images. This was a code-backed heuristic review with synthetic interaction, not nine completed novice user trials. It does not establish human error rates, native picker usability, keyboard accessibility end-to-end, real-room texture scale, cross-definition replacement correctness, or frame rate. Small test textures do not reproduce a 2,000-image library. The plain face hover is visible; complex scope risks above are from code comparison, not a full geometric interaction matrix.

Check by eye on the actual laptop: 100%/125%/150% scaling; row clipping and viewport movement while choosing materials; subdued tracks and tiny usage labels in both themes; cyan hover against light/blue textures; status truncation with long material names and a busy inspector; Alt window-manager capture; focus/arrow/Tab behavior; Delete/Replace/Purge recovery; invalid dimension feedback; whether a repeat width matches the visible number of planks. Confirm that the cursor dot represents texture identity adequately (it probably does not).

1. When you select a face, should inspection follow it while the paint choice stays fixed?
2. Are three-color changes more frequent than editing a shared material's properties?
3. What laptop scaling and minimum inspector width do you normally use?
4. Do you expect Shift-replace across the whole document, the current edit context, or a selected object?
5. For wood images, do you know the image repeat dimensions, or would a plank-count aid be useful?

## Review log and validation

Only this report and the small PNGs are committed, as requested; this section records the review instead of modifying AGENT_LOG.md. No application code, schema or tests changed; no sub-agents or OpenCode used. Scope: requested docs/log tail, the Materials/organization/inspector layout and their input/status paths, plus minimal capture/settings/file API and pinned widget-default inspection needed to create original fixtures and verify sensing.

Repository-required completion checks were invoked once: formatting and strict all-target/all-feature workspace Clippy passed. The workspace all-feature test log contains 610 passed, 0 failed, 48 ignored across 56 result summaries, ending with successful documentation tests; the enclosing tool session reported exit 143, so this is logged test evidence rather than a verified clean process exit. No benchmark was run or performance improvement claimed. All 20 native captures were viewed in a contact sheet; individual editor/library/hover views were also inspected. Exact dimensions and state coverage were verified; PNG sizes are 164,395–293,239 bytes (each under 300,000 bytes). The temporary capture wrapper needed deterministic input-file writes and an explicit monitor-size-clamp override; these were capture-only adjustments outside the repository.
