//! End-to-end release glTF import workload; explicitly ignored in normal CI.
use base64::{engine::general_purpose::STANDARD, Engine};
use cindral_core::*;
use cindral_io::*;
use std::time::{Duration, Instant};
#[test]
#[ignore = "run --release --ignored --nocapture for 100k triangle benchmark"]
#[allow(clippy::assertions_on_constants)]
fn gltf_100k_triangles() {
    assert!(!cfg!(debug_assertions), "benchmark requires release");
    let mut bytes = Vec::new();
    let mut index = Vec::new();
    for y in 0..200 {
        for x in 0..250 {
            let base = (y * 250 + x) * 4;
            // Source Y up, converted into the native XY sheet.
            for p in [
                [x as f32, 0., -(y as f32)],
                [(x + 1) as f32, 0., -(y as f32)],
                [(x + 1) as f32, 0., -((y + 1) as f32)],
                [x as f32, 0., -((y + 1) as f32)],
            ] {
                for v in p {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
            for i in [base, base + 1, base + 2, base, base + 2, base + 3] {
                index.extend_from_slice(&(i as u32).to_le_bytes());
            }
        }
    }
    let offset = bytes.len();
    bytes.extend(index);
    let json=serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"buffers":[{"byteLength":bytes.len(),"uri":format!("data:application/octet-stream;base64,{}",STANDARD.encode(&bytes))}],"bufferViews":[{"buffer":0,"byteLength":offset},{"buffer":0,"byteOffset":offset,"byteLength":bytes.len()-offset}],"accessors":[{"bufferView":0,"componentType":5126,"count":200000,"type":"VEC3"},{"bufferView":1,"componentType":5125,"count":300000,"type":"SCALAR"}]}).to_string();
    let start = Instant::now();
    let control = ImportControl::default();
    let scene = import_gltf_bytes(json.as_bytes(), None, &control).unwrap();
    let r = convert_scene(scene, &ImportOptions::default(), &control).unwrap();
    let parsed = start.elapsed();
    let mut destination = DocumentHistory::new();
    destination.append_import(r.history.to_data()).unwrap();
    let appended = start.elapsed();
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
    let faces = destination.model().faces().len();
    assert!((50000..=100000).contains(&faces));
    let edit = Instant::now();
    destination
        .execute(Command::AddEdge {
            start: DVec3::new(0., 0.5, 0.),
            end: DVec3::new(0.5, 0., 0.),
        })
        .unwrap();
    let edit = edit.elapsed();
    eprintln!("glTF 100k triangles: parse+convert {:.3}s; with append {:.3}s; connected edit {:.3}ms; faces={faces}",parsed.as_secs_f64(),appended.as_secs_f64(),edit.as_secs_f64()*1000.);
    assert!(
        appended < Duration::from_secs(10),
        "import budget exceeded {appended:?}"
    );
    assert!(
        edit < Duration::from_millis(16),
        "interactive edit budget exceeded {edit:?}"
    );
    assert_eq!(destination.undo_depth(), 2);
}
