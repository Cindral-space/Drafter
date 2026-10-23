//! One Rotate interaction for loose geometry and occurrence placements.
use super::*;
use cindral_core::{InstancePath, OrganizationCommand as Org};
use glam::{DMat4, DVec2};

#[derive(Default)]
pub(crate) struct RotateUi {
    reference_set: bool,
    point: Option<DVec3>,
    normal: Option<DVec3>,
    instance: Option<(InstancePath, DMat4, DMat4)>,
}
impl DrafterApp {
    pub(crate) fn rotate_hint(&self) -> &'static str {
        if !self.modify.active() {
            if self.selection.vertex_ids(self.history.model()).is_empty() && self.organization.selected.is_none() {
                "Select objects to rotate"
            } else { "Click to set rotation center" }
        } else if !self.rotate.reference_set { "Click to set start angle" }
        else { "Move to rotate, click to finish, or type an angle" }
    }
    fn rotate_command(&self, command: Command) -> Result<Option<Org>, cindral_core::KernelError> {
        if let Some((path, before, owner)) = &self.rotate.instance {
            if let Command::Transform { matrix, copy, .. } = command {
                let transform = owner.inverse() * matrix * *owner * *before;
                return Ok(Some(if copy { Org::Duplicate { path: path.clone(), transform } } else { Org::Transform { path: path.clone(), transform } }));
            }
        }
        Ok(None)
    }
    fn rotate_local_command(&self, command: Command) -> Command {
        let context = self.history.world_transform();
        match command {
            Command::Transform { vertices, matrix, copy } => Command::Transform { vertices, matrix: context.inverse() * matrix * context, copy },
            Command::Batch(commands) => Command::Batch(commands.into_iter().map(|c| self.rotate_local_command(c)).collect()),
            c => c,
        }
    }
    fn rotate_preview(&mut self, command: Command) {
        let result = match self.rotate_command(command.clone()) {
            Ok(Some(Org::Transform { path, transform })) => self.history.preview_instance_transform(&path, transform),
            Ok(Some(Org::Duplicate { path, transform })) => self.history.preview_duplicate(&path, transform),
            Ok(None) => self.history.preview(self.rotate_local_command(command)),
            Err(e) => Err(e),
            _ => unreachable!(),
        };
        self.error = result.err().map(|e| e.to_string());
        self.refresh_mesh();
    }
    pub(crate) fn rotate_measurement(&mut self) {
        let before = self.modify.clone();
        let replacing = self.modify.editing_completed();
        let result = self.modify.measure_with_settings(&self.measurements.text, self.presentation.measurements);
        match result {
            Ok(Some(command)) => {
                let result = match self.rotate_command(command.clone()) {
                    Ok(Some(command)) if replacing => self.history.replace_organization(command),
                    Ok(Some(command)) => self.history.organization(command),
                    Ok(None) if replacing => self.history.replace_last(self.rotate_local_command(command)),
                    Ok(None) => self.history.preview_atomic(self.rotate_local_command(command)).and_then(|_| self.history.commit_preview()),
                    Err(e) => Err(e),
                };
                match result {
                    Ok(true) => { self.modify.finish(); self.measurements.completed(); self.error = None; self.refresh_mesh(); }
                    result => { self.modify = before; self.error = Some(result.err().map(|e| e.to_string()).unwrap_or_else(|| "No change".into())); }
                }
            }
            Err(e) => { self.modify = before; self.error = Some(e.to_string()); }
            _ => {}
        }
        self.measurements.invalid = self.error.is_some();
    }
    pub(crate) fn rotate_pointer(&mut self, response: &egui::Response) {
        let ctx = &response.ctx;
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) { self.cancel_modify(); self.measurements.reset(); self.error = None; return; }
            let undo = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Z));
            let redo = ctx.input(|i| i.modifiers.command && (i.key_pressed(Key::Y) || i.modifiers.shift && i.key_pressed(Key::Z)));
            if undo || redo { self.cancel_modify(); self.history_step(redo); return; }
        }
        if !response.hovered() { return; }
        let rect = response.rect;
        let Some(position) = ctx.input(|i| i.pointer.hover_pos()) else { return; };
        let cursor = DVec2::new((position.x-rect.left()) as f64, (position.y-rect.top()) as f64);
        let aspect = rect.width()/rect.height().max(1.);
        let view_world = View { matrix: self.camera.view_projection(aspect).as_dmat4(), size: DVec2::new(rect.width() as f64, rect.height() as f64) };
        let context = self.history.world_transform();
        let inverse = context.inverse();
        let view = View { matrix: view_world.matrix * context, size: view_world.size };
        let (origin, direction) = self.camera.ray(Vec2::new(2.*cursor.x as f32/rect.width()-1., 1.-2.*cursor.y as f32/rect.height()), aspect);
        let world_ray = Ray { origin: origin.as_dvec3(), direction: direction.as_dvec3() };
        let ray = Ray { origin: inverse.transform_point3(world_ray.origin), direction: inverse.transform_vector3(world_ray.direction) };
        let hit = cindral_tools::pick_section(self.history.model(), ray, view, cursor, 10., self.local_section().as_ref());
        let normal = self.rotate.normal.unwrap_or_else(|| match hit {
            Some(cindral_tools::Hit::Face(id)) => inverse.transpose().transform_vector3(self.history.model().faces()[&id].plane.normal).normalize(),
            _ => DVec3::Z,
        });
        let center = self.modify.anchor().unwrap_or(DVec3::ZERO);
        let local_normal = context.transpose().transform_vector3(normal).normalize();
        let mut query = Query::new(cursor, ray, view);
        query.plane = DrawingPlane { origin: inverse.transform_point3(center), normal: local_normal };
        query.anchor = self.modify.anchor().map(|p| inverse.transform_point3(p));
        query.section = self.local_section();
        self.snap = self.engine.infer(self.history.model(), &query);
        let point = self.snap.as_ref().and_then(|s| {
            let own = self.modify.active() && match s.entity {
                Some(SnapEntity::Vertex(id)) => self.history.changes().vertices.contains(&id),
                Some(SnapEntity::Edge(id)) => self.history.changes().edges.contains(&id),
                Some(SnapEntity::Face(id)) => self.history.changes().faces.contains(&id),
                None => false,
            };
            (!own).then_some(context.transform_point3(s.position))
        }).or_else(|| DrawingPlane { origin: center, normal }.intersect(world_ray));
        self.rotate.point = point;
        let clicked = response.clicked_by(PointerButton::Primary);
        if !self.modify.active() {
            self.rotate.normal = Some(normal);
            if clicked {
                let vertices = self.selection.vertex_ids(self.history.model());
                let instance = self.organization.selected.as_ref().filter(|p| p.starts_with(self.history.context()) && p.len() == self.history.context().len()+1).and_then(|path| {
                    let instance = self.history.instance(path)?;
                    Some((path.clone(), instance.transform, context))
                });
                if vertices.is_empty() && instance.is_none() { self.error = Some("Select objects to rotate".into()); return; }
                if let Some(p) = point {
                    self.rotate.instance = instance;
                    self.rotate.reference_set = false;
                    self.measurements.reset();
                    self.error = self.modify.begin_rotation(vertices, p, normal).err().map(|e| e.to_string());
                }
            }
            return;
        }
        if !self.rotate.reference_set {
            if clicked {
                if let Some(p) = point {
                    match self.modify.set_reference(p) { Ok(()) => { self.rotate.reference_set = true; self.error = None; }, Err(e) => self.error = Some(e.to_string()) }
                }
            }
            return;
        }
        if let Some(p) = point {
            match self.modify.update(p) { Ok(Some(c)) => self.rotate_preview(c), Err(e) => self.error = Some(e.to_string()), _ => {} }
        }
        if clicked && self.error.is_none() {
            match self.history.commit_preview() {
                Ok(true) => { self.modify.finish(); self.measurements.completed(); },
                Err(e) => self.error = Some(e.to_string()),
                _ => {}
            }
        }
    }
    pub(crate) fn rotate_overlay(&self, _ui: &egui::Ui, _rect: egui::Rect) {}
}
