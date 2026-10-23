//! Authored binary seeds under the repository license; no third-party models.
use cindral_core::*;
use cindral_io::*;
use std::{
    io::{Cursor, Write},
    time::{Duration, Instant},
};
fn tlv(tag: [u8; 2], body: &[u8]) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}
fn id(id: usize) -> Vec<u8> {
    tlv([0xde, 5], &(id as u32).to_le_bytes())
}
fn vertex(index: usize, p: [f64; 3]) -> Vec<u8> {
    tlv(
        [0xc4, 9],
        &[
            id(index),
            tlv(
                [0xc5, 9],
                &p.into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>(),
            ),
        ]
        .concat(),
    )
}
fn edge(index: usize, a: usize, b: usize) -> Vec<u8> {
    tlv(
        [0xb8, 11],
        &[
            id(index),
            tlv([0xb9, 11], &(a as u32).to_le_bytes()),
            tlv([0xba, 11], &(b as u32).to_le_bytes()),
        ]
        .concat(),
    )
}
fn ring(edges: &[(usize, bool)]) -> Vec<u8> {
    let body: Vec<u8> = edges
        .iter()
        .flat_map(|(e, rev)| {
            tlv(
                [0xa0, 15],
                &[
                    tlv([0xa1, 15], &(*e as u32).to_le_bytes()),
                    tlv([0xa2, 15], &[u8::from(*rev)]),
                ]
                .concat(),
            )
        })
        .collect();
    tlv([0x94, 17], &body)
}
fn face(index: usize, rings: &[Vec<u8>]) -> Vec<u8> {
    tlv(
        [0xac, 13],
        &[id(index), tlv([0xae, 13], &rings.concat())].concat(),
    )
}
fn wrap(model: &[u8]) -> Vec<u8> {
    wrap_files(model, &[])
}
fn wrap_files(model: &[u8], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut header = vec![255, 254, 255, 14, 0, 0, 0, 0, 0, 0, 255, 254, 255, 10];
    header.extend("{25.0.575}".encode_utf16().flat_map(u16::to_le_bytes));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "model.dat",
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .unwrap();
    zip.write_all(&tlv([0xf4, 1], model)).unwrap();
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    header.extend(zip.finish().unwrap().into_inner());
    header
}
fn hole_seed() -> Vec<u8> {
    let mut root = Vec::new();
    for (i, p) in [
        [0., 0., 0.],
        [10., 0., 0.],
        [10., 10., 0.],
        [0., 10., 0.],
        [3., 3., 0.],
        [3., 5., 0.],
        [5., 5., 0.],
        [5., 3., 0.],
    ]
    .into_iter()
    .enumerate()
    {
        root.extend(vertex(i + 1, p));
    }
    for (i, (a, b)) in [
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 1),
        (5, 6),
        (6, 7),
        (7, 8),
        (8, 5),
    ]
    .into_iter()
    .enumerate()
    {
        root.extend(edge(20 + i, a, b));
    }
    root.extend(face(
        50,
        &[
            ring(&[(20, false), (21, false), (22, false), (23, false)]),
            ring(&[(24, false), (25, false), (26, false), (27, false)]),
        ],
    ));
    // A corrupt face must not destroy the good annulus.
    root.extend(face(
        51,
        &[ring(&[(999, false), (998, false), (997, false)])],
    ));
    wrap(&tlv([0xf6, 1], &root))
}
#[test]
fn authored_polygon_hole_units_recovery_and_native_roundtrip() {
    let bytes = hole_seed();
    let scene = import_skp_bytes(&bytes, &ImportControl::default()).unwrap();
    assert_eq!(scene.meters_per_unit, 0.0254);
    assert!(scene.skipped >= 1);
    let result = convert_scene(
        scene,
        &ImportOptions {
            merge_coplanar: false,
            soften_coplanar: false,
            ..Default::default()
        },
        &ImportControl::default(),
    )
    .unwrap();
    assert_eq!(result.report.faces, 1);
    let occurrence = result
        .history
        .occurrences()
        .unwrap()
        .into_iter()
        .find(|o| {
            !result
                .history
                .definition_model(o.definition)
                .unwrap()
                .faces()
                .is_empty()
        })
        .unwrap();
    let model = result
        .history
        .definition_model(occurrence.definition)
        .unwrap();
    let (fid, f) = model.faces().iter().next().unwrap();
    assert_eq!(f.outer.len(), 4);
    assert_eq!(f.holes.len(), 1);
    assert!((model.face_area(*fid).unwrap() - 96. * 0.0254f64.powi(2)).abs() < 1e-10);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("imported.cdraft");
    let data = result.history.to_data();
    save_draft(
        &path,
        &DraftBundle {
            document: data.clone(),
            textures: result.textures,
            presentation: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(load_draft(&path).unwrap().document, data);
    if let Some(path) = std::env::var_os("CINDRAL_SKP_AUTHORED_SEED") {
        std::fs::write(path, bytes).unwrap();
    }
}
#[test]
fn malformed_sources_and_cancellation_do_not_panic() {
    let seed = hole_seed();
    for cut in (0..seed.len()).step_by(11) {
        let _ = import_skp_bytes(&seed[..cut], &ImportControl::default());
    }
    let control = ImportControl::default();
    control.cancel();
    assert!(import_skp_bytes(&seed, &control).is_err());
}
#[test]
fn user_local_models_skip_when_absent() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/skp-local");
    let files: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("skp"))
        })
        .collect();
    if files.is_empty() {
        eprintln!("SKIP: no user local .skp fixtures");
        return;
    }
    for file in files {
        import_file(
            &file.path(),
            &ImportOptions::default(),
            &ImportControl::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", file.path().display()));
    }
}
#[test]
#[ignore = "release-only connected 20k polygon import and local edit workload"]
#[allow(clippy::assertions_on_constants)]
fn connected_skp_import_performance() {
    assert!(!cfg!(debug_assertions));
    const N: usize = 140;
    let mut geometry = Vec::new();
    let v = |x: usize, y: usize| y * (N + 1) + x + 1;
    for y in 0..=N {
        for x in 0..=N {
            geometry.extend(vertex(v(x, y), [x as f64 * 10., y as f64 * 10., 0.]));
        }
    }
    let mut edges = std::collections::BTreeMap::new();
    let mut next = 100_000;
    for y in 0..N {
        for x in 0..N {
            let vertices = [v(x, y), v(x + 1, y), v(x + 1, y + 1), v(x, y + 1)];
            let mut boundary = Vec::new();
            for i in 0..4 {
                let (a, b) = (vertices[i], vertices[(i + 1) % 4]);
                let key = (a.min(b), a.max(b));
                let eid = *edges.entry(key).or_insert_with(|| {
                    let id = next;
                    next += 1;
                    geometry.extend(edge(id, key.0, key.1));
                    id
                });
                boundary.push((eid, a > b));
            }
            geometry.extend(face(200_000 + y * N + x, &[ring(&boundary)]));
        }
    }
    let bytes = wrap(&tlv([0xf6, 1], &geometry));
    if let Some(path) = std::env::var_os("CINDRAL_SKP_BENCHMARK_SEED") {
        std::fs::write(path, &bytes).unwrap();
    }
    let started = Instant::now();
    let control = ImportControl::default();
    let scene = import_skp_bytes(&bytes, &control).unwrap();
    let result = convert_scene(
        scene,
        &ImportOptions {
            merge_coplanar: false,
            soften_coplanar: false,
            ..Default::default()
        },
        &control,
    )
    .unwrap();
    let imported = started.elapsed();
    let mut destination = DocumentHistory::new();
    destination.append_import(result.history.to_data()).unwrap();
    let appended = started.elapsed();
    let occurrence = destination
        .occurrences()
        .unwrap()
        .into_iter()
        .find(|o| {
            !destination
                .definition_model(o.definition)
                .unwrap()
                .faces()
                .is_empty()
        })
        .unwrap();
    destination.enter(occurrence.path).unwrap();
    let started = Instant::now();
    destination
        .execute(Command::AddEdge {
            start: DVec3::new(0., 0.127, 0.),
            end: DVec3::new(0.127, 0., 0.),
        })
        .unwrap();
    let edit = started.elapsed();
    eprintln!("SKP connected {} quads, expanded {} bytes, compressed {} bytes: import {:.3}s; with append {:.3}s; kernel chord {:.3}ms",N*N,geometry.len(),bytes.len(),imported.as_secs_f64(),appended.as_secs_f64(),edit.as_secs_f64()*1000.);
    assert!(appended < Duration::from_secs(10));
    assert!(edit < Duration::from_millis(16));
    assert_eq!(destination.model().faces().len(), N * N + 1);
}

