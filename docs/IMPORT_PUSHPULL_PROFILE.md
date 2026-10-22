# Imported Push/Pull drag profile

2026-10-22, Ryzen 5 2400G (4C/8T), Vega 11, Linux, Rust release. Investigation
baseline `1291bb5`, based on main `a6eef20`. This section was recorded and committed
**before changing any production algorithm**. OpenCode disabled.

No local files exist in `tests/fixtures/skp-local/`. The authored ImportedScene uses
the same converter/cleanup settings as SKP (no polygon cleanup), a welded 64×64 quad
sheet and 300 placements of one shared definition. The equivalent drawn sheet uses
Rectangle Tool commands, preserving grid subdivisions. Import and drawing setup,
initial CPU/GPU caches, undo/redo verification and full diagnostic validation are
outside the timed drag. Each drag uses begin, 60 changing absolute previews, commit.
Picking and inference run on the current preview as in the app; the top view is
zoomed out with multiple boundaries inside the 10-pixel pick radius. Real mouse
routing/presentation/OS scheduling are not included. No user-model reproduction claim.

## Baseline before optimization

| Phase, mean / max ms over 60 updates | Imported 4,096 faces | Drawn 4,096 faces | Imported 19,600 faces |
| --- | ---: | ---: | ---: |
| Face/edge/vertex pick | 14.207 / 35.394 | 15.320 / 39.070 | 77.953 / 760.384 |
| Inference | 0.207 / 0.325 | 0.209 / 0.479 | 0.984 / 1.581 |
| Tool update + history preview | 0.516 / 1.009 | 0.515 / 1.327 | 1.793 / 2.769 |
| Dirty CPU scene mesh | 0.065 / 0.707 | 0.050 / 0.105 | 0.061 / 0.113 |
| Entire update max | **36.950** | **39.834** | **763.673** |
| Begin | 0.008 | 0.009 | 0.008 |
| Commit | 0.001 | 0.054 | 0.001 |

Headless runs do not invoke a GPU: the printed zero callback time is not a GPU
measurement. Completed adapter upload measurements are recorded separately below.
Sequential runs, no parallel benchmark/agent/Cargo work during these measurements.
Earlier exploratory overlapping runs were discarded. Raw timings:
[4,096 faces](benchmarks/2026-10-22-import-pushpull-before.txt),
[19,600 faces](benchmarks/2026-10-22-import-pushpull-large-before.txt).

Tracing (`CINDRAL_PROFILE=1`, span close/busy timing) on a separate 4,096-face run
shows 959.900 ms total inclusive pick time across 60 updates. Vertex occlusion costs
369.880 ms / 244 calls, edge occlusion 370.650 ms / 255 calls: **77% of pick time**.
Each such call scans all faces and projects polygon rings. History previews total
38.933 ms, including local proof 24.378 ms, planar proof 15.049 ms, delta apply
9.043 ms (including undo/redo), CPU dirty meshes 3.885 ms. Inclusive spans overlap;
do not add parent/child totals. Trace output overhead makes these diagnostic timings
different from the untraced table. [Aggregated spans](benchmarks/2026-10-22-import-pushpull-spans-before.txt).

## Suspects checked

- **a, full dirty chunk/upload:** disproved for this fixture. Exactly one definition
  changes, no instance batches change, dirty updates have `reset=false`, first
  update writes just 2,448 bytes at both sizes. CPU mesh work is below 1 ms.
- **b, missing derived state:** bulk construction builds spatial and adjacency
  caches. The same geometry drawn through tools has identical V/E/F counts and
  essentially the same costs. Those existing spatial indices are simply unused by
  picking's ray/occlusion scans.
- **c, pairwise cleanup/healing:** not executed during these local previews.
  Import cleanup is disabled for authored SKP polygons. General-path split, seam,
  heal and reconstruction spans exist but do not fire in this drag.
- **d, full reconstruction per move:** not observed here. The local proof scans
  whole-model holes, incidence and vertices, costing about 0.5–1.8 ms including
  history. The dominant whole-model scan is instead picking: every screen-near
  vertex/edge performs a face-wide occlusion query, in addition to the initial
  ray face scan. Overview density amplifies candidate count.
- **e, undo:** entity deltas are local; no snapshot, commit is negligible.
- **f, inflated import topology:** disproved on the equivalent fixture: imported
  and drawn both have 4,225 vertices / 8,320 edges / 4,096 faces. Large scene has
  19,881 / 39,480 / 19,600. No cleanup/tolerance change is justified by this evidence.

The smallest fix is to use the existing face R-tree for conservative forward-ray
candidate rejection, including each candidate's own occlusion ray, retaining exact
polygon/hole/section tests and stable tie ordering. No pick/snapping semantics change.
A global scan remains the fallback for unindexed models. Inference can reuse the
same query for face candidates. Arbitrary nonplanar Boolean fallbacks are a separate
known limitation; this investigation does not establish general Boolean latency.

## Reproduction and regression

```sh
CARGO_TARGET_DIR=/path/to/reused/target scripts/check-import-pushpull-performance.sh
CINDRAL_IMPORT_GRID=140 CINDRAL_IMPORT_CASE=imported CINDRAL_ENFORCE_BUDGET=1 \
  timeout 90s target/release/examples/import_pushpull_benchmark
CINDRAL_PROFILE=1 CINDRAL_IMPORT_CASE=imported \
  timeout 90s target/release/examples/import_pushpull_benchmark > spans.txt
```

