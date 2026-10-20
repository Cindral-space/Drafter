//! Reproducible M5 CPU workload using the actual document and scene-cache paths.
use crate::scene_cache::SceneCache;
use cindral_core::*;
use cindral_render::scene::SceneUpdate;
use glam::DMat4;
use std::time::Instant;
fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
fn placed(data: GeometryData, count: usize, kind: DefinitionKind) -> DocumentData {
    let mut doc = DocumentHistory::new().to_data();
    let instances = (0..count)
        .map(|i| {
            let id = InstanceId(i as u64 + 2);
            let transform = DMat4::from_translation(DVec3::new(
                (i % 100) as f64 * 2.,
                (i / 100) as f64 * 2.,
                0.,
            ));
            (
                id,
                Instance {
                    id,
                    name: format!("Part {}", i + 1),
                    definition: DefinitionId(1),
                    transform,
                    tag: None,
                    material: None,
                    hidden: false,
                },
            )
        })
        .collect();
    if let Some(root) = doc.definitions.get_mut(&DefinitionId(0)) {
        root.instances = instances;
    }
    doc.definitions.insert(
        DefinitionId(1),
        DefinitionData {
            name: "Part".into(),
            kind,
            geometry: data,
            instances: Default::default(),
        },
    );
    doc.next_id = count as u64 + 2;
    doc
}
/// Captured mesh and matrix updates, in the order the GPU benchmark should apply them.
pub struct Workload {
    /// Full scene initialization.
    pub initial: SceneUpdate,
    /// One shared face edit, with no instance matrix changes.
    pub edit: SceneUpdate,
    /// One occurrence placement, with no geometry changes.
    pub placement: SceneUpdate,
}
/// Measure shared geometry, one placement, undo/redo and dirty mesh updates.
pub fn run_cpu() -> Result<Workload, KernelError> {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
        closed: true,
    })?;
    let face = *h
        .model()
        .faces()
        .keys()
        .next()
        .ok_or(KernelError::MissingEntity)?;
    h.execute(Command::PushPull { face, distance: 1. })?;
    let start = Instant::now();
    let mut doc =
        DocumentHistory::from_data(placed(h.model().to_data(), 5000, DefinitionKind::Component))?;
    println!("5000-component document setup: {:.3} ms", elapsed(start));
    let mut cache = SceneCache::default();
    let start = Instant::now();
    let initial = cache.sync(&doc)?;
    println!("5000 occurrence traversal + initial mesh/instance preparation: {:.3} ms; definitions with geometry: {}",elapsed(start),initial.geometry.len());
    doc.enter(vec![InstanceId(2)])?;
    cache.sync(&doc)?;
    let start = Instant::now();
    doc.execute(Command::PushPull {
        face,
        distance: 0.2,
    })?;
    println!(
        "shared face edit (5000 visible occurrences): {:.3} ms",
        elapsed(start)
    );
    let start = Instant::now();
    let update = cache.sync(&doc)?;
    println!("single-definition dirty mesh preparation: {:.3} ms; updated definitions: {}; instance upload: {}",elapsed(start),update.geometry.len(),update.batches.is_some());
    let edit = update;
    let start = Instant::now();
    doc.undo()?;
    println!("shared edit undo: {:.3} ms", elapsed(start));
    cache.sync(&doc)?;
    let start = Instant::now();
    doc.redo()?;
    println!("shared edit redo: {:.3} ms", elapsed(start));
    cache.sync(&doc)?;
    doc.exit()?;
    cache.sync(&doc)?;
    let start = Instant::now();
    doc.preview_instance_transform(&[InstanceId(2)], DMat4::from_translation(DVec3::Z))?;
    let preview_ms = elapsed(start);
    let start = Instant::now();
    let update = cache.sync(&doc)?;
    println!("one instance transform preview: {:.3} ms; instance batch preparation: {:.3} ms; geometry uploads: {}",preview_ms,elapsed(start),update.geometry.len());
    let placement = update;
    doc.cancel_preview()?;
    let occurrences = doc.occurrences()?;
    let items: Vec<_> = occurrences
        .iter()
        .filter_map(|o| {
            Some(cindral_tools::PickInstance::new(
                o.path.clone(),
                o.definition,
                doc.definition_model(o.definition)?,
                o.world,
            ))
        })
        .collect();
    let vp = DMat4::orthographic_rh(-110., 110., -60., 60., 0.1, 1000.)
        * DMat4::look_at_rh(
            DVec3::new(100., 50., 300.),
            DVec3::new(100., 50., 0.),
            DVec3::Y,
        );
    let view = cindral_inference::View {
        matrix: vp,
        size: glam::DVec2::new(1280., 800.),
    };
    let target = DVec3::new(100.5, 50.5, 1.);
    let cursor = view.project(target).ok_or(KernelError::InvalidPosition)?;
    let start = Instant::now();
    let hit = cindral_tools::pick_instances(
        &items,
        cindral_inference::Ray {
            origin: target + DVec3::Z * 100.,
            direction: -DVec3::Z,
        },
        view,
        cursor,
        5.,
    );
    println!(
        "5000 occurrence transformed pick: {:.3} ms; hit: {}",
        elapsed(start),
        hit.is_some()
    );
    let (solid, _) = cindral_core::connected_fixture::connected_solid(60)?;
    let mut group = DocumentHistory::benchmark_from_data(placed(
        solid.model().to_data(),
        1,
        DefinitionKind::Group,
    ))?;
    let start = Instant::now();
    group.preview_instance_transform(&[InstanceId(2)], DMat4::from_translation(DVec3::Z * 0.1))?;
    println!(
        "21600-face connected group placement preview: {:.3} ms",
        elapsed(start)
    );
    let start = Instant::now();
    group.commit_preview()?;
    println!("connected group placement commit: {:.3} ms", elapsed(start));
    let start = Instant::now();
    group.undo()?;
    println!("connected group placement undo: {:.3} ms", elapsed(start));
    Ok(Workload {
        initial,
        edit,
        placement,
    })
}