#[test]
fn face_tags_hidden_geometry_and_instance_paint_are_retained() {
    let mut geometry = Vec::new();
    for (i, p) in [[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]]
        .into_iter()
        .enumerate()
    {
        geometry.extend(vertex(i + 1, p));
    }
    for (i, (a, b)) in [(1, 2), (2, 3), (3, 4), (4, 1)].into_iter().enumerate() {
        geometry.extend(edge(i + 20, a, b));
    }
    let mut polygon = id(50);
    polygon.extend(tlv(
        [0xae, 13],
        &ring(&[(20, false), (21, false), (22, false), (23, false)]),
    ));
    polygon.extend(tlv(
        [0xd0, 7],
        &[
            tlv([0xd2, 7], &[9]),
            tlv([0xd3, 7], &[7]),
            tlv([0xd1, 7], &[0]),
        ]
        .concat(),
    ));
    polygon.extend(tlv([0xaf, 13], &[7]));
    geometry.extend(tlv([0xac, 13], &polygon));
    let definition = tlv([0x7c, 21], &[id(70), geometry].concat());
    let instance = tlv(
        [0x64, 25],
        &[
            tlv([0x67, 25], &[70]),
            tlv(
                [0xd0, 7],
                &[tlv([0xd1, 7], &[7]), tlv([0xd3, 7], &[7])].concat(),
            ),
        ]
        .concat(),
    );
    let layer = tlv(
        [0x99, 58],
        &tlv(
            [0x8c, 60],
            &[
                tlv([0xdc, 5], &id(9)),
                tlv([0x8d, 60], b"Walls"),
                tlv([0x8e, 60], &[0]),
            ]
            .concat(),
        ),
    );
    let material = tlv(
        [0xc8, 50],
        &[tlv([0xdc, 5], &id(7)), tlv([0xcc, 50], b"Tint")].concat(),
    );
    let model = [material, layer, definition, tlv([0xf6, 1], &instance)].concat();
    let xml=br#"<material name="Tint" colorRed="255" colorGreen="0" colorBlue="0" useTrans="1" trans="0.25"/>"#;
    let bytes = wrap_files(&model, &[("materials/Tint/material.xml", xml)]);
    let control = ImportControl::default();
    let scene = import_skp_bytes(&bytes, &control).unwrap();
    assert_eq!(scene.materials.len(), 1);
    assert!(scene
        .definitions
        .iter()
        .flat_map(|d| &d.geometry.faces)
        .all(|f| f.material == Some(MaterialId(1))));
    assert!((scene.materials[0].opacity - 0.75).abs() < 1e-6);
    assert!(scene
        .definitions
        .iter()
        .flat_map(|d| &d.instances)
        .any(|n| n.hidden && n.material == Some(MaterialId(1))));
    assert!(scene
        .definitions
        .iter()
        .flat_map(|d| &d.instances)
        .any(|n| n.hidden && n.tag == Some(0)));
    assert_eq!(
        scene
            .definitions
            .iter()
            .map(|d| d.geometry.faces.len())
            .sum::<usize>(),
        1
    );
    let result = convert_scene(scene, &ImportOptions::default(), &control).unwrap();
    assert_eq!(result.report.faces, 1);
    assert!(result.history.occurrences().unwrap().iter().all(|o| result
        .history
        .definition_model(o.definition)
        .unwrap()
        .faces()
        .is_empty()));
}

