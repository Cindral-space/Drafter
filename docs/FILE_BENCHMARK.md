# M6 native save/load measurements

Measured 2026-10-21 on Linux, Rust 1.98.1 release, AMD Ryzen 5 2400G (8 logical CPUs), the same Vega 11 workstation as the connected-solid reports. No concurrent Cargo builds or agent tasks during the final measurements.

```sh
cargo run --release -p cindral-io --features bench-fixtures --example file_benchmark
```

Three repeated saves and loads per fixture, using a temporary directory and warm filesystem caches. Every load is compared with the complete original `DraftBundle`. These are native archive measurements: save includes full document validation/runtime index construction, compact RON serialization, ZIP deflate level3 and durable atomic replacement; load includes ZIP/RON decoding, migration/schema checks and full runtime validation. They exclude drawing tools, UI, GPU mesh construction, and the app's additional reconstruction after taking ownership of loaded DTOs. The fixtures have no embedded images; textured exact round trips and image validation are tested separately.

The connected fixtures are welded cube boundaries: 21,600 faces/43,200 edges and 60,000 faces/120,000 edges. The instance fixture has 5,000 placements sharing one six-face box definition; file values store geometry once. No history, navigation or GPU caches are stored.

| Fixture | Mean save ms (range) | Mean load ms (range) | ZIP bytes |
| --- | ---: | ---: | ---: |
| connected-21600 | 412.748 (383.263–465.114) | 536.676 (526.491–555.585) | 720,272 |
| connected-60000 | 1079.619 (1039.526–1145.459) | 1482.567 (1466.720–1512.521) | 1,795,954 |
| instances-5000 | 14.457 (14.215–14.914) | 39.187 (38.760–39.708) | 47,897 |

[Raw final output](benchmarks/2026-10-21-m6-files.txt). An earlier run overlapped dependency compilation and is excluded. The 2.78× face-count increase costs 2.62× mean save and 2.76× mean load here, approximately linear. Shared-instance file size is 47,897 bytes because geometry is stored once. These IO costs are intentionally outside the interactive edit path; large snapshots compress/write/validate on the autosave worker after a UI-thread DTO capture. Explicit save/open currently block while IO completes.