/// Measure raw connected edits through document history, CPU mesh cache and a completed GPU consumer.
/// The consumer is invoked once for setup and once per live update; drawing/picking/UI are excluded.
pub fn run_connected(
    segments: usize,
    mode: &str,
    mut upload: impl FnMut(&SceneUpdate) -> Result<(), String>,
) -> Result<(), String> {
    fn run(
        segments: usize,
        mode: &str,
        upload: &mut impl FnMut(&SceneUpdate) -> Result<(), String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (fixture, face) = cindral_core::connected_fixture::connected_solid(segments)?;
        let mut data = DocumentHistory::new().to_data();
        data.definitions
            .get_mut(&DefinitionId(0))
            .ok_or("missing root")?
            .geometry = fixture.model().to_data();
        let mut app = crate::DrafterApp {
            history: DocumentHistory::benchmark_from_data(data)?,
            ..Default::default()
        };
        app.refresh_mesh();
        if let Some(initial) = app.pending.take() {
            upload(&initial)?;
        }
        let vertices = app.model().vertices().keys().copied().collect::<Vec<_>>();
        let mut samples = Vec::new();
        for i in 0..31 {
            let start = Instant::now();
            let distance = 0.1 + i as f64 * 0.001;
            let command = if mode == "pushpull" {
                Command::PushPull { face, distance }
            } else if mode == "move" {
                Command::Transform {
                    vertices: vertices.clone(),
                    matrix: DMat4::from_translation(DVec3::Z * distance),
                    copy: false,
                }
            } else {
                return Err("unknown connected mode".into());
            };
            app.preview(command)?;
            if let Some(update) = app.pending.take() {
                upload(&update)?;
            }
            samples.push(elapsed(start));
        }
        let cold = samples[0];
        let max = samples.iter().copied().fold(0., f64::max);
        let start = Instant::now();
        app.commit_preview()?;
        let commit = elapsed(start);
        if app.document().undo_depth() != 1 {
            return Err("connected preview committed multiple entries".into());
        }
        println!("connected GPU-completed {mode} n{segments} faces{}: cold {cold:.3} ms; steady mean {:.3} ms; max {max:.3} ms; commit {commit:.3} ms",app.model().faces().len(),samples[1..].iter().sum::<f64>()/30.);
        let scale = ((segments as f64 / 60.).powi(2)).max(1.);
        if std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some()
            && (max >= 16. * scale || commit >= 100. * scale)
        {
            return Err("completed GPU update exceeded connected budget".into());
        }
        Ok(())
    }
    run(segments, mode, &mut upload).map_err(|e| e.to_string())
}