Runner builds release, then uses a **90-second hard process timeout per case**
including setup; asserts each full preview <16 ms, begin <250 ms, commit <100 ms,
exact geometry undo/redo, partial meshes and unchanged instance batches. Baseline
fails the preview assertion. Absolute budgets assume an otherwise idle machine.
`CINDRAL_IMPORT_CASE=local` plus `CINDRAL_SKP_BENCHMARK_SEED=/private/model.skp`
optionally uses an actual file opened at root; nested editing contexts need a custom
selection. No local model is copied/committed. GPU example uses the same workload
and waits for queue completion; software adapters must be identified separately.

Pre-optimization completed GPU upload confirmation (same fixture, actual Vega 11,
OpenGL/Mesa 26.2.3): upload mean/max **0.605/4.003 ms**, entire update max
**59.865 ms**, pick mean/max **13.699/54.848 ms**. Upload remains secondary;
its first-update maximum includes buffer growth. No full presented-frame claim.
[Adapter/raw output](benchmarks/2026-10-22-import-pushpull-gpu-before.txt).

## Optimized result

The existing R-tree now prunes both parent and leaf envelopes using a conservative
ray/slab test, including tolerance/roundoff padding and the maintained translation
frame. Candidate IDs remain ordered. Picking's initial face query and each
screen-near candidate's own occlusion ray use it; inference's face query shares it.
The exact polygon/hole/section tests, pick priorities, kernel algorithms, import
cleanup, undo semantics and dirty mesh path are unchanged.

| Workload | Before full preview max ms | After full preview max ms | After commit ms |
| --- | ---: | ---: | ---: |
| Imported 4,096 faces, CPU | 36.950 | **3.085** | 0.001 |
| Drawn equivalent 4,096 faces, CPU | 39.834 | **2.654** | 0.049 |
| Imported 19,600 faces, CPU | 763.673 | **12.635** | 0.001 |
| Imported 4,096 faces, completed Vega 11 upload | 59.865 | **4.114** | 0.001 |
| Imported 19,600 faces, completed Vega 11 upload | Not sampled | **12.413** | 0.001 |

At 19,600 faces the CPU-only pick mean/max is now **7.979/8.910 ms**,
inference **0.938/1.219 ms**, kernel **1.708/2.492 ms**, dirty CPU mesh
**0.055/0.073 ms**. GPU run completed-upload mean/max **0.606/0.954 ms**;
begin **0.016 ms**. All 60 updates, cold included, pass <16 ms; commit passes
<100 ms and no measured interactive phase/update exceeds 250 ms. Thus no worker
or progress UI is needed for the measured operation. No import quality changes:
before/after source V/E/F counts are identical. [CPU results](benchmarks/2026-10-22-import-pushpull-after.txt),
[GPU results and adapter](benchmarks/2026-10-22-import-pushpull-gpu-after.txt).

Correctness compares indexed versus retained exhaustive picking for vertices,
edges, faces and misses with holes, foreground occlusion, sections, perspective
and orthographic views, boundary tolerances, preview/cancel, raw translation,
commit and undo/redo. A 60-preview imported/drawn cube regression compares exact
face boundary coordinates after every +/- update and checks single-step history,
undo/redo and cancellation. Existing kernel/local-extrusion equivalence tests
continue to cover the geometry algorithms; those algorithms were not modified.

```sh
cargo test -p cindral-core ray_tests
cargo test -p cindral-tools indexed_pick_equivalence
cargo test -p cindral-io --test pushpull_import
WGPU_BACKEND=gl CINDRAL_BENCH_GPU=1 scripts/check-import-pushpull-performance.sh
```

The benchmark examples are behind the existing `kernel-benchmark` feature. GPU
runs use a display-free real adapter/device, actual SceneRenderer updates and queue
completion. They exclude raster/presentation, UI layout and input dispatch. Timings
are one sequence each, not p95/editor FPS guarantees. `bacon run` used an unoptimized
debug executable in the earlier session; use `cargo run --release -p cindral-app`
(or configure bacon's job for release) when comparing these budgets.

Remaining vertex/edge projection scans scale linearly with model size. Very dense
screen regions can still produce many ray queries; arbitrary nonplanar/holed
Boolean fallbacks retain their prior unbounded synchronous behavior. No actual
user SKP was available, so this does not prove every reported file is fixed. If
such a file still stalls, capture these spans on that exact operation; a bounded
screen-query index or cancellable worker for a measured general-Boolean fallback
would be separate options. No redesign was started, and no blanket event-loop
latency guarantee is made beyond the measured fixtures.

Final completion validation: formatting, strict workspace/all-target/all-feature
Clippy, and the full all-feature workspace tests each ran once and passed:
**342 tests including the doctest**, seven ignored release workloads. The complete
runner was then executed against final formatted sources with GPU enabled; every
case passed its hard deadline/budgets. Final maxima: imported CPU 4,096 / 19,600
faces **3.426 / 14.712 ms**, drawn CPU **2.824 ms**, completed Vega 11 upload cases
**5.248 / 14.486 ms**, imported commit **0.001 ms**. Variation from the first after
samples above is retained rather than replacing evidence. These measurements are
sequential, without concurrent Cargo/agent/benchmark work. [Final runner output](benchmarks/2026-10-22-import-pushpull-final-regression.txt).
