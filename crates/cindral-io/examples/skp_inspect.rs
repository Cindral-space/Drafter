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
    eprintln!(
        "source definitions: {:?}; tags={}; encoded textures={}",
        raw.definitions
            .iter()
            .map(|d| (
                &d.name,
                d.geometry.vertices.len(),
                d.geometry.edges.len(),
                d.geometry.faces.len(),
                d.instances.len()
            ))
            .collect::<Vec<_>>(),
        raw.tags.len(),
        raw.textures.len()
    );
    let pixels: usize = scene
        .textures
        .values()
        .map(|t| cindral_io::decode_texture(t).map(|d| d.width as usize * d.height as usize))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sum();
    eprintln!(
        "decoded texture count={} total_pixels={pixels}",
        scene.textures.len()
    );
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
            as_component: false,
        },
        &control,
    )?;
    for occurrence in result.history.occurrences()? {
        let model = result
            .history
            .definition_model(occurrence.definition)
            .unwrap();
        let triangles: Vec<_> = model
            .faces()
            .iter()
            .filter(|(_, f)| f.outer.len() == 3 && f.holes.is_empty())
            .collect();
        let mut unmerged = std::collections::BTreeSet::new();
        for (i, (a, fa)) in triangles.iter().enumerate() {
            for (b, fb) in triangles.iter().skip(i + 1) {
                let parallel = fa.plane.normal.cross(fb.plane.normal).length() < 1e-7;
                let shared = fa.outer.iter().filter(|v| fb.outer.contains(v)).count();
                let same_plane = (fa.plane.normal.dot(model.vertices()[&fb.outer[0]].position)
                    - fa.plane.distance)
                    .abs()
                    < 1e-7;
                if parallel && shared == 2 && same_plane {
                    unmerged.insert(**a);
                    unmerged.insert(**b);
                }
            }
        }
        eprintln!(
            "triangles={} edge-adjacent coplanar triangles remaining={}",
            triangles.len(),
            unmerged.len()
        );
        eprintln!(
            "occurrence {:?} definition {:?} kind {:?}: V={} E={} F={}",
            occurrence.path,
            occurrence.definition,
            result.history.definition_kind(occurrence.definition),
            model.vertices().len(),
            model.edges().len(),
            model.faces().len()
        );
    }
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
