//! Dev-only counterpart of scripts/compare_skp.py; JSON on stdout.
use cindral_io::{convert_scene, import_skp, ImportControl, ImportOptions};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::env::args_os()
        .nth(1)
        .ok_or("usage: skp_inspect FILE.skp")?;
    let control = ImportControl::default();
    let scene = import_skp(std::path::Path::new(&file), &control)?;
    // The source model itself is represented by one synthetic root placement.
    let raw = cindral_skp::read(&std::fs::read(&file)?)?;
    let instances = raw
        .definitions
        .iter()
        .map(|d| d.instances.len())
        .sum::<usize>()
        + raw.roots.len().saturating_sub(1);
    let started = std::time::Instant::now();
    let result = convert_scene(
        scene,
        &ImportOptions {
            merge_coplanar: false,
            soften_coplanar: false,
            as_component: true,
        },
        &control,
    )?;
    let mut bounds: Option<(glam::DVec3, glam::DVec3)> = None;
    for occurrence in result.history.occurrences()? {
        if let Some(model) = result.history.definition_model(occurrence.definition) {
            for vertex in model.vertices().values() {
                let p = occurrence.world.transform_point3(vertex.position);
                bounds = Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))));
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "faces": result.report.faces, "instances": instances,
            "bounds": bounds.map(|(a,b)| [a.to_array(),b.to_array()]),
            "convert_seconds": started.elapsed().as_secs_f64(),
            "skipped": result.report.skipped, "warnings": result.report.warnings,
        })
    );
    Ok(())
}
