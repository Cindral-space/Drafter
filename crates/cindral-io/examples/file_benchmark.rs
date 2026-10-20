//! Actual native save/load latency, including validation, compression and durable atomic replacement.
use cindral_core::*;
use cindral_io::*;
use glam::DMat4;
use std::{collections::BTreeMap, time::Instant};
fn placed_scene() -> Result<DraftBundle> {
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
    let mut document = DocumentHistory::new().to_data();
    let definition = DefinitionId(1);
    document.definitions.insert(
        definition,
        DefinitionData {
            name: "Box".into(),
            kind: DefinitionKind::Component,
            geometry: h.model().to_data(),
            instances: BTreeMap::new(),
        },
    );
    document
        .definitions
        .get_mut(&DefinitionId(0))
        .ok_or(KernelError::MissingEntity)?
        .instances = (0..5000)
        .map(|i| {
            let id = InstanceId(i + 2);
            (
                id,
                Instance {
                    id,
                    name: format!("Box {i}"),
                    definition,
                    transform: DMat4::from_translation(DVec3::new(
                        (i % 100) as f64 * 2.,
                        (i / 100) as f64 * 2.,
                        0.,
                    )),
                    tag: None,
                    material: None,
                    hidden: false,
                },
            )
        })
        .collect();
    document.next_id = 5002;
    Ok(DraftBundle {
        document,
        textures: BTreeMap::new(),
    })
}
fn connected(n: usize) -> Result<DraftBundle> {
    let (h, _) = cindral_core::connected_fixture::connected_solid(n)?;
    let mut bundle = DraftBundle::default();
    bundle
        .document
        .definitions
        .get_mut(&DefinitionId(0))
        .ok_or(KernelError::MissingEntity)?
        .geometry = h.model().to_data();
    Ok(bundle)
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for (name, bundle) in [
        ("connected-21600", connected(60)?),
        ("connected-60000", connected(100)?),
        ("instances-5000", placed_scene()?),
    ] {
        let path = directory.path().join(format!("{name}.cdraft"));
        for run in 1..=3 {
            let start = Instant::now();
            save_draft(&path, &bundle)?;
            let save = start.elapsed().as_secs_f64() * 1000.;
            let start = Instant::now();
            let loaded = load_draft(&path)?;
            let load = start.elapsed().as_secs_f64() * 1000.;
            if loaded != bundle {
                return Err("native benchmark round trip changed file values".into());
            }
            println!(
                "{name} run{run}: save_ms {save:.3}; load_ms {load:.3}; zip_bytes {}",
                std::fs::metadata(&path)?.len()
            );
        }
    }
    Ok(())
}
