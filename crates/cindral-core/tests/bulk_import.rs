//! Bulk construction invariants and import transaction behavior.
use cindral_core::*;
fn square() -> BulkGeometry {
    BulkGeometry {vertices:vec![DVec3::ZERO,DVec3::X,DVec3::X+DVec3::Y,DVec3::Y,DVec3::ZERO+DVec3::splat(TOLERANCE/4.)],faces:vec![BulkFace{outer:vec![0,1,2],..Default::default()},BulkFace{outer:vec![4,2,3],..Default::default()}],..Default::default()}
}
#[test]
fn weld_merge_style_and_validate() {
    let m=square().build(true,true).unwrap();assert_eq!(m.vertices().len(),4);assert_eq!(m.faces().len(),1);assert_eq!(m.edges().len(),4);m.validate().unwrap();
    let m=square().build(false,true).unwrap();assert_eq!(m.faces().len(),2);assert_eq!(m.edges().values().filter(|e|e.soft&&e.smooth).count(),1);
}
#[test]
fn import_undo_redo_then_local_edit() {
    let mut data=DocumentHistory::new().to_data();data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry=square().build(true,true).unwrap().to_data();
    let mut h=DocumentHistory::from_import_data(data).unwrap();let original=h.to_data();assert_eq!(h.undo_depth(),1);
    h.execute(Command::AddEdge{start:DVec3::new(10.,0.,0.),end:DVec3::new(11.,0.,0.)}).unwrap();assert_eq!(h.undo_depth(),2);
    h.undo().unwrap();assert_eq!(h.to_data().definitions,original.definitions);
    h.undo().unwrap();assert!(h.model().faces().is_empty());assert!(!h.changes().faces.is_empty());h.redo().unwrap();assert_eq!(h.to_data(),original);h.redo().unwrap();assert_eq!(h.model().edges().len(),5);
}
#[test]
fn bad_indices_nonfinite_cancel() {
    let mut g=square();g.faces[0].outer[1]=usize::MAX;assert!(g.build(false,false).is_err());
    let mut g=square();g.vertices[0].x=f64::NAN;assert!(g.build(false,false).is_err());
    assert!(square().build_cancellable(true,true,||true).is_err());
}
#[test]
fn append_retains_existing_history_and_imports_one_step() {
    let mut h=DocumentHistory::new();h.execute(Command::AddEdge{start:DVec3::new(10.,0.,0.),end:DVec3::new(11.,0.,0.)}).unwrap();let before=h.to_data();
    let mut data=DocumentHistory::new().to_data();data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry=square().build(true,true).unwrap().to_data();
    h.append_import(data).unwrap();assert_eq!(h.undo_depth(),2);assert_eq!(h.model().faces().len(),1);assert_eq!(h.model().edges().len(),5);
    h.undo().unwrap();assert_eq!(h.to_data().definitions,before.definitions);h.undo().unwrap();assert!(h.model().edges().is_empty());h.redo().unwrap();h.redo().unwrap();assert_eq!(h.model().edges().len(),5);
}
