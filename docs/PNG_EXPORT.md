# Exporting a current view as PNG

Choose **File > Export > Image (PNG)…**. Choose Match window (physical viewport pixels), 1920×1080, 3840×2160, or Custom width/height, then Off / Medium / High antialiasing and optional Transparent background. Choose a destination and confirm replacement if it exists. Completion shows the path and output dimensions. Export settings and last folder are saved atomically in `cindral-drafter/png-export.json` under the platform configuration directory (`XDG_CONFIG_HOME` when set). Overwrite permission is never remembered.

The snapshot uses the current camera/projection, visible document geometry, style edges/profiles, material colors, decoded texture previews and opacity, section cut and filled caps, and document sunlight/shadows. It preserves Shaded, Textured, Hidden Line, Wireframe and X-ray behavior, including existing transparency sorting and 256-pixel texture-preview limitations. Background, sky and ground follow the viewport's existing horizon. Transparent mode omits that environment fill; actual geometry and the enabled shadow ground receiver remain visible.

Selection highlights, tool previews, construction guides, grid, axes, section-plane widgets and UI are excluded. No overlay checkboxes are provided in this slice. A pending modeling operation is cancelled before taking the export snapshot. No batch, PDF, print layout, animation, video, tiled giant images or alternate renderer is included.

## Framing and color

The existing camera and fit behavior use a fixed **vertical** field of view; orthographic projection uses a fixed vertical span. Export preserves that behavior and the camera target/distance. A wider aspect extends the sides; a narrower aspect crops them. The options dialog explains this policy. The viewport size and export size agree exactly at the same aspect; picking and inference are untouched. Sky/ground horizon math is shared with the viewport.

Off renders one sample; Medium renders 2× width and height; High renders 3× width and height. The production SceneRenderer supplies geometry, section and shadow passes, followed by GPU readback. CPU resolution averages premultiplied samples, composites environment when requested, and unpremultiplies for straight-alpha PNG. Fully transparent RGB is zero. The usual egui gamma framebuffer uses the existing scene bytes directly; an sRGB framebuffer applies its transfer after linear composition. Export does not double-encode gamma. Readback removes wgpu's 256-byte row padding without reversing top-to-bottom rows. PNG includes an sRGB chunk and only `Software=Cindral Drafter` and the document basename in text metadata; no full source path.

## Hardware limits and failure behavior

Both output and supersampled render dimensions must fit **8192** and the adapter's texture limit. Total estimated export working set must fit **512 MiB**: color/depth targets, padded mapped readback, CPU samples/output/encoder, four times uploaded scene data for buffer growth/copies, plus 128 MiB texture/shadow/device headroom. This is an estimate, not a total-process RAM guarantee; the already open editor, model topology, driver and OS use additional shared RAM. Original decoded images count in scene data. GPU allocation/validation/internal errors and device/poll failures become export errors.

The app refuses over-budget requests; it does not silently resize. Reduce size or AA. In particular, 3840×2160 High needs a 11520-wide target and is refused; 8192² exceeds the memory budget even at Off. High at 2048×1152 is a measured conservative 16:9 upper benchmark size on the instance fixture; actual limits depend on scene data. No tiling is implemented. A preset is a requested size and may need a lower AA setting to fit.

Scene preparation, a separate window-free GPU device, rendering, readback, conversion and encoding run on a worker. Progress reports phases and rows. Cancel is checked between phases/rows; submitted GPU work must finish (30-second completion timeout), so cancellation is not instantaneous. Cancellation and final publication share a lock: a successful cancellation leaves no output; after publication Cancel reports completion. Temporary files are removed on handled failure/cancel, and atomic publication prevents exposing partial PNGs. A process/power crash may leave an unreferenced temporary file, but cannot truncate an existing destination. A destination racing into existence is preserved unless overwrite was confirmed. Missing/unwritable parents and disk-write failures report errors.

## Command line

```sh
cindral-app --export-png in.cdraft out.png --width 1920 --height 1080 \
  --aa medium [--transparent] [--scene 'Scene name'] [--force]
```

