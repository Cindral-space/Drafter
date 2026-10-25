# Materials and painting

This document describes material properties, color storage, the Materials
panel, the Paint Bucket tool, texture import/storage limits and the file
schema migrations as implemented in this milestone. The texture-storage
design rationale (options and memory math) lives in
[TEXTURE_DESIGN.md](TEXTURE_DESIGN.md).

## Material properties

A material has:

- **Name** — free text; duplicates and deletions are undoable one-entry
  history commands.
- **Color** — stored as display sRGB bytes; see below.
- **Opacity** — 0–100 % slider in the editor; blends over what is behind.
- **Texture** — optional reference to one PNG/JPEG asset in the document's
  content-addressed texture store.
- **Texture mapping** — real-world size, rotation and flips; see below.

Faces without an explicit material use the default material; instances can
carry a material inherited by unpainted descendant faces.

## Color storage and shading (sRGB)

Colors are authored and stored as **display sRGB bytes** (0–255 per channel)
and converted to linear light only when shaded, so the picker swatch, the
panel swatch and the viewport render the same color. The storage is schema 2;
schema-1 files stored linear floats and are converted on load
(`linear_rgb_to_srgb`), so an old file renders exactly as it did before the
migration. glTF import/export converts linear color factors; OBJ/DAE/SKP
channels are already sRGB and pass through unchanged.

## Materials panel

Materials has its own destination in the right inspector, beside Model,
Appearance and Recovery. Choosing Paint opens Materials. The inspector keeps
its width while switching among three views:

- **Browse** chooses the active paint material. Search and Name/Usage sorting
  operate on a virtualized list with 32 px rows, named swatches/thumbnails,
  usage counts, and a Material actions menu. Click a row, then use Up/Down to
  move through the list. New color opens Edit; Import image creates a material
  from a PNG/JPEG; Collections opens Library. Default clears an assignment.
- **Edit** changes a shared material, with an explicit usage count: edits update
  every user. Name, HSV/RGB, exact hex and opacity editing retain live preview
  and undo. The recent **color values** in this view recolor the shared material;
  they are deliberately separate from the quick picker's recent **materials**.
  Default offers New color, Import image and Eyedropper actions.
- **Library** contains a wrapped grid of 40 original named colors, six cached
  procedural pattern buttons, and folder collections. Folder scans run off the
  UI thread (64 px thumbnails, at most 2,000 files). Folder image grids build
  only visible rows and reuse texture handles; the scan's thumbnail pixels are
  borrowed rather than deep-cloned every frame. Pattern/image buttons support
  actual clicking, keyboard focus and accessible names. Folder lists persist.

A selection readout follows face/container selection independently of the active
paint slot. It shows inherited and mixed face materials and offers **Use for
painting**. Container selection reports the container assignment, rather than
claiming that all descendant faces use one material. Inspecting geometry does
not silently change the paint choice.

**Quick pick:** click the named active-material chip in Paint's header, click
Paint again when it is already active, or press **Shift+B** with the viewport
focused/hovered (the normal canvas-shortcut rules apply). Search the In model or
Recent materials lists; Up/Down changes the focused row, Enter chooses it, and
Escape/outside click closes without painting. Choosing a material changes only
the active slot, not its shared properties. Recent material IDs are session-only,
limited to 12 and reset when the document is replaced. Recent color values remain
persisted independently. Texture chips use an already prepared 64 px mip, without
decoding source images during paint frames; a color swatch remains the fallback
until preparation completes.

**Actions:** Rename, Duplicate, Select users, Replace throughout document and
Delete. Used-material Delete requires confirmation showing the affected usage
count and Default fallback. Replace first selects a target, then explicitly
applies to all document users, including definitions and material-carrying
instances. Purge unused shows a count, disables when nothing is unused, and may
remove an unused active paint material. Delete/Replace/Purge provide an Undo
result while that action is still the latest document revision. A later edit
hides the action-specific Undo button so it cannot undo an unrelated change.

Usage counts count painted faces in every definition plus material-carrying
instances; they are assignment counts, not the number of visible copies.

## Paint Bucket

The Paint Bucket paints materials only (the old flat-RGBA session paint path
was removed).

| Input | Behavior |
| --- | --- |
| Click | Paint the clicked face (or the clicked group/component instance). |
| Click on selected object | Paint the whole selection. |
| Ctrl + click | Fill the clicked face plus all connected adjacent faces sharing the clicked face's material. |
| Shift + click | Replace matching faces in the current edit context. Use the material action for replacement throughout the document. |
| Ctrl + Shift + click | Replace the material only inside the clicked object. |
| Alt + click | Sample the clicked face's material into the active slot; never mutates. |

- **Sampling**: Alt+click picks up the material under the cursor. On Linux,
  window managers that grab Alt for window dragging also have an
  **Eyedropper button** in the panel, which performs the same non-mutating
  sample with a click.
