//! Section placement, live view-only movement and cached cap meshes.
use super::*;
use cindral_core::SectionPlane;
use cindral_render::mesh::FaceVertex;
#[derive(Default)]
pub(crate) struct SectionUi {
    placing: bool,
    axis: Option<Axis>,
    moving: bool,
    drag: Option<(SectionPlane, egui::Pos2)>,
    stamp: Option<(u64, Option<SectionPlane>)>,
    pub(crate) caps: Arc<Vec<FaceVertex>>,
    pub(crate) bounds: Option<[glam::Vec3; 2]>,
    bounds_revision: Option<u64>,
}
impl DrafterApp {
    pub(crate) fn local_section(&self) -> Option<SectionPlane> {
        self.presentation.sections.cut()?.in_frame(self.history.world_transform())
    }
    pub(crate) fn place_section(&mut self, point: DVec3, normal: DVec3) {
        let Some(plane)=SectionPlane::through(format!("Section {}", self.presentation.sections.planes.len()+1),point,normal) else {return;};
        let state=&mut self.presentation.sections;
        if state.planes.len()>=1000 {return;}
        state.planes.push(plane); state.active=Some(state.planes.len()-1); state.enabled=true;
        self.section_ui.placing=false;
        self.presentation_changed();
    }
    pub(crate) fn section_panel(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Section planes",|ui| {
            let before=self.presentation.sections.clone();
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("section_axis").selected_text(match self.section_ui.axis {None=>"Face",Some(Axis::X)=>"X",Some(Axis::Y)=>"Y",Some(Axis::Z)=>"Z"}).show_ui(ui,|ui| {
                    ui.selectable_value(&mut self.section_ui.axis,None,"Face");
                    for a in [Axis::X,Axis::Y,Axis::Z] {ui.selectable_value(&mut self.section_ui.axis,Some(a),format!("{a:?}"));}
                });
                if ui.selectable_label(self.section_ui.placing,"Place").clicked() {
                    self.cancel_modify(); self.tool.cancel(); self.section_ui.placing=!self.section_ui.placing;
                    self.section_ui.moving=false;
                }
            });
            let state=&mut self.presentation.sections;
            ui.checkbox(&mut state.enabled,"Enable cut");
            for (i,p) in state.planes.iter_mut().enumerate() {
                ui.horizontal(|ui| {ui.selectable_value(&mut state.active,Some(i),&p.name); ui.checkbox(&mut p.visible,"Widget");});
            }
            if let Some(p)=state.active.and_then(|i|state.planes.get_mut(i)) {
                ui.text_edit_singleline(&mut p.name);
                ui.horizontal(|ui| {
                    if ui.button("Flip").clicked() {p.flip();}
                    ui.toggle_value(&mut self.section_ui.moving,"Move widget");
                });
                ui.add(egui::DragValue::new(&mut p.distance).speed(0.05).prefix("Offset ").suffix(" m").range(-1e6..=1e6));
                ui.label("Drag the widget to move; Esc cancels.");
                if ui.button("Delete plane").clicked() {
                    if let Some(i)=state.active {state.planes.remove(i);}
                    state.active=(!state.planes.is_empty()).then_some(0);
                    self.section_ui.drag=None;
                }
            }
            if before!=self.presentation.sections {self.presentation_changed();}
        });
    }
    pub(crate) fn section_input(&mut self,response:&egui::Response)->bool {
        let ui=&mut self.section_ui;
        if !(ui.placing || ui.moving) {return false;}
        if response.ctx.input(|i|i.key_pressed(Key::Escape)) {
            if let Some((original,_))=ui.drag.take() {
                if let Some(p)=self.presentation.sections.active.and_then(|i|self.presentation.sections.planes.get_mut(i)) {*p=original;}
            }
            ui.placing=false;ui.moving=false;self.presentation_changed();return true;
        }
        let Some(pos)=response.ctx.input(|i|i.pointer.hover_pos()) else {return true;};
        let rect=response.rect;
        let cursor=glam::DVec2::new((pos.x-rect.left()) as f64,(pos.y-rect.top()) as f64);
        let aspect=rect.width()/rect.height().max(1.);
        let view=View {matrix:self.camera.view_projection(aspect).as_dmat4(),size:glam::DVec2::new(rect.width() as f64,rect.height() as f64)};
        let (o,d)=self.camera.ray(Vec2::new(2.*cursor.x as f32/rect.width()-1.,1.-2.*cursor.y as f32/rect.height()),aspect);
        if self.section_ui.placing && response.clicked_by(PointerButton::Primary) {
            let ray=Ray {origin:o.as_dvec3(),direction:d.as_dvec3()};
            let hit=self.scene_hit(ray,view,cursor);
            let normal=self.section_ui.axis.map(Axis::direction).or_else(|| {
                let hit=hit.as_ref()?;
                let cindral_tools::Hit::Face(id)=hit.hit else {return None;};
                let occurrence=self.scene.occurrences().iter().find(|v|v.path==hit.path)?;
                let n=self.history.definition_model(occurrence.definition)?.faces().get(&id)?.plane.normal;
                Some(occurrence.world.inverse().transpose().transform_vector3(n).normalize())
            });
            if let Some(normal)=normal {
                let point=hit.map(|h|h.world_position).or_else(||DrawingPlane {origin:DVec3::ZERO,normal}.intersect(ray));
                if let Some(point)=point {self.place_section(point,normal);}
            }
        }
        if self.section_ui.moving {
            let Some(p)=self.presentation.sections.active.and_then(|i|self.presentation.sections.planes.get_mut(i)) else {return true;};
            let n=DVec3::from_array(p.normal);
            let center=self.camera.target.as_dvec3()-n*p.signed_distance(self.camera.target.as_dvec3());
            if response.drag_started_by(PointerButton::Primary) && p.visible {
                if view.project(center).is_some_and(|c| c.distance(cursor)<55.) {self.section_ui.drag=Some((p.clone(),pos));}
            }
            if let Some((original,start))=&self.section_ui.drag {
                let delta=pos-*start;
                let pixels=view.project(center+n).zip(view.project(center)).map(|(a,b)|a-b).unwrap_or(glam::DVec2::ZERO);
                let offset=if pixels.length()>1. {glam::DVec2::new(delta.x as f64,delta.y as f64).dot(pixels)/pixels.length_squared()} else {-(delta.y as f64)*self.camera.distance as f64/rect.height() as f64};
                p.distance= (original.distance+offset).clamp(-1e6,1e6);
                self.presentation_changed();
                response.ctx.request_repaint();
            }
            if response.drag_stopped_by(PointerButton::Primary) {self.section_ui.drag=None;}
        }
        true
    }
    pub(crate) fn section_widgets(&self,ui:&egui::Ui,rect:egui::Rect) {
        let view=View {matrix:self.camera.view_projection(rect.width()/rect.height().max(1.)).as_dmat4(),size:glam::DVec2::new(rect.width() as f64,rect.height() as f64)};
        for (i,p) in self.presentation.sections.planes.iter().enumerate().filter(|(_,p)|p.visible) {
            let n=DVec3::from_array(p.normal);let center=self.camera.target.as_dvec3()-n*p.signed_distance(self.camera.target.as_dvec3());
            let u=n.any_orthonormal_vector();let v=n.cross(u);let size=self.camera.distance as f64*0.07;
            let points:Option<Vec<_>>=[-u-v,u-v,u+v,-u+v].iter().map(|d|view.project(center+*d*size).map(|p|rect.min+egui::vec2(p.x as f32,p.y as f32))).collect();
            if let Some(points)=points {
                let color=if self.presentation.sections.active==Some(i) {Color32::from_rgb(255,175,55)} else {Color32::GRAY};
                ui.painter().add(egui::Shape::convex_polygon(points.clone(),color.gamma_multiply(0.12),egui::Stroke::new(1.5_f32,color)));
                if let Some(p)=view.project(center) {let p=rect.min+egui::vec2(p.x as f32,p.y as f32);ui.painter().text(p,egui::Align2::CENTER_CENTER,&self.presentation.sections.planes[i].name,egui::FontId::proportional(12.),color);}
            }
        }
    }
    pub(crate) fn prepare_sections(&mut self) {
        let revision=self.history.revision();
        if self.section_ui.bounds_revision!=Some(revision) {
            let mut bounds:Option<[glam::Vec3;2]>=None;
            let mut local=std::collections::BTreeMap::new();
            for o in self.scene.occurrences() {
                let b=*local.entry(o.definition).or_insert_with(||self.history.definition_model(o.definition).and_then(|m|m.bounds()));
                if let Some((min,max))=b {for x in [min.x,max.x] {for y in [min.y,max.y] {for z in [min.z,max.z] {
                    let p=o.world.transform_point3(DVec3::new(x,y,z)).as_vec3();
                    bounds=Some(bounds.map_or([p,p],|[a,b]|[a.min(p),b.max(p)]));
                }}}}
            }
            self.section_ui.bounds=bounds;self.section_ui.bounds_revision=Some(revision);
        }
        let plane=self.presentation.sections.cut().cloned();
        let stamp=(revision,plane.clone());
        if self.section_ui.stamp.as_ref()==Some(&stamp) {return;}
        let mut vertices=Vec::new();
        if let Some(plane)=plane {
            let mut cache=std::collections::BTreeMap::new();
            for o in self.scene.occurrences() {
                let Some(local)=plane.in_frame(o.world) else {continue;};
                let key=(o.definition,local.normal.map(f64::to_bits),local.distance.to_bits());
                let cap=cache.entry(key).or_insert_with(|| self.history.definition_model(o.definition).map(|m|cindral_core::section_cap(m,&local)).transpose().map(|c|c.unwrap_or_default()));
                match cap {
                    Ok(cap)=>for tri in cap {for p in tri {
                        vertices.push(FaceVertex {position:o.world.transform_point3(*p).as_vec3().to_array(),normal:(-DVec3::from_array(plane.normal)).as_vec3().to_array(),color:[0.85,0.48,0.16,1.],material:-1.});
                    }},
                    Err(e)=>self.error=Some(e.to_string()),
                }
            }
        }
        self.section_ui.caps=Arc::new(vertices);self.section_ui.stamp=Some(stamp);
    }
}
