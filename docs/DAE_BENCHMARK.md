# E1 COLLADA export measurements

Local Ryzen 5 2400G / Linux, release profile, three sequential samples per scene,
without concurrent builds or agents during measured execution:

| Scene | Mean export time | Range | DAE file size |
| --- | ---: | ---: | ---: |
| 5,000 shared box instances | 10.262 ms | 10.177–10.311 ms | 573,592 bytes |
| Connected solid, 21,600 quad faces | 493.424 ms | 488.732–502.781 ms | 4,744,584 bytes |

The box definition is stored once, with 5,000 placement nodes and shared
`instance_node` references; 60,000 triangles across placed occurrences. The large
connected solid uses shared positions and 21,600 convex polylists (43,200 implied
triangles). Default meter units, textures enabled (these scenes have no images),
hidden geometry excluded, triangulate-all off. No warmup/distribution claim.

Measured scope includes bundle validation/runtime reconstruction, geometry and
material preparation, XML construction, synchronized temporary-file writes and
atomic replacement. Fixture construction, native input loading, UI snapshot
cloning, receiving application import and display are excluded. These are CPU/IO
samples, not editor frame-rate or textured-export performance guarantees.

```sh
CARGO_TARGET_DIR=/home/earl/Desktop/Cindral/Drafter/target \
DAE_BENCH_OUTPUT=.agent-out/dae-validation \
cargo run --release -p cindral-io --features bench-fixtures --example dae_benchmark
scripts/validate-dae.sh .agent-out/dae-validation/*.dae
```

[Raw output](benchmarks/2026-10-22-e1-dae.txt). Both generated scenes passed installed
xmllint XML well-formedness checks with networking disabled. No offline COLLADA
1.4.1 XSD was found in project fixtures/docs or system XML catalogs, so schema
validation was explicitly skipped. No schema was downloaded to change that result.
Tests separately inspect unique IDs, resolved references, float-array counts,
primitive strides/counts, shared geometry, normals, UV bindings and image alpha.

Round trips through the I1 importer compare meter/inch placed bounds,
geometry-bearing occurrence counts, material colors/opacity/textures, annulus area
and expected imported face counts. Mandated triangulation changes an annulus from
one authored face to eight imported triangles; no claim of identical topology.
Native SketchUp and other external modeling application runtime validation has
not been performed. See [format options and limitations](DAE_EXPORT.md).
