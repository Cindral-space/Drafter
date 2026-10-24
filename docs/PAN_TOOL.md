# Pan

Choose **Pan** in the toolbar, **Camera → Pan**, or press **H** over the canvas.
The status bar says “Drag to pan the view.” Drag with the left button; the hand
closes while dragging and opens on release. Pan remains active, including after
Escape ends a drag. Choosing another tool ends Pan. Text editing keeps H as text.

Shift + middle drag temporarily pans in any tool and preserves unfinished work.
Both routes use `Camera::pan_at_depth`: perspective captures the picked point's
view-axis depth once at press; absent, invalid or behind-camera hits use target
depth. Depth is bounded by the near plane / 0.001 minimum and 1,000,000 maximum.
Orthographic scale uses view height and camera distance. The model follows the
mouse without smoothing. Camera position and target translate together; orientation,
FOV and projection stay fixed. Pan-mode normalized two-axis scroll points also pan
(using raw egui scroll, without added smoothing); scrolling otherwise still zooms.

Only viewport-owned presses start a drag. Panels and floating windows retain their
input. Release outside the canvas, Escape, tool switching, focus loss, zero-sized
viewports and resize terminate capture. A held button cannot restart an aborted
drag. Picking and inference do not run on drag moves; temporary navigation returns
to the original tool afterward. Pan activation cancels provisional work using the
existing switch path. Saved scenes change only through the existing Update action;
scene switching still animates. Panning records no model/selection/context/history
changes. The existing f32 viewport coordinate limits still apply.

The pan move itself allocates nothing and changes only camera transforms. Existing
UI composition and GPU frame submission still allocate their usual temporary data.
Mesh, instance, section and shadow caches retain their existing invalidation rules.
The release workload asserts no queued mesh changes, revision/history changes,
GPU allocation growth or shadow rerenders during 200 moves per shadow setting.
Its measurements include full UI, tessellation, GPU submission and completion wait
at 1280×800; startup, depth picking at press, and shader warmup are reported separately
or excluded from the move maximum. It skips explicitly without a hardware adapter;
failed headless file loading is reported and substitutes a labeled synthetic square.
The executable enforces a hard 120-second runtime deadline and fails the 33 ms
minimum move budget; under 16 ms is the preferred target.

```sh
WGPU_BACKEND=gl cargo run --release -p cindral-app --features kernel-benchmark --example pan_benchmark -- /path/to/private/model.skp
```

Measured on the private bedroom SKP (loaded headlessly), Radeon Vega 11 / Mesa GL:

| Shadows | Moves | Mean frame | Worst frame | Press frame |
| --- | ---: | ---: | ---: | ---: |
| Off | 200 | 2.231 ms | 3.766 ms | 2.568 ms |
| On | 200 | 3.261 ms | 5.155 ms | 4.062 ms |

Both runs meet the preferred 16 ms budget; these are local measurements, not a
cross-platform guarantee. No hardware touchpad or reference-app trial was performed.

## Questions for the user

1. [verify] In real SketchUp or SketchUp for Web, begin an unfinished Line, press H, pan, then choose Line again: is the unfinished line cancelled or resumed? Cindral cancels it.
2. [verify] In real SketchUp or SketchUp for Web, press H, start dragging and press Escape: does Pan stay active with an open hand? Cindral ends the drag and keeps Pan active.
