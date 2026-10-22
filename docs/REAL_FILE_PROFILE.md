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
