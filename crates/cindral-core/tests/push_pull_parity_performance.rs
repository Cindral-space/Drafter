//! CPU budget probe; excludes picking, inference, mesh preparation and GPU work.
#![cfg(feature = "bench-fixtures")]

use cindral_core::{benchmark, Command};
use std::time::{Duration, Instant};

#[test]
#[ignore = "release-only manual performance workload; not a complete live-preview measurement"]
#[allow(clippy::assertions_on_constants)] // Deliberate runtime guard for manual ignored-test runs.
fn push_pull_parity_20k_cpu_budget() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut fixture = benchmark::tiled_squares(100, 100).unwrap();
    let face = benchmark::add_test_solid(&mut fixture.history).unwrap();
    let depth = fixture.history.undo_depth();
    let mut worst = Duration::ZERO;
    for i in 1..=30 {
        let start = Instant::now();
        fixture
            .history
            .preview(Command::PushPull {
                face,
                distance: i as f64 * 0.01,
            })
            .unwrap();
        worst = worst.max(start.elapsed());
    }
    let start = Instant::now();
    assert!(fixture.history.commit_preview().unwrap());
    let commit = start.elapsed();
    println!(
        "20,006-face isolated-solid CPU preview max={:.3} ms; commit={:.3} ms; samples=30",
        worst.as_secs_f64() * 1000.,
        commit.as_secs_f64() * 1000.
    );
    assert_eq!(fixture.history.undo_depth(), depth + 1);
    assert!(worst < Duration::from_millis(16), "CPU preview {worst:?}");
    assert!(commit < Duration::from_millis(100), "CPU commit {commit:?}");
    // A second extrusion exercises merging with existing walls, rather than
    // only creating new walls on an initially subdivided supporting face.
    let mut worst = Duration::ZERO;
    for i in 1..=30 {
        let start = Instant::now();
        fixture
            .history
            .preview(Command::PushPull {
                face,
                distance: i as f64 * 0.01,
            })
            .unwrap();
        worst = worst.max(start.elapsed());
    }
    fixture.history.commit_preview().unwrap();
    println!(
        "20,006-face repeated-extrusion cleanup CPU preview max={:.3} ms; samples=30",
        worst.as_secs_f64() * 1000.
    );
    assert!(
        worst < Duration::from_millis(16),
        "CPU wall cleanup {worst:?}"
    );
}
