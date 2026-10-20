//! Local edit/history baseline and M4 closed-solid interactive previews.
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let setup = Instant::now();
    let mut f = cindral_core::benchmark::tiled_squares(100, 100)?;
    println!(
        "fixture: {} vertices / {} edges / {} faces; setup {:.3} ms",
        f.history.model().vertices().len(),
        f.history.model().edges().len(),
        f.history.model().faces().len(),
        setup.elapsed().as_secs_f64() * 1000.0
    );
    let edit = Instant::now();
    f.history.execute(f.edit)?;
    println!("edit_ms: {:.3}", edit.elapsed().as_secs_f64() * 1000.0);
    let undo = Instant::now();
    f.history.undo()?;
    println!("undo_ms: {:.3}", undo.elapsed().as_secs_f64() * 1000.0);
    let redo = Instant::now();
    f.history.redo()?;
    println!("redo_ms: {:.3}", redo.elapsed().as_secs_f64() * 1000.0);
    let mesh = Instant::now();
    let mut triangles = 0;
    for id in f.history.model().faces().keys() {
        triangles += f.history.model().triangulate(*id)?.len();
    }
    println!(
        "triangulate_all_ms: {:.3} ({triangles} triangles)",
        mesh.elapsed().as_secs_f64() * 1000.0
    );
    let mut fixture = cindral_core::benchmark::tiled_squares(100, 100)?;
    let face = cindral_core::benchmark::add_test_solid(&mut fixture.history)?;
    let depth = fixture.history.undo_depth();
    let mut samples = Vec::new();
    for i in 1..=30 {
        let start = Instant::now();
        fixture.history.preview(cindral_core::Command::PushPull {
            face,
            distance: 0.1 + i as f64 * 0.01,
        })?;
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    fixture.history.commit_preview()?;
    println!(
        "pushpull_preview_ms: mean {:.3}; max {:.3}; samples {}; undo_entries {}",
        samples.iter().sum::<f64>() / samples.len() as f64,
        samples.iter().copied().fold(0., f64::max),
        samples.len(),
        fixture.history.undo_depth() - depth
    );
    let vertices = fixture
        .history
        .model()
        .vertices()
        .iter()
        .filter(|(_, v)| v.position.x < -1. && v.position.y < -1.)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for (name, matrix, copy) in [
        (
            "move",
            glam::DMat4::from_translation(cindral_core::DVec3::new(0., 0., 0.1)),
            false,
        ),
        ("rotate", glam::DMat4::from_rotation_z(0.01), false),
        (
            "scale",
            glam::DMat4::from_scale(cindral_core::DVec3::splat(1.01)),
            false,
        ),
    ] {
        let start = Instant::now();
        let result = fixture.history.preview(cindral_core::Command::Transform {
            vertices: vertices.clone(),
            matrix,
            copy,
        });
        println!(
            "{name}_preview_ms: {:.3}; result {:?}",
            start.elapsed().as_secs_f64() * 1000.,
            result
        );
        fixture.history.cancel_preview()?;
    }
    Ok(())
}