#[test]
fn authored_skp_opens_hole_face_at_root_and_edits_reversibly() {
    let scene = import_skp_bytes(&hole_seed(), &ImportControl::default()).unwrap();
    assert!(scene.root_definition.is_some());
    let mut history = convert_scene(
        scene,
        &ImportOptions {
            as_component: false,
            merge_coplanar: false,
            soften_coplanar: false,
        },
        &ImportControl::default(),
    )
    .unwrap()
    .history;
    assert!(history.context().is_empty());
    assert_eq!(history.model().faces().len(), 1);
    let face = *history.model().faces().keys().next().unwrap();
    assert_eq!(history.model().faces()[&face].holes.len(), 1);
    let before = history.model().to_data();
    assert!(history
        .execute(Command::PushPull {
            face,
            distance: 0.1
        })
        .unwrap());
    let after = history.model().to_data();
    assert!(history.undo().unwrap());
    assert_eq!(history.model().to_data().faces, before.faces);
    assert_eq!(history.model().to_data().edges, before.edges);
    assert_eq!(history.model().to_data().vertices, before.vertices);
    assert!(history.redo().unwrap());
    assert_eq!(history.model().to_data().faces, after.faces);
    history.model().validate().unwrap();
}

#[test]
fn skp_summary_separates_editable_notes_and_unsupported_features() {
    let scene = import_skp_bytes(&hole_seed(), &ImportControl::default()).unwrap();
    let result =
        convert_scene(scene, &ImportOptions::default(), &ImportControl::default()).unwrap();
    assert_eq!(result.report.warnings.len(), 1);
    assert!(result.report.warnings[0].contains("entity 51"));
    assert!(result.report.warnings[0].contains("unknown edge"));
    assert!(result
        .report
        .notes
        .iter()
        .any(|s| s.contains("editable") && s.contains("Save As")));
    assert!(result
        .report
        .notes
        .iter()
        .all(|s| !s.contains("Texture") && !s.contains("read-only")));
    assert_eq!(result.report.not_imported.len(), 1);
    for feature in [
        "Scenes/cameras",
        "styles",
        "dynamic behavior",
        "section planes",
        "text/dimensions",
        "UV pins",
    ] {
        assert!(result.report.not_imported[0].contains(feature));
    }
}
#[test]
fn individual_geometry_diagnostics_survive_conversion_without_truncation() {
    let mut scene = import_skp_bytes(&hole_seed(), &ImportControl::default()).unwrap();
    scene.warnings = (0..1005)
        .map(|i| format!("entity {i}: degenerate face skipped at Test/Root"))
        .collect();
    scene.skipped = 1005;
    let result =
        convert_scene(scene, &ImportOptions::default(), &ImportControl::default()).unwrap();
    assert_eq!(result.report.warnings.len(), 1005);
    assert!(result.report.warnings[1004].contains("entity 1004"));
}
