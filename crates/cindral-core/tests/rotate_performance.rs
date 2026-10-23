//! Synthetic release subset: entire 19,600-face definition, kernel previews only.
#![cfg(feature="bench-fixtures")]
use cindral_core::{benchmark,Command,DVec3};
use glam::DMat4;
use std::time::{Instant,Duration};
#[test]
#[ignore="release workload; run under timeout 60s"]
fn synthetic_19600_face_cached_rotate() {
    assert!(!cfg!(debug_assertions),"run --release");
    let mut fixture=benchmark::tiled_squares(98,100).unwrap();let h=&mut fixture.history;
    assert_eq!(h.model().faces().len(),19_600);let before=h.model().clone();let depth=h.undo_depth();
    let vertices:Vec<_>=h.model().vertices().keys().copied().collect();let mut worst=Duration::ZERO;let mut total=Duration::ZERO;
    for i in 1..=40 {
        let c=Command::Transform {vertices:vertices.clone(),matrix:DMat4::from_rotation_z(i as f64*0.003),copy:false};
        let start=Instant::now();h.preview(c).unwrap();let dt=start.elapsed();worst=worst.max(dt);total+=dt;
        assert_eq!(h.undo_depth(),depth);
    }
    let t=Instant::now();h.commit_preview().unwrap();let commit=t.elapsed();assert_eq!(h.undo_depth(),depth+1);
    println!("SYNTHETIC 19,600 faces, all {} vertices: kernel preview mean={:.3}ms max={:.3}ms commit={:.3}ms; excludes inference/UI/mesh/GPU",vertices.len(),total.as_secs_f64()*1000./40.,worst.as_secs_f64()*1000.,commit.as_secs_f64()*1000.);
    h.model().validate().unwrap();let rotated=h.model().clone();h.undo().unwrap();assert_eq!(h.model(),&before);h.redo().unwrap();assert_eq!(h.model(),&rotated);
    // Exercise ordinary indexed edits after the cached rotation, then both undo slots.
    h.execute(Command::AddEdge {start:DVec3::new(-10.,-10.,0.),end:DVec3::new(-9.,-10.,0.)}).unwrap();
    h.model().validate().unwrap();h.undo().unwrap();h.undo().unwrap();assert_eq!(h.model(),&before);h.model().validate().unwrap();
    assert!(worst<Duration::from_millis(33),"minimum kernel budget exceeded: {worst:?}");
}
