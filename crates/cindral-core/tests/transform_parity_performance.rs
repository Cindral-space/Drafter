//! Release CPU transaction subset; no picking, inference, mesh/upload or GPU.
#![cfg(feature = "bench-fixtures")]
use cindral_core::{benchmark, Command, DVec3};
use glam::DMat4;
use std::time::{Duration, Instant};

#[test]
#[ignore = "release-only CPU probe; full viewport performance remains unverified"]
#[allow(clippy::assertions_on_constants)]
fn transform_parity_20k_cpu_budget() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut results = Vec::new();
    for name in ["Move", "Copy", "Rotate", "Scale"] {
        let mut fixture = benchmark::tiled_squares(100, 100).unwrap();
        benchmark::add_test_solid(&mut fixture.history).unwrap();
        assert_eq!(fixture.history.model().faces().len(), 20_006);
        let vertices: Vec<_> = fixture
            .history
            .model()
            .vertices()
            .iter()
            .filter(|(_, v)| v.position.x < 0.)
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(vertices.len(), 8);
        let pivot = DVec3::new(-3., -3., 0.5);
        let depth = fixture.history.undo_depth();
        let mut worst = Duration::ZERO;
        for sample in 1..=30 {
            let amount = sample as f64 * 0.01;
            let matrix = match name {
                "Rotate" => {
                    DMat4::from_translation(pivot)
                        * DMat4::from_rotation_z(amount)
                        * DMat4::from_translation(-pivot)
                }
                "Scale" => {
                    DMat4::from_translation(pivot)
                        * DMat4::from_scale(DVec3::splat(1. + amount))
                        * DMat4::from_translation(-pivot)
                }
                _ => DMat4::from_translation(DVec3::Z * (3. + amount)),
            };
            let command = Command::Transform {
                vertices: vertices.clone(),
                matrix,
                copy: name == "Copy",
            };
            let start = Instant::now();
            fixture.history.preview(command).unwrap();
            worst = worst.max(start.elapsed());
        }
        let start = Instant::now();
        assert!(fixture.history.commit_preview().unwrap());
        let commit = start.elapsed();
        assert_eq!(fixture.history.undo_depth(), depth + 1);
        println!("{name}: 20,006 faces / 8 selected vertices / 30 previews; CPU max={:.6} ms; commit={:.6} ms", worst.as_secs_f64()*1000., commit.as_secs_f64()*1000.);
        results.push((name, worst, commit));
    }
    for (name, preview, commit) in results {
        assert!(
            preview < Duration::from_millis(16),
            "{name} CPU preview {preview:?}"
        );
        assert!(
            commit < Duration::from_millis(100),
            "{name} CPU commit {commit:?}"
        );
    }
}
