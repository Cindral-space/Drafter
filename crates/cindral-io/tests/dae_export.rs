//! COLLADA export structure, geometry, sharing, round trips and publication safety.
use cindral_core::*;
use cindral_io::*;
use glam::DMat4;
use std::collections::BTreeMap;
fn geometry(holes: bool) -> GeometryData {
    let mut vertices=vec![DVec3::ZERO,DVec3::new(4.,0.,0.),DVec3::new(4.,4.,0.),DVec3::new(0.,4.,0.)];
    let mut inner=Vec::new();
    if holes { vertices.extend([DVec3::new(1.,1.,0.),DVec3::new(1.,3.,0.),DVec3::new(3.,3.,0.),DVec3::new(3.,1.,0.)]);inner.push(vec![4,5,6,7]); }
    BulkGeometry{vertices,faces:vec![BulkFace{outer:vec![0,1,2,3],holes:inner,material:Some(MaterialId(10)),color:[255;4]}],edges:vec![]}.build(false,false).unwrap().to_data()
}
fn instance(id:u64,definition:u64,transform:DMat4)->Instance { Instance{id:InstanceId(id),name:format!("Part <{id}> & \"name\""),definition:DefinitionId(definition),transform,tag:None,material:None,hidden:false} }
fn fixture() -> DraftBundle {
    let mut b=DraftBundle::default(); b.document.next_id=50;
    b.document.materials.insert(MaterialId(10),Material{name:"Paint & glass".into(),color:[0.2,0.4,0.6],opacity:0.4,texture:None});
    b.document.definitions.insert(DefinitionId(1),DefinitionData{name:"Panel".into(),kind:DefinitionKind::Component,geometry:geometry(true),instances:BTreeMap::new()});
    b.document.definitions.insert(DefinitionId(2),DefinitionData{name:"Assembly".into(),kind:DefinitionKind::Component,geometry:Model::default().to_data(),instances:[(InstanceId(3),instance(3,1,DMat4::from_translation(DVec3::Z)))].into()});
    b.document.definitions.get_mut(&DefinitionId(0)).unwrap().instances=[
        (InstanceId(4),instance(4,2,DMat4::from_translation(DVec3::new(10.,2.,3.)))),
        (InstanceId(5),instance(5,2,DMat4::from_scale_rotation_translation(DVec3::new(-2.,3.,0.5),glam::DQuat::from_rotation_z(0.3),DVec3::new(-8.,1.,5.)))),
    ].into(); b
}
fn bounds(b:&DraftBundle)->(DVec3,DVec3,usize,usize) {
    let h=validate_bundle(b).unwrap();let mut min=DVec3::splat(f64::INFINITY);let mut max=DVec3::splat(f64::NEG_INFINITY);let mut faces=0;let mut count=0;
    for o in h.occurrences().unwrap() { let model=h.definition_model(o.definition).unwrap(); if model.faces().is_empty(){continue;} count+=1; faces+=model.faces().len();for v in model.vertices().values(){let p=o.world.transform_point3(v.position); min=min.min(p);max=max.max(p);} }
    (min,max,count,faces)
}
#[test]
fn nested_mirrored_holes_round_trip_both_units() {
    let b=fixture();let original=bounds(&b);
    for units in [DaeUnits::Meters,DaeUnits::Inches] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("scene.dae");
        let summary=export_dae(&path,&b,&DaeOptions{units,..Default::default()},&DaeControl::default()).unwrap();
        assert_eq!((summary.instances,summary.triangles),(2,16));
        let xml=std::fs::read_to_string(&path).unwrap();let doc=roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(doc.root_element().attribute("version"),Some("1.4.1"));
        assert_eq!(doc.descendants().find(|n|n.has_tag_name("up_axis")).unwrap().text(),Some("Z_UP"));
        let geom:Vec<_>=doc.descendants().filter(|n|n.has_tag_name("geometry")).collect();assert_eq!(geom.len(),1);
        let pos=doc.descendants().find(|n|n.attribute("id")==Some("g1-pos-array")).unwrap();assert_eq!(pos.attribute("count"),Some("24"));
        assert_eq!(doc.descendants().filter(|n|n.has_tag_name("instance_node")).count(),3);
        let scene=import_dae(&path,&ImportControl::default()).unwrap();
        let imported=convert_scene(scene,&ImportOptions{merge_coplanar:false,soften_coplanar:false,..Default::default()},&ImportControl::default()).unwrap();
        let loaded=DraftBundle{document:imported.history.to_data(),textures:imported.textures,presentation:Default::default()};let after=bounds(&loaded);
        assert!(original.0.distance(after.0)<1e-7 && original.1.distance(after.1)<1e-7,"{original:?} {after:?}");
        assert_eq!(after.2,original.2);assert_eq!(original.3,2);assert_eq!(after.3,16); // Each annulus becomes eight faces.
        assert!(loaded.document.materials.values().any(|m|m.color==[0.2,0.4,0.6] && (m.opacity-0.4).abs()<1e-6));
        let h=validate_bundle(&loaded).unwrap();
        for o in h.occurrences().unwrap(){let m=h.definition_model(o.definition).unwrap();if !m.faces().is_empty(){let area:f64=m.faces().keys().map(|f|m.face_area(*f).unwrap()).sum();assert!((area-12.).abs()<1e-7);}}
    }
}
#[test]
fn convex_polylists_shared_vertices_normals_and_triangulate_all() {
    let mut b=fixture(); b.document.definitions.get_mut(&DefinitionId(1)).unwrap().geometry=geometry(false);
    for triangulate_all in [false,true] {
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("out.dae");
        export_dae(&p,&b,&DaeOptions{triangulate_all,..Default::default()},&DaeControl::default()).unwrap();
        let xml=std::fs::read_to_string(p).unwrap();let d=roxmltree::Document::parse(&xml).unwrap();
        let tag=if triangulate_all {"triangles"} else {"polylist"};
        assert_eq!(d.descendants().filter(|n|n.has_tag_name(tag)).count(),1);
        let positions:Vec<f64>=d.descendants().find(|n|n.attribute("id")==Some("g1-pos-array")).unwrap().text().unwrap().split_whitespace().map(|v|v.parse().unwrap()).collect();
        let normals:Vec<f64>=d.descendants().find(|n|n.attribute("id")==Some("g1-norm-array")).unwrap().text().unwrap().split_whitespace().map(|v|v.parse().unwrap()).collect();
        assert_eq!(positions.len(),12); assert_eq!(normals.len(),3);
        let values:Vec<usize>=d.descendants().find(|n|n.has_tag_name(tag)).unwrap().children().find(|n|n.has_tag_name("p")).unwrap().text().unwrap().split_whitespace().map(|v|v.parse().unwrap()).collect();
        let corners:Vec<_>=values.chunks(3).map(|c|DVec3::from_slice(&positions[3*c[0]..3*c[0]+3])).collect();
        assert!((corners[1]-corners[0]).cross(corners[2]-corners[0]).dot(DVec3::from_slice(&normals))>0.);
    }
}
#[test]
fn empty_and_selection_only_keep_context_transform() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("out.dae");
    export_dae(&path,&DraftBundle::default(),&DaeOptions::default(),&DaeControl::default()).unwrap();
    let empty=import_dae(&path,&ImportControl::default()).unwrap();assert!(empty.definitions.iter().all(|d|d.geometry.faces.is_empty()));
    let b=fixture();let selection=DaeSelection{context:vec![InstanceId(4),InstanceId(3)],faces:b.document.definitions[&DefinitionId(1)].geometry.faces.keys().copied().collect(),..Default::default()};
    let options=DaeOptions{force:true,selection:Some(selection),..Default::default()};
    let result=export_dae(&path,&b,&options,&DaeControl::default()).unwrap();assert_eq!((result.instances,result.triangles),(1,8));
    let imported=convert_scene(import_dae(&path,&ImportControl::default()).unwrap(),&ImportOptions::default(),&ImportControl::default()).unwrap();
    let loaded=DraftBundle{document:imported.history.to_data(),..Default::default()};let bb=bounds(&loaded);assert!(bb.0.distance(DVec3::new(10.,2.,4.))<1e-7);
    let options=DaeOptions{force:true,selection:Some(DaeSelection::default()),..Default::default()};
    let s=export_dae(&path,&b,&options,&DaeControl::default()).unwrap();assert_eq!(s.instances,0);assert_eq!(s.triangles,0);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("<geometry"));
}
#[test]
fn hidden_occurrences_and_inherited_paint_share_geometry() {
    let mut b=fixture();b.document.definitions.get_mut(&DefinitionId(0)).unwrap().instances.get_mut(&InstanceId(5)).unwrap().hidden=true;
    let dir=tempfile::tempdir().unwrap();let p=dir.path().join("hidden.dae");
    let s=export_dae(&p,&b,&DaeOptions::default(),&DaeControl::default()).unwrap();assert_eq!(s.instances,1);
    let s=export_dae(&p,&b,&DaeOptions{force:true,include_hidden:true,..Default::default()},&DaeControl::default()).unwrap();assert_eq!(s.instances,2);
    let f=b.document.definitions.get_mut(&DefinitionId(1)).unwrap().geometry.faces.values_mut().next().unwrap();f.material=None;f.color=[160,177,193,255];
    b.document.definitions.get_mut(&DefinitionId(0)).unwrap().instances.get_mut(&InstanceId(4)).unwrap().material=Some(MaterialId(10));
    export_dae(&p,&b,&DaeOptions{force:true,include_hidden:true,..Default::default()},&DaeControl::default()).unwrap();
    let text=std::fs::read_to_string(&p).unwrap();let d=roxmltree::Document::parse(&text).unwrap();assert_eq!(d.descendants().filter(|n|n.has_tag_name("geometry")).count(),1);
    assert!(text.contains("symbol=\"inherit\" target=\"#m10\""));assert!(text.contains("symbol=\"inherit\" target=\"#c160_177_193_255\""));
}
fn textured() -> DraftBundle {
    let mut b=fixture();let mut png=std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2,2,image::Rgba([255,120,10,128])).write_to(&mut png,image::ImageFormat::Png).unwrap();
    b.textures.insert("checker".into(),TextureAsset{mime:"image/png".into(),bytes:png.into_inner()});b.document.materials.get_mut(&MaterialId(10)).unwrap().texture=Some("checker".into());b
}
#[test]
fn texture_sidecars_uv_round_trip_and_disable() {
    let b=textured();let dir=tempfile::tempdir().unwrap();let p=dir.path().join("spaces & unicode 模型.dae");
    export_dae(&p,&b,&DaeOptions::default(),&DaeControl::default()).unwrap();
    let text=std::fs::read_to_string(&p).unwrap();let d=roxmltree::Document::parse(&text).unwrap();
    let image=d.descendants().find(|n|n.has_tag_name("image")).unwrap();let uri=image.children().find(|n|n.has_tag_name("init_from")).unwrap().text().unwrap();
    assert!(!std::path::Path::new(uri).is_absolute());assert!(dir.path().join(uri).exists());assert!(text.contains("semantic=\"TEXCOORD\""));
    let scene=import_dae(&p,&ImportControl::default()).unwrap();assert_eq!(scene.textures.len(),1);assert!(scene.materials.iter().any(|m|m.texture.is_some() && (m.opacity-0.4).abs()<1e-6));
    export_dae(&p,&b,&DaeOptions{force:true,include_textures:false,..Default::default()},&DaeControl::default()).unwrap();assert!(!std::fs::read_to_string(&p).unwrap().contains("<library_images>"));
}
#[test]
fn failures_cancel_and_overwrite_leave_no_partial_outputs() {
    let b=textured();let dir=tempfile::tempdir().unwrap();let p=dir.path().join("out.dae");
    std::fs::write(&p,b"original").unwrap();
    assert!(export_dae(&p,&b,&DaeOptions::default(),&DaeControl::default()).unwrap_err().to_string().contains("already exists"));
    let c=DaeControl::default();assert!(c.cancel());assert!(export_dae(&p,&b,&DaeOptions{force:true,..Default::default()},&c).is_err());
    assert_eq!(std::fs::read(&p).unwrap(),b"original");assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(),1);
    let bad=dir.path().join("directory.dae");std::fs::create_dir(&bad).unwrap();
    assert!(export_dae(&bad,&b,&DaeOptions{force:true,..Default::default()},&DaeControl::default()).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(),2); // Images rolled back after publication fails.
    assert!(export_dae(&dir.path().join("missing/out.dae"),&b,&DaeOptions::default(),&DaeControl::default()).unwrap_err().to_string().contains("COLLADA export to"));
    let c=DaeControl::default();export_dae(&p,&b,&DaeOptions{force:true,..Default::default()},&c).unwrap();assert_eq!(c.progress(),1.);assert!(!c.cancel());
}
#[test]
fn concave_faces_are_triangulated_and_shared_instances_stay_small() {
    let mut b=fixture();
    let mesh=BulkGeometry{vertices:vec![DVec3::ZERO,DVec3::new(4.,0.,0.),DVec3::new(4.,1.,0.),DVec3::new(1.,1.,0.),DVec3::new(1.,4.,0.),DVec3::new(0.,4.,0.)],faces:vec![BulkFace{outer:vec![0,1,2,3,4,5],..Default::default()}],edges:vec![]}.build(false,false).unwrap();
    b.document.definitions.get_mut(&DefinitionId(1)).unwrap().geometry=mesh.to_data();
    b.document.definitions.get_mut(&DefinitionId(0)).unwrap().instances=(100..5100).map(|id|(InstanceId(id),instance(id,1,DMat4::from_translation(DVec3::new(id as f64,0.,0.))))).collect();b.document.next_id=5100;
    let dir=tempfile::tempdir().unwrap();let p=dir.path().join("shared.dae");
    let result=export_dae(&p,&b,&DaeOptions::default(),&DaeControl::default()).unwrap();assert_eq!((result.instances,result.triangles),(5000,20000));
    let text=std::fs::read_to_string(&p).unwrap();let d=roxmltree::Document::parse(&text).unwrap();
    assert_eq!(d.descendants().filter(|n|n.has_tag_name("geometry")).count(),1);assert_eq!(d.descendants().filter(|n|n.has_tag_name("polylist")).count(),0);
    assert_eq!(d.descendants().filter(|n|n.has_tag_name("instance_node")).count(),5000);assert!(text.len()<2_000_000);
    let c=DaeControl::default();let worker=c.clone();let output=dir.path().join("cancel.dae");let path=output.clone();
    let t=std::thread::spawn(move||export_dae(&path,&b,&DaeOptions::default(),&worker));
    let start=std::time::Instant::now();while c.progress()<0.1 && !t.is_finished(){assert!(start.elapsed().as_secs()<10);std::thread::yield_now();}
    let cancelled=c.cancel();let result=t.join().unwrap();
    if cancelled {assert!(result.unwrap_err().to_string().contains("cancelled"));assert!(!output.exists());}else{assert!(result.is_ok());assert!(output.exists());}
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().starts_with(".cindral")));
}
#[test]
fn concurrent_no_clobber_exports_publish_one_complete_texture_set() {
    let dir=tempfile::tempdir().unwrap();let p=dir.path().join("race.dae");let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs:Vec<_>=(0..2).map(|_|{let path=p.clone();let barrier=barrier.clone();std::thread::spawn(move||{barrier.wait();export_dae(&path,&textured(),&DaeOptions::default(),&DaeControl::default())})}).collect();
    let results:Vec<_>=jobs.into_iter().map(|j|j.join().unwrap()).collect();assert_eq!(results.iter().filter(|r|r.is_ok()).count(),1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(),2);assert_eq!(import_dae(&p,&ImportControl::default()).unwrap().textures.len(),1);
}