- **Hover outline** previews the fill extent before clicking (face, connected
  region, or same-material region per modifier). Regions beyond 256 faces
  show the object's bounding box instead of per-face outlines.
- The **active material** is shown in a named header chip with a texture
  thumbnail where available, plus a base-color cursor dot. The chip opens quick
  pick; the nearby help button explains all modifiers, including Ctrl+Shift.
- Click execution and hover use the same paint plan, including selected faces,
  selected containers and the current definition's matching-face scope. Alt and
  Eyedropper clear the paint preview. Object plans draw world-space bounding
  boxes; face plans use their definition's transform. Status text gives the
  planned face/object counts and the active material. A shared component's
  definition edit affects all copies; Ctrl+Shift does not make a copy unique.
- Sampling uses an explicit hit-face material before walking its instance path
  for inherited material, so sampling a painted face inside a container does
  not accidentally choose only the container assignment.
- Ctrl+Shift on a container operates on matching faces directly in that
  container definition; it does not recursively replace every nested definition.
  Shift is likewise scoped to the active context, not a global document action.

## Texture size, rotation and flip

The mapping is real-world: `width`/`height` are the physical size of one
texture repeat in meters, `rotation` turns the texture about the mapping
origin (−360…+360 degrees), and `Flip H` / `Flip V` mirror it. Order is
flip → rotate → scale, matching the vertex shader exactly; the same
transform is applied by the OBJ/glTF/DAE exporters when they write UVs.

**UV origin per format**: the viewport and glTF use a top-left UV origin
(V down); OBJ and COLLADA (DAE) use a bottom-left origin (V up). The
exporters therefore write OBJ/DAE V as `1 - V` applied as the final step on
the final UVs (after size, rotation and flip), and keep glTF as computed —
texture orientation matches the viewport in convention-respecting
consumers.

- A newly imported texture defaults to its **physical size at 96 DPI**
  (pixels ÷ 96 × 25.4 mm). Tiny images whose DPI size would be under
  5 cm on an edge fall back to 1 m wide with the aspect preserved.
- Texture-only Edit controls label **Width** and **Height** as the size of one
  image repeat, accept explicit units, and explain that an image containing
  multiple planks repeats as a whole. The raw texture asset reference lives
  under Advanced. Invalid sizes show local feedback and retain editable text.
- The size editor has a **lock-aspect toggle** (on by default): editing one
  dimension scales the other to preserve the image aspect; unlock to set
  independent width/height. Size text edits preview valid values live and
  commit **one** undo entry on Enter or focus loss (Escape reverts); zero,
  negative or incomplete values never reach the document. Rotation commits
  one entry per drag; the flip toggles and "Reset to image size" commit one
  entry per click.
- A material without a mapping (`None`, the pre-schema-3 state and schema-2
  files) keeps the original planar look: one repeat per meter, no rotation,
  no flip.

## Texture import caps

Per image, enforced at import with clear rejection messages:

- **Formats**: PNG and JPEG only.
- **Encoded size**: ≤ 32 MiB per file.
- **Dimensions**: ≤ 8192 × 8192 px.
- **Decoded size**: ≤ 256 MiB (e.g. 8192² RGBA).
- **Dedup**: assets are content-addressed by SHA-256; importing the same
  image twice stores one asset. De-duplication merges on a full hash
  collision (two different images with the same digest would share one
  slot); this is accepted as practically impossible rather than guarded
  against.
- **Count**: a document holds at most 127 texture assets (slot 0 is the
  white fallback).

Original encoded bytes are stored untouched in `.cdraft`; only the GPU copy
is resampled.

## Texture-array sizing and budget

All texture layers live in one uniform `texture_2d_array` whose square edge
length `L` adapts to the document (never per-size-class arrays):

- `L = 512` by default; upgraded to `L = 1024` while the document has ≤ 35
  assets (36 layers × 5.33 MiB = exactly 192 MiB).
- Every layer is resampled from the decoded source directly to `L` (box
  prefilter plus Catmull-Rom) and carries a full CPU-built mip chain averaged
  in premultiplied alpha (×4/3 memory), so transparent edges stay free of
  dark halos. Sampling is trilinear with Repeat addressing and anisotropic
  filtering (`anisotropy_clamp = 16`, clamped internally by wgpu to the
  adapter maximum; at least 4× when supported).
- Resident estimate `(assets + 1) × L² × 4 B × 4/3` stays inside the
  192 MiB working target under the 256 MiB texture budget: 128 layers at
  512² ≈ 170.7 MiB; 36 layers at 1024² = 192 MiB. A decode that crosses the
  edge threshold re-creates the array once and re-uploads from the original
  embedded bytes.
- Import decodes one asset at a time (decode → resample to `L` → mip chain →
  drop before the next), so peak decoded memory is one asset even with 127
  large textures.
