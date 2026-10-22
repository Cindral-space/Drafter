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
