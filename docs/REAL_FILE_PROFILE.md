# mybedroom.skp interaction profile

2026-10-22, Ryzen 5 2400G, Vega 11, release, baseline main `e4bdb6d` plus instrumentation `df87893`. Recorded before changing any performance algorithm. Private source stays at `/home/earl/Desktop/Cindral/Drafter/SKP/mybedroom.skp`; never copied/staged. OpenCode disabled. No synthetic fallback.

## File inventory (importer output)

- 304 accepted faces, 2,021 edges, 1,760 vertices across unique root/component geometry. Reader reports 305 source faces; one corrupt/nonplanar face is skipped by the existing importer.
- Two source geometry definitions: Model (root: 303 V / 494 E / 207 F) and ThomThom (nested component: 1,457 V / 1,527 E / 97 F). One real instance, depth one after opening, no imported groups. The synthetic Model wrapper is promoted to root.
- One tag. Four embedded encoded texture records become three unique decoded textures, totaling 3,000,000 pixels.
- Three triangle faces; zero triangles share an edge with another coplanar triangle (normal/plane tolerance 1e-7). This measures remaining mergeable triangle adjacency, not all potentially overlapping/copolanar surface regions.
- Earlier synthetic case: 19,600 regular coplanar quads, 300 placements, no corresponding textured mixed/nonplanar connected geometry. Its fast extrusion proof does not establish performance here.

## Production-path baseline

Release replay imports through `import_file`, runs production import texture/scene preparation, then replays egui pointer press, 60 changing mouse moves and release through `viewport_input`, `organization_pointer`/`modify_viewport_input`, including inference. Loose Move selects the picked connected component using the production selection method; component Move selects the actual imported instance. Push/Pull starts from a visible root face. Hover covers 200 screen positions. Full frames draw all UI panels and tessellate egui output. GPU variant executes the same production Grid/Scene callbacks, draws egui to a 1280×800 offscreen target, and waits for GPU completion, on the actual Vega 11/GL/Mesa 26.2.3. No OS event-loop/presentation/vsync measurement. One sample sequence, not p95 guarantees. Summary dialog dismissed before replay; recovery/autosave isolated.

| Phase, ms | CPU-only mean / max | Completed GPU frame mean / max |
| --- | ---: | ---: |
| Hover frame | 0.493 / 0.925 | 2.908 / 5.976 |
| Component Move begin | 0.943 | 3.280 |
| Component Move previews | 0.485 / 0.777 | 3.119 / 10.345 |
| Component release/commit frame | 0.435 | 3.218 |
| Loose connected Move begin | 7.013 | 12.339 |
| Loose Move previews | 15.441 / 18.514 | 17.240 / 20.216 |
| Loose release/commit frame | 14.119 | 16.073 |
| Push/Pull begin | 212.002 | 231.440 |
| Push/Pull previews | 215.250 / 237.024 | 222.551 / 242.207 |
| Push/Pull release/commit frame | 0.480 | 2.846 |
| First full UI frame | 1.686 | 31.663 |
| Import + textures + initial CPU meshes | 210.134 | 204.782 |

CPU-only GPU zeros mean **not measured**, never zero-cost rendering. GPU full-frame hover exceeds 5 ms, while the pick phase itself is under 1 ms. Startup import is outside interaction budgets. All phase breakdowns: [CPU](../benchmarks/2026-10-22-real-before.txt), [GPU](../benchmarks/2026-10-22-real-gpu-before.txt). Phase scopes are inclusive: app edit includes dirty mesh work in this instrumentation baseline; do not add nested phases. GPU upload and shadow scopes measure CPU work; completion is measured separately.

Push/Pull CPU phase mean/max: edit **214.312/235.968 ms**, mesh **0.283/0.564**, pick **0.070/0.133**, inference **0.106/0.119**, tool update **0.001/0.003**, UI **0.180/0.281**, bounds **0.018/0.073**. Actual GPU-run upload **0.190/0.756**, shadow configuration **0.016/0.071**, complete render submission/wait **2.515/5.232**. Loose Move edit **13.933/16.911**, meshes **1.303/1.811**.

## CPU sampling and suspects

Locally extracted Arch perf 7.2.11, user cycles at 199 Hz, DWARF stack capture; no system installation. Entire CPU-only replay: 2,899 captured samples, no lost samples. Flat profile retained because this optimized build did not yield useful parent call chains. Dominant self samples: segment-cut sorting **27.39%**, adjacency-map lookup **19.34%**, exact segment intersections **15.91%**, segment insertion **5.09%**, vertex interning **4.83%**. [Flat report](../benchmarks/2026-10-22-real-perf-before.txt). The sampled run is separate from timing tables.