- If the GPU cannot allocate the array, a clear message appears in the
  status bar and the previous textures remain in use.
- Texture (re)uploads happen only on asset-table changes; style or shadow
  toggles touch uniforms only (asserted by regression).

Rationale and rejected alternatives: [TEXTURE_DESIGN.md](TEXTURE_DESIGN.md).

## Measured performance

All numbers are release-mode runs of the real private file
`SKP/mybedroom.skp` (207 faces) on AMD Radeon Vega 11 / Mesa GL
(`WGPU_BACKEND=gl`), warmup excluding one-time pipeline creation; they are
editor-responsiveness figures, not frame-rate guarantees for arbitrary
documents.

- **Color tick** and **texture-size tick**: 60 sequential edits through the
  live editor path (material-table preview + scene sync + a *full* UI+GPU
  frame — egui tessellation, draw and GPU completion). Reported as *worst*
  and, for color ticks, *mean* over the 60 frames; target < 16 ms. Review
  run (2026-10-25): color worst **8.10 ms** / mean 5.30 ms; size worst
  **9.95 ms**. Re-measure after the mapping-editor fix (same machine):
  color worst 8.19 ms / mean 3.09 ms, size worst 4.07 ms — the size figure
  depends heavily on background GPU state; both runs are inside the target.
- **Paint-all** and **undo + repaint**: CPU-side command + mesh-sync cost
  only (no full-frame render), target < 100 ms each. Review run: **0.839 ms**
  and **1.25 ms**.
- **Texture upload** (one-time per texture change): CPU decode of a
  generated 4096² PNG + resample to the 1024² array edge + premultiplied
  mip chain + layer sync — measured **714.8 ms** on the re-run above
  (605–616 ms in the previous session; run-to-run variance). Opening a
  document or attaching a texture runs this preparation on a worker thread
  with a "Preparing textures k/n" status; the GPU upload then stages at
  most one texture-array layer per frame, so the UI stays responsive
  (measured: 10 large textures preparing, worst full UI frame 19.1 ms
  against a 33 ms budget; the old synchronous path cost 3.75 s on the UI
  thread). Import already prepared off-thread; headless PNG export keeps
  the synchronous path.
- **GPU resident copy**: reported by `gpu.allocated_bytes()` as a constant
  ~1.44 MiB on Mesa GL because wgpu's counter does not include GL texture
  storage; the analytic array bytes for one 4096² asset are the 2-layer
  1024² array with mips ≈ 11.2 MiB (see [TEXTURE_DESIGN.md](TEXTURE_DESIGN.md)).

## File schema versions and migrations

`.cdraft` documents carry `schema_version`:

| Version | Change | Migration |
| --- | --- | --- |
| 1 → 2 | `Material.color` linear floats → sRGB bytes | Tolerant deserializer converts on load; resaving is stable and renders identically. |
| 2 → 3 | `Material.texture_mapping` added (`Option<TextureMapping>`) | Missing field defaults to `None`, preserving the one-repeat-per-meter planar look. |

Migration guarantees: loading never discards or rewrites material data; the
converted value renders exactly as the pre-migration file did, and saving
after load writes the current schema (the round trip is idempotent).

## Out of scope for this milestone

- Per-face UV editing or per-face texture positioning (no "Texture Position"
  mode).
- PBR properties (roughness, metalness, normal maps).
- Back-face (two-sided) materials.
- Correct sorting of transparent objects (opacity blends without depth
  ordering guarantees).
- Importer UV derivation: re-importing a file replaces authored UVs with
  native planar mapping from the material's size/rotation/flip.

## Questions for the user

Each of these can be answered by trying one thing in the reference desktop
application or its web version; the conventional choice is implemented until
then.

1. **Multi-selection painting** — with two objects selected, does clicking one
   of them paint both selected objects, or only the clicked one? (Implemented:
   clicking a selected object paints the whole selection, both objects.)
2. **Sampling binding** — is Alt+click really the conventional sample
   (eyedropper) binding for the Paint Bucket, and does the reference app
   offer a separate Eyedropper tool/button? (Implemented: Alt+click plus a
   panel Eyedropper button for the Linux Alt-grab case.)
3. **Default texture size** — for an imported image without explicit
   metadata, is the default physical size pixels-at-96-DPI, the image's
   embedded DPI metadata, or a fixed one-repeat-per-meter rule? (Implemented:
   96 DPI with a 1-meter fallback for tiny images.)
4. **Shift-replace scope** — does Shift+click replace the material across the
   whole model, or only inside the clicked group/component? (Implemented:
   whole model; Ctrl+Shift limits it to the clicked object.)
5. **Rotation behavior** — does rotating a texture in the reference app
   rotate about the face-local mapping origin (our choice) or about the
   face/camera center, and is the rotation sign clockwise on screen?
