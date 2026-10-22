//! Opt-in CPU subset; excludes inference, egui, highlight generation and GPU.
use cindral_core::{BulkFace, BulkGeometry, DVec3};
use cindral_inference::{Ray, View};
use cindral_tools::{pick, Hit, Selection};
use glam::{DMat4, DVec2};
use std::time::{Duration, Instant};

#[test]
#[ignore = "release CPU pick exceeds 16 ms on 19,600 faces; excludes full viewport work"]
#[allow(clippy::assertions_on_constants)]
fn select_parity_20k_cpu_budget() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut geometry = BulkGeometry::default();
    for y in 0..140 {
        for x in 0..140 {
            let i = geometry.vertices.len();
            let p = DVec3::new(x as f64, y as f64, 0.);
            geometry.vertices.extend([
                p,
                p + DVec3::X * 0.8,
                p + (DVec3::X + DVec3::Y) * 0.8,
                p + DVec3::Y * 0.8,
            ]);
            geometry.faces.push(BulkFace {
                outer: vec![i, i + 1, i + 2, i + 3],
                ..Default::default()
            });
        }
    }
    let model = geometry.build(false, false).unwrap();
    assert_eq!(model.faces().len(), 19_600);
    let view = View {
        matrix: DMat4::orthographic_rh(-75., 75., -75., 75., 0.1, 100.)
            * DMat4::look_at_rh(
                DVec3::new(70., 70., 10.),
                DVec3::new(70., 70., 0.),
                DVec3::Y,
            ),
        size: DVec2::new(1280., 800.),
    };
    let target = DVec3::new(70.4, 70.4, 0.);
    let ray = Ray {
        origin: target + DVec3::Z * 10.,
        direction: -DVec3::Z,
    };
    let cursor = view.project(target).unwrap();
    let mut worst = Duration::ZERO;
    for _ in 0..30 {
        let start = Instant::now();
        let hit = pick(&model, ray, view, cursor, 1.);
        worst = worst.max(start.elapsed());
        assert!(matches!(hit, Some(Hit::Face(_))));
    }
    let mut selection = Selection::default();
    let start = Instant::now();
    selection.region(&model, view, DVec2::ZERO, view.size, false);
    let release = start.elapsed();
    assert_eq!(selection.faces.len(), 19_600);
    let start = Instant::now();
    selection.connected(
        &model,
        Hit::Face(*model.faces().keys().next().unwrap()),
        false,
    );
    let connected = start.elapsed();
    assert_eq!(selection.faces.len(), 1);
    println!("19,600 disconnected faces / 30 picks: CPU pick max={:.3} ms; box release={:.3} ms; connected release={:.3} ms; excludes inference/UI/GPU", worst.as_secs_f64()*1000., release.as_secs_f64()*1000., connected.as_secs_f64()*1000.);
    assert!(worst < Duration::from_millis(16), "CPU pick {worst:?}");
    assert!(
        release < Duration::from_millis(100),
        "CPU box release {release:?}"
    );
    assert!(
        connected < Duration::from_millis(100),
        "CPU connected release {connected:?}"
    );
}