- a: inference still scans root vertices/edges, but measured below 0.3 ms here; no evidence it causes this stall.
- b: component Move already calls `preview_instance_transform`, retaining definition meshes. Measured <0.02 ms edit/mesh in CPU replay, no geometry definition cloning.
- c: hover uses indexed face occlusion plus vertex/edge projection scans. Picking and highlight are not the measured bottleneck on this file.
- d: Outliner already caches rows and uses `ScrollArea::show_rows`. Only one real instance here; UI below 0.6 ms. No virtualization feature needed.
- e: shadows disabled. Configuration checks execute, but no depth-map render should occur; callback counters will be asserted in regression.
- f: component preview clones only placement metadata; loose geometry instead prepares/validates a transaction cavity. Bounds are small here.
- g: general Push/Pull reconstructs surfaces from polygons. `surfaces` creates an unindexed temporary Model; each inserted polygon segment scans/sorts/intersects all existing temporary edges and linearly interns vertices, despite an existing indexed construction path. CPU sampling directly identifies this work. Rebuild/validation remain synchronous.
- h: previews prepare a local delta and validate its patch each update; no undo entry until commit. Loose Move's ~14 ms edit cost needs deeper spans before changing validation semantics.
- i: two geometry definitions, not a single flattened chunk. Push/Pull expands its transaction to connected surfaces by design and falls back to general reconstruction for this mixed geometry; the regular-sheet local proof does not apply.

## Reproduction

```
CARGO_TARGET_DIR=/home/earl/Desktop/Cindral/Drafter/target cargo build --release -p cindral-app --features kernel-benchmark --example real_file_benchmark
CINDRAL_PROFILE=1 timeout 120s target/release/examples/real_file_benchmark /private/mybedroom.skp
CINDRAL_PROFILE=1 CINDRAL_BENCH_GPU=1 WGPU_BACKEND=gl timeout 120s target/release/examples/real_file_benchmark /private/mybedroom.skp
perf record -e cycles:u -F 199 --call-graph dwarf,16384 -- target/release/examples/real_file_benchmark /private/mybedroom.skp
```

Next bounded optimization: enable the already-maintained construction spatial index for temporary surface segment insertion, keeping exact predicates, stable ID ordering, and geometry semantics. No threads, import cleanup changes or generalized Boolean redesign. Re-measure before deciding whether remaining targets need redesign.

## Final verification and remaining limit

The change enables the existing temporary construction R-tree, removes exact duplicate polygon boundaries, skips empty intersection sorting, caches healing/reconstruction adjacency, and indexes vertices for wire-ring expansion. Exact geometry predicates, tolerance, stable ID ordering, importer, inference semantics, undo boundary and placement-only instance Move are retained. No threads or feature work.

The full UI GPU replay uses one sample per pixel (native app requests four), offscreen RGBA8 output and completed queue waits. This is production input/UI/scene callback execution, not a native presented-frame guarantee. Cold import and adapter creation remain setup.

Before/after completed Vega 11 frames, **mean / max milliseconds**, 60 previews per tool and 200 hover positions. CPU-only final phase evidence is in `benchmarks/2026-10-22-real-cpu-final.txt`; GPU evidence in `benchmarks/2026-10-22-real-gpu-final.txt`. Earlier intermediate runs are retained; timings vary between runs.

| Additional phase | Before ms | After ms |
| --- | ---: | ---: |
| import + initial CPU mesh + textures | 204.782 | 185.100 |
| first full UI frame incl tessellation | 31.663 | 35.251 |
| component Move begin | 3.280 | 3.379 |
| component Move release/commit frame | 3.218 | 2.572 |
| loose Move begin | 12.339 | 11.395 |
| loose Move release/commit frame | 16.073 | 16.344 |
| PushPull begin | 231.440 | 34.906 |
| PushPull release/commit frame | 2.846 | 3.174 |

### hover full frame

| Phase | Before mean / max ms | After mean / max ms |
| --- | ---: | ---: |
| frame | 2.908 / 5.976 | 2.987 / 5.140 |
| pick | 0.162 / 0.339 | 0.181 / 0.367 |
| inference/snap | 0.221 / 0.523 | 0.246 / 0.767 |
| tool update | 0.000 / 0.000 | 0.000 / 0.000 |
| kernel edit | 0.000 / 0.000 | 0.000 / 0.000 |
| mesh rebuild | 0.000 / 0.000 | 0.000 / 0.000 |
| GPU upload (CPU) | 0.000 / 0.000 | 0.000 / 0.000 |
| shadow pass (CPU) | 0.021 / 0.049 | 0.023 / 0.061 |
| UI/outliner | 0.263 / 0.551 | 0.281 / 0.561 |
| bounds/sections | 0.001 / 0.002 | 0.000 / 0.001 |
| render + GPU completion | 2.077 / 4.377 | 2.089 / 3.788 |

### component Move preview full frame

