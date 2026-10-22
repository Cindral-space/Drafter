use cindral_core::*;
use cindral_io::*;

fn cube(hole: bool) -> BulkGeometry {
    let mut g = BulkGeometry {
        vertices: vec![DVec3::new(0.,0.,0.),DVec3::new(1.,0.,0.),DVec3::new(1.,1.,0.),DVec3::new(0.,1.,0.),DVec3::new(0.,0.,1.),DVec3::new(1.,0.,1.),DVec3::new(1.,1.,1.),DVec3::new(0.,1.,1.)],
        faces: [[0,3,2,1],[4,5,6,7],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]].into_iter().map(|outer| BulkFace {outer: outer.to_vec(), ..Default::default()}).collect(),
        ..Default::default()
    };
    if hole {
        g.vertices.extend([DVec3::new(0.25,0.25,1.),DVec3::new(0.25,0.75,1.),DVec3::new(0.75,0.75,1.),DVec3::new(0.75,0.25,1.)]);
        g.faces[1].holes.push(vec![8,9,10,11]);
    }
    g
}
fn edit(h: &mut DocumentHistory) {
    let face = h.model().faces().iter().find(|(_,f)| f.plane.normal.z > 0.9).map(|(id,_)| *id).expect("picked top face must be in active root");
    assert!(h.execute(Command::PushPull {face,distance:0.2}).unwrap());
}
fn direct(g: BulkGeometry) -> DocumentHistory {
    let mut d = DocumentHistory::new().to_data();
    d.definitions.get_mut(&DefinitionId(0)).unwrap().geometry = g.build(false,false).unwrap().to_data();
    DocumentHistory::from_data(d).unwrap()
}
#[test]
fn bulk_cube_pushpull() { edit(&mut direct(cube(false))); }
#[test]
fn bulk_cube_with_hole_pushpull() { edit(&mut direct(cube(true))); }
#[test]
fn skp_scene_cube_root_pushpull() {
    let scene = ImportedScene {definitions: vec![ImportedDefinition {name:"Model".into(),geometry:cube(false),..Default::default()}], roots:vec![ImportedInstance::default()],..Default::default()};
    let mut h = convert_scene(scene,&ImportOptions::default(),&ImportControl::default()).unwrap().history;
    edit(&mut h);
}
#[test]
fn skp_scene_cube_with_hole_root_pushpull() {
    let scene = ImportedScene {definitions: vec![ImportedDefinition {name:"Model".into(),geometry:cube(true),..Default::default()}], roots:vec![ImportedInstance::default()],..Default::default()};
    let mut h = convert_scene(scene,&ImportOptions::default(),&ImportControl::default()).unwrap().history;
    edit(&mut h);
}

#[test]
fn rectangle_tool_cube_pushpull() {
    let mut h = DocumentHistory::new();
    let mut rectangle = cindral_tools::ToolSession::new(cindral_tools::ToolKind::Rectangle);
    rectangle.click(DVec3::ZERO).unwrap();
    h.execute(rectangle.click(DVec3::new(1.,1.,0.)).unwrap().unwrap()).unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    h.execute(Command::PushPull {face,distance:1.}).unwrap();
    edit(&mut h);
}
