# Phase 2 texture storage design (decision gate)

Status: **decided — proceed with Option A (budget-sized uniform array with
mipmaps and anisotropy)**. This document records the options and the memory
math that selected it, per the session rule: if the best option exceeded the
256 MiB texture budget or needed a large renderer rewrite, Phase 2 would stop
and report instead of implementing.

## Constraints

- Hardware target: Ryzen 5 2400G with Vega 11 integrated graphics sharing
  system RAM. Texture budget adopted for this decision: **256 MiB**, leaving
  headroom for geometry buffers, shadow maps (a 4096² depth map is 64 MiB),
  offscreen targets and the OS.
- Backend reality: the GL (Mesa/GLES) backend drives the design. WGSL cannot
  dynamically index bind groups, and `texture_2d_array` layers must share one
  size. One array = one binding = today's shader structure stays.
- Asset limits already in force: 127 embedded texture assets (slot 0 is the
  white fallback), SHA-256 content-addressed slots (identical images dedup),
  PNG/JPEG only, decode bounded by the IO layer.
- Phase 2 quality goals: mipmaps + anisotropic filtering (≥4× when supported)
  so floors and wood do not shimmer or blur at grazing angles; correct sRGB
  sampling (already `Rgba8UnormSrgb`); mapping (size/rotation/flip) lives in
  the material table from Phase 1.

## Measured today (baseline)

One 128-layer array of fixed 256×256 `Rgba8UnormSrgb` layers, one mip level,
Linear/Linear, no anisotropy:

    128 × 256 × 256 × 4 B            = 33.55 MiB

## Options

### A — budget-sized uniform array (chosen)

Keep the single texture array; all layers share one adaptive edge length `L`
(one uniform array, never per-size-class arrays); add a CPU mip chain (×4/3
bytes) and an anisotropic sampler (hardcoded 16, which wgpu clamps internally
to the adapter maximum). `L` is picked per document from the asset count so
the estimated resident set stays inside the budget:

    estimate(n, L) = (n + 1) × L² × 4 B × 4/3

One 1024² layer with mips is 16/3 MiB ≈ 5.33 MiB; one 512² layer with mips is
4/3 MiB ≈ 1.33 MiB.

| L | layers that fit ≤ 192 MiB working target | resident at 127 layers |
| --- | --- | --- |
| 512 | 144 (the 127-asset cap needs 128) | 170.7 MiB |
| 1024 | 36 | — (rejected at 127) |

Policy: `L = 1024` while the document has ≤ 35 assets (36 layers × 5.33 MiB =
exactly 192 MiB), else `L = 512` (≤ 127 assets, 170.7 MiB). A decode that
would cross the edge threshold re-creates the array once and re-uploads
(rare, user-visible only as a brief one-time cost). Original encoded bytes
stay untouched in `.cdraft`; only the GPU copy is resampled.

- 4096×4096 import: the decoded source is 64 MiB of transient CPU memory and
  only one asset is ever decoded at a time (decode → resample to L → mip
  chain → drop, before the next asset), so peak decoded memory is one asset,
  not all assets. The GPU resident copy is one L² layer with mips ≈ 5.33 MiB
  at `L = 1024`. Upload = resample the decoded source directly to L (box
  prefilter + Catmull-Rom, never through a smaller preview) plus an 11-level
  mip chain. If the asset count later crosses the edge threshold, every
  layer is re-resampled from the original embedded bytes.
- Mip levels below the base are averaged in premultiplied alpha and
  un-premultiplied afterwards, so transparent texels cannot bleed dark
  colors into visible edges; the base level and all opaque textures are
  pixel-identical to the resampled source.
- Array creation and upload run inside wgpu error scopes: an allocation
  failure reports a clear status-bar message and leaves the previous array
  and the document unchanged.
- Fits the budget, touches only `create_materials_texture_array`, the layer
  preparation, the sampler and their tests — no shader restructure.

### B — size-class arrays

One array per class (256/512/1024/2048) so a few hero textures keep detail.
Costs: four bind groups and per-material class branching in WGSL (arrays are
separate bindings), padding waste (a 513 px image occupies a 1024 layer),
eviction rules when a class fills. Example mix that fits: 4×2048 (89.5 MiB) +
24×1024 (134 MiB) — already 223 MiB with only 28 images. Rejected for this
milestone: a moderate renderer rewrite for a capacity this use case does not
need; revisit if furniture imports routinely exceed ~30 detailed textures.

### C — LRU residency at native resolution

Full-size textures paged in/out on demand. Requires eviction that renumbers
layers (material-table churn), thrash risk when orbiting between rooms, and
per-frame memory variance on shared RAM. Rejected: largest rewrite, least
predictability, unnecessary at ≤127 assets.

## Decision

Option A. Numbers for the implementation contract:

- Default `L = 512`, upgrading to `L = 1024` only for documents with ≤ 35
  assets; both stay under the 192 MiB working target and the 256 MiB budget.
- Mipmaps: CPU chain, each level half the previous, averaged in premultiplied
  space, uploaded once per layer change; `mip_level_count = log2(L) + 1`.
- Filtering: trilinear (`Linear`/`Linear`/`Linear` mipmap) with Repeat
  addressing, plus `anisotropy_clamp = 16` (wgpu clamps internally to the
  adapter maximum; at least 4× when supported).
- Texture bytes on disk remain the original encoded images; dedup by SHA-256
  is unchanged. The 127-asset cap keeps its clear rejection message.
- Style or shadow toggles only touch uniforms; texture (re)uploads happen on
  asset-table changes only — asserted by regression.