| Phase | Before mean / max ms | After mean / max ms |
| --- | ---: | ---: |
| frame | 3.119 / 10.345 | 2.943 / 8.918 |
| pick | 0.000 / 0.000 | 0.000 / 0.000 |
| inference/snap | 0.249 / 0.512 | 0.243 / 0.493 |
| tool update | 0.000 / 0.000 | 0.000 / 0.000 |
| kernel edit | 0.014 / 0.040 | 0.013 / 0.032 |
| mesh rebuild | 0.010 / 0.033 | 0.009 / 0.024 |
| GPU upload (CPU) | 0.015 / 0.050 | 0.014 / 0.027 |
| shadow pass (CPU) | 0.020 / 0.034 | 0.020 / 0.038 |
| UI/outliner | 0.301 / 0.547 | 0.283 / 0.504 |
| bounds/sections | 0.026 / 0.059 | 0.026 / 0.081 |
| render + GPU completion | 2.305 / 9.137 | 2.162 / 8.081 |

### loose Move preview full frame

| Phase | Before mean / max ms | After mean / max ms |
| --- | ---: | ---: |
| frame | 17.240 / 20.216 | 18.502 / 22.094 |
| pick | 0.117 / 0.165 | 0.131 / 0.207 |
| inference/snap | 0.180 / 0.284 | 0.195 / 0.332 |
| tool update | 0.003 / 0.006 | 0.003 / 0.018 |
| kernel edit | 12.560 / 14.818 | 13.630 / 16.302 |
| mesh rebuild | 1.175 / 1.578 | 1.252 / 1.677 |
| GPU upload (CPU) | 0.383 / 0.668 | 0.420 / 0.676 |
| shadow pass (CPU) | 0.017 / 0.031 | 0.019 / 0.047 |
| UI/outliner | 0.377 / 0.575 | 0.404 / 0.685 |
| bounds/sections | 0.019 / 0.040 | 0.023 / 0.070 |
| render + GPU completion | 3.212 / 7.415 | 3.292 / 9.405 |

### PushPull preview full frame

| Phase | Before mean / max ms | After mean / max ms |
| --- | ---: | ---: |
| frame | 222.551 / 242.207 | 30.827 / 40.820 |
| pick | 0.085 / 0.182 | 0.087 / 0.225 |
| inference/snap | 0.118 / 0.278 | 0.117 / 0.331 |
| tool update | 0.001 / 0.003 | 0.002 / 0.011 |
| kernel edit | 218.986 / 238.513 | 27.721 / 36.653 |
| mesh rebuild | 0.294 / 0.541 | 0.298 / 0.409 |
| GPU upload (CPU) | 0.190 / 0.756 | 0.193 / 0.835 |
| shadow pass (CPU) | 0.016 / 0.071 | 0.016 / 0.035 |
| UI/outliner | 0.227 / 0.458 | 0.227 / 0.464 |
| bounds/sections | 0.018 / 0.073 | 0.019 / 0.046 |
| render + GPU completion | 2.515 / 5.232 | 2.432 / 3.182 |

Picking satisfies <5 ms; completed hover **frames** include GPU/UI and may exceed 5 ms. Matrix-only component previews and commits satisfy the requested limits. Loose Move satisfies the 33 ms minimum, while missing the preferred 16 ms. Push/Pull is substantially faster but still exceeds 33 ms in some measured frames: **the freeze is not declared fixed**. All sampled post-change interactive frames stay below 100 ms. The strict 33 ms regression must expose this failure rather than silently passing.

No broader redesign was started. Options for the remaining target: (1) incremental general-surface extrusion, retaining untouched authored surfaces and rebuilding only a proven edit cavity; (2) retain validated rigid-selection transaction structure across changing Move offsets, rechecking only potential new intersections. Both need separate geometry-equivalence work; moving current reconstruction to a worker would hide the remaining algorithmic cost and is not proposed as the first fix.

`CINDRAL_PROFILE=1` enables cached, opt-in host phase timers. Disabled calls read no clocks and allocate nothing. Slow UI passes and host frames through the scene callback print phase breakdowns; expensive upload/shadow host callbacks also log independently. The normal app profiler does not measure GPU execution or vsync; the regression explicitly waits for GPU completion. Phases are inclusive.

Run `CINDRAL_REAL_FILE=/private/mybedroom.skp WGPU_BACKEND=gl scripts/check-real-file-performance.sh`. The script skips if the private file is missing, never substitutes synthetic geometry, enforces 5 ms pick / 33 ms preview / 100 ms frame limits, and has a 120-second replay timeout. It currently identifies the remaining Push/Pull target failure. The optional release test accepts the same private-file environment variable and skips without it or in debug builds.

Validation: targeted indexed/exhaustive exact reconstruction test passes, strict workspace/all-target/all-feature Clippy passes after two instrumentation lint repairs, and the full all-feature workspace suite passes **344 tests, zero failures, seven ignored workloads**. Formatting applied once; two later lint-driven item-order/blank-line adjustments match formatter output. No external model was copied or committed.

Final strict budget run: Push/Pull max **39.892 ms**, exit **101**, as expected for the unmet 33 ms target. The final unguarded GPU sequence reports max **40.820 ms**; an earlier identical-algorithm run reported **36.688 ms**. Both are retained. No post-change interactive frame exceeds 100 ms.