Width and height are required positive integers. AA is `off`, `medium` (default), or `high`. One named scene restores its camera, tag/object visibility, style, sections and shadows before rendering. Unknown scenes/flags fail. `--force` permits replacement; otherwise existing files are preserved. The CLI does not modify desktop export preferences.

The native archive does not store the live unsaved viewport camera. Without `--scene`, CLI uses the default camera fitted to document bounds; desktop export uses the actual current camera. This is one scene at a time, not batch export. No visible window is created. A suitable headless wgpu adapter is required; GL backends may require the platform display server. Missing adapter/device fails with a clear error. `WGPU_BACKEND=gl` selects the local Vega 11 GL path for measurements.

## Validation and release measurements

Formatting and strict workspace/all-target/all-feature Clippy pass. The full all-feature workspace suite passes **460 tests, zero failures, 45 ignored workloads**. Nine targeted PNG tests cover size/memory, framing, row/alpha/color behavior, settings, atomic output/overwrite/Unicode metadata, failure and concurrent cancellation. Real GPU checks cover cube pixels/dimensions, transparent corners, section/shadow differences, saved-scene CLI style and production viewport callback agreement in gamma and sRGB framebuffers (tolerance two byte levels). Readback row padding/orientation also has a render-crate unit test. GPU tests skip only when an adapter/device cannot be obtained.

Measured 2026-10-23 on the local **AMD Radeon Vega 11 / OpenGL / Mesa 26.2.3**, using the existing **5,000 shared cube instances**, default Textured style, default environment, shadows off. One fresh release process per size, sequentially with no concurrent compiler/agent, each bounded by a 90-second external deadline. GPU/driver shader caches may already be warm. Time includes snapshot validation, scene preparation, device creation, GPU completion/readback, conversion, PNG encoding and atomic publication; fixture construction is outside timing. RSS is process VmHWM (including fixture setup), **not total system/shared GPU RAM**, so it must not be treated as actual GPU allocation measurement. These are single completed-export samples, not p95 or editor frame-rate claims.

| Requested output / High AA | Complete export | Peak process RSS | Conservative export estimate | PNG bytes |
| --- | ---: | ---: | ---: | ---: |
| 1920×1080 (5760×3240 render) | 455.220 ms | 233132 KiB / 227.67 MiB | 432 MiB | 715072 |
| 2048×1152 (6144×3456 render), near budget | 493.151 ms | 242836 KiB / 237.14 MiB | 474 MiB | 770849 |
| 3840×2160 | Refused before GPU work: 11520-wide render | N/A | Dimension cap | No output |
| 8192×8192 | Refused before GPU work: 24576² render | N/A | Dimension cap | No output |

Build with `cargo build --release -p cindral-app --features kernel-benchmark --example png_benchmark --bin cindral-app`; run `WGPU_BACKEND=gl timeout 90s target/release/examples/png_benchmark WIDTH HEIGHT`. The example creates one image per process and cleans it up. [Raw measurements and commands](benchmarks/2026-10-23-png-export.txt) include the fixture's separate CPU setup measurements; they are not added to the export timer. There was no large imported model in `tests/fixtures/skp-local/`; imported-model export performance remains unmeasured. No private file was copied or committed. Native desktop interaction and Windows/macOS runtime were not manually exercised; the desktop options/menu compile and the shared production callback/worker path is tested.


Desktop export retains the active component editing context and its existing inactive-object dimming. Extremely thin image aspects outside the existing camera range (1:1000 through 1000:1) are refused rather than silently changing framing. Document text uses UTF-8 iTXt for Unicode filenames.

## Questions for the user

1. In real SketchUp, export the same view at a narrower aspect ratio: does it crop the sides or change vertical framing?
2. In real SketchUp, export transparent PNG with ground shadows enabled: does the shadow receiver remain opaque?
3. In real SketchUp, export once while editing a component: are other objects dimmed in the image?
