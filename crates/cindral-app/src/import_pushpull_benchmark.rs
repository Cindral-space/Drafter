//! Release-only drag workload including picking, inference, history and dirty CPU meshes.
use crate::scene_cache::SceneCache;
use cindral_core::*;
use cindral_inference::{Engine, Query, Ray, View};
use cindral_io::{convert_scene, import_file, ImportedScene, ImportedDefinition, ImportedInstance, ImportOptions, ImportControl};
use cindral_render::scene::SceneUpdate;
use cindral_tools::{Hit, ModifyKind, ModifySession, Selection, ToolKind, ToolSession};
use glam::{DMat4, DVec2};
use std::time::Instant;

fn ms(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1000. }
fn sheet(n: usize) -> BulkGeometry {
    BulkGeometry {
        vertices: (0..=n).flat_map(|y| (0..=n).map(move |x| DVec3::new(x as f64, y as f64, 0.))).collect(),
        faces: (0..n).flat_map(|y| (0..n).map(move |x| {
            let a = y*(n+1)+x;
            BulkFace { outer: vec![a,a+1,a+n+2,a+n+1], ..Default::default() }
        })).collect(),
        ..Default::default()
    }
}
fn imported(n: usize) -> DocumentHistory {
    // No local SKP files are required: the DTO uses exactly the SKP converter options.
    let scene = ImportedScene {
        root_definition: Some(0),
        definitions: vec![ImportedDefinition { geometry: sheet(n), instances: (0..300).map(|i| ImportedInstance {
            definition: 1, transform: DMat4::from_translation(DVec3::new(i as f64*2., -5., 0.)), ..Default::default()
        }).collect(), ..Default::default() }, ImportedDefinition { geometry: sheet(1), ..Default::default() }],
        roots: vec![ImportedInstance::default()], ..Default::default()
    };
    convert_scene(scene, &ImportOptions { as_component:false, merge_coplanar:false, soften_coplanar:false }, &ImportControl::default()).unwrap().history
}
fn drawn(n: usize) -> DocumentHistory {
    let mut h = DocumentHistory::new();
    let mut rectangle = ToolSession::new(ToolKind::Rectangle);
    for y in 0..n { for x in 0..n {
        rectangle.click(DVec3::new(x as f64,y as f64,0.)).unwrap();
        h.execute(rectangle.click(DVec3::new((x+1) as f64,(y+1) as f64,0.)).unwrap().unwrap()).unwrap();
    }}
    h
}
/// Run a 60-update sequence; an optional callback measures completed GPU uploads.
/// Setup, import and final diagnostic validation are outside interactive timing.
pub fn run(mut upload: impl FnMut(&SceneUpdate) -> Result<(), String>) -> Result<(), String> {
    if cfg!(debug_assertions) { return Err("use --release".into()); }
    let n = std::env::var("CINDRAL_IMPORT_GRID").ok().map(|s| s.parse::<usize>().unwrap()).unwrap_or(64);
    let enforce = std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some();
    let kinds = std::env::var("CINDRAL_IMPORT_CASE").unwrap_or_else(|_| "imported,drawn".into());
    for kind in kinds.split(',') {
        let mut h = if kind == "drawn" { drawn(n) } else if kind == "local" {
            let path = std::env::var_os("CINDRAL_SKP_BENCHMARK_SEED").ok_or("local case requires CINDRAL_SKP_BENCHMARK_SEED")?;
            import_file(std::path::Path::new(&path), &ImportOptions { as_component:false, ..Default::default() }, &ImportControl::default()).map_err(|e| e.to_string())?.history
        } else { imported(n) };
        let mut cache = SceneCache::default();
        upload(&cache.sync(&h).map_err(|e| e.to_string())?)?;
        let face = *h.model().faces().keys().nth(h.model().faces().len()/2).ok_or("empty root")?;
        let source = &h.model().faces()[&face];
        let anchor = source.outer.iter().map(|v| h.model().vertices()[v].position).sum::<DVec3>() / source.outer.len() as f64;
        let axis = source.plane.normal;
        let bounds = h.model().bounds().unwrap();
        let center = (bounds.0+bounds.1)*0.5;
        // Zoomed out: many tiny imported faces under the pick radius, like an overview drag.
        let extent = (bounds.1-bounds.0).max_element().max(1.) * 1.5;
        let view = View { matrix: DMat4::orthographic_rh(-extent,extent,-extent,extent,0.1,10000.) * DMat4::look_at_rh(center + DVec3::Z*1000.,center,DVec3::Y), size:DVec2::new(1280.,800.) };
        let cursor = view.project(anchor).unwrap();
        let ray = Ray { origin: anchor+DVec3::Z*1000., direction:-DVec3::Z };
        let mut query = Query::new(cursor,ray,view);
        query.anchor = Some(anchor);
        let mut tool = ModifySession::new(ModifyKind::PushPull);
        let t = Instant::now();
        tool.begin(h.model(),&Selection::default(),Some(Hit::Face(face)),anchor,axis).unwrap();
        let begin = ms(t);
        println!("{kind}: V={} E={} F={} instances={} begin={begin:.3}ms",h.model().vertices().len(),h.model().edges().len(),h.model().faces().len(),h.occurrences().unwrap().len());
        let mut maxima = [0_f64;5]; let mut sums = [0.;5]; let mut worst = 0_f64;
        let before = h.model().to_data(); let depth = h.undo_depth();
        for i in 1..=60 {
            let total = Instant::now(); let t = Instant::now();
            std::hint::black_box(cindral_tools::pick(h.model(),ray,view,cursor,10.));
            let pick = ms(t); let t = Instant::now();
            std::hint::black_box(Engine::new().infer(h.model(),&query));
            let infer = ms(t); let t = Instant::now();
            let c = tool.update(anchor+axis*(i as f64*0.001)).unwrap().unwrap();
            h.preview(c).unwrap();
            let preview = ms(t); let t = Instant::now();
            let update = cache.sync(&h).unwrap();
            let cpu = ms(t); let t = Instant::now(); upload(&update)?; let gpu = ms(t);
            assert!(update.batches.is_none(), "geometry preview must not rewrite instances");
            assert!(update.geometry.iter().all(|g| !g.mesh.reset));
            let bytes:usize = update.geometry.iter().map(|g| g.mesh.face_writes.iter().chain(&g.mesh.edge_writes).map(|w|w.bytes.len()).sum::<usize>()).sum();
            if i==1 { println!("dirty first update: definitions={} bytes={bytes}",update.geometry.len()); }
            for (j,value) in [pick,infer,preview,cpu,gpu].into_iter().enumerate() { maxima[j]=maxima[j].max(value); sums[j]+=value; }
            let elapsed = ms(total); worst=worst.max(elapsed);
            if enforce { assert!(elapsed<16., "preview {i} exceeded 16ms: {elapsed:.3}"); }
        }
        let t=Instant::now(); assert!(h.commit_preview().unwrap()); let commit=ms(t);
        println!("{kind}: phase mean/max ms pick={:.3}/{:.3} inference={:.3}/{:.3} kernel={:.3}/{:.3} cpu_mesh={:.3}/{:.3} upload={:.3}/{:.3}; total_max={worst:.3} commit={commit:.3}",sums[0]/60.,maxima[0],sums[1]/60.,maxima[1],sums[2]/60.,maxima[2],sums[3]/60.,maxima[3],sums[4]/60.,maxima[4]);
        assert_eq!(h.undo_depth(),depth+1);
        let after=h.model().to_data();
        h.undo().unwrap(); assert_eq!(h.model().to_data(),before);
        h.redo().unwrap(); assert_eq!(h.model().to_data(),after);
        if enforce { assert!(begin<250. && commit<100.); }
    }
    Ok(())
}
