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

The panel lives in the right inspector (Model / Appearance / Materials tabs).
It shows one row per material with a color swatch, name and usage count
(number of faces using it). Rows are virtualized — only visible rows are
built — so a 5,000-material palette stays responsive.

- **Search** filters by name; **Sort** switches between name and usage count.
- **Actions** per material (with one-entry undo): Rename, Duplicate, Delete
  (faces fall back to the default material), Select Faces (selects every
  face using it), Replace (repaint all its faces with another material),
  Purge (delete unused materials in one command).
- **Editor**: HSV/hex color editing with live viewport preview while a
  slider is dragged; the committed edit is exactly one history entry on
  release. A 12-entry recent-colors row persists between sessions.
- **Library**: 40 builtin colors in four groups and six procedural tileable
  patterns generated on demand (patterns are never embedded in files; they
  are regenerated on load). Folder import scans a chosen directory
  off the UI thread, shows 64 px thumbnails, caps the scan at 2,000 files
  and remembers the folder list.
- **Texture import** loads a user-chosen PNG/JPEG file into the document
  store (see limits below) and assigns it to the material.

## Paint Bucket

The Paint Bucket paints materials only (the old flat-RGBA session paint path
was removed).

| Input | Behavior |
| --- | --- |
| Click | Paint the clicked face (or the clicked group/component instance). |
| Click on selected object | Paint the whole selection. |
| Ctrl + click | Fill the clicked face plus all connected adjacent faces. |
| Shift + click | Replace every face using the clicked face's current material. |
| Ctrl + Shift + click | Replace the material only inside the clicked object. |
| Alt + click | Sample the clicked face's material into the active slot; never mutates. |

- **Sampling**: Alt+click picks up the material under the cursor. On Linux,
  window managers that grab Alt for window dragging also have an
  **Eyedropper button** in the panel, which performs the same non-mutating
  sample with a click.
- **Hover outline** previews the fill extent before clicking (face, connected
  region, or same-material region per modifier). Regions beyond 256 faces
  show the object's bounding box instead of per-face outlines.
- The **active material** is shown as a cursor swatch, and the status bar
  hint reflects the current modifier.

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
- The size editor has a **lock-aspect toggle** (on by default): editing one
  dimension scales the other to preserve the image aspect; unlock to set
  independent width/height.
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
  image twice stores one asset.
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
