//! Release-only import/edit performance regression, run explicitly with --ignored.
use cindral_core::*;
use std::time::{Duration, Instant};
#[test]
#[ignore = "100k triangle timing requires --release --ignored --nocapture"]
fn import_100k_triangles_and_edit() {
    assert!(!cfg!(debug_assertions), "benchmark requires --release");
    let mut g=BulkGeometry::default();
    for y in 0..200 { for x in 0..250 {
        let base=g.vertices.len();let p=DVec3::new(x as f64,y as f64,0.);
        g.vertices.extend([p,p+DVec3::X,p+DVec3::X+DVec3::Y,p+DVec3::Y]);
        g.faces.push(BulkFace{outer:vec![base,base+1,base+2],..Default::default()});
        g.faces.push(BulkFace{outer:vec![base,base+2,base+3],..Default::default()});
    }}
    let start=Instant::now();let model=g.build(true,true).unwrap();
    let mut data=DocumentHistory::new().to_data();data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry=model.to_data();
    let mut h=DocumentHistory::from_import_data(data).unwrap();let elapsed=start.elapsed();
    let original_faces=h.model().faces().len();assert!(original_faces<=60_000);assert_eq!(h.undo_depth(),1);
    let edit=Instant::now();h.execute(Command::AddEdge{start:DVec3::new(0.,0.5,0.),end:DVec3::new(0.5,0.,0.)}).unwrap();let edit=edit.elapsed();
    eprintln!("100k triangles: import {:.3}s; connected corner chord edit {:.3}ms; {} faces",elapsed.as_secs_f64(),edit.as_secs_f64()*1000.,h.model().faces().len());
    assert!(elapsed<Duration::from_secs(10),"import budget exceeded: {elapsed:?}");
    assert!(edit<Duration::from_millis(16),"edit target exceeded: {edit:?}");
    assert_eq!(h.undo_depth(),2);h.undo().unwrap();assert_eq!(h.model().faces().len(),original_faces);
}
