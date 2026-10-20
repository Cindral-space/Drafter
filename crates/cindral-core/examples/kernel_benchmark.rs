//! Global topology and history baseline; fixture setup is excluded from timings.
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
    Ok(())
}
