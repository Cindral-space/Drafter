//! One Rotate interaction for loose geometry and occurrence placements.
use super::*;
use cindral_core::{InstancePath, OrganizationCommand as Org};
use glam::{DMat4, DVec2};

#[derive(Default)]
pub(crate) struct RotateUi {
    pub(crate) reference_set: bool,
    pub(crate) copy: bool,
    copy_key_down: bool,
    point: Option<DVec3>,
    normal: Option<DVec3>,
    inferred: bool,
    has_loose: bool,
    lock: Option<DVec3>,
    held: Option<DVec3>,
    radius: f64,
    inference_direction: Option<DVec3>,
    others: Vec<(InstancePath, DMat4, DMat4)>,
    instance: Option<(InstancePath, DMat4, DMat4)>,
}
impl DrafterApp {
    pub(crate) fn rotate_hint(&self) -> &'static str {
        if !self.modify.active() {
            if self.selection.vertex_ids(self.history.model()).is_empty()
                && self.organization.selected.is_none()
            {
                "Select objects to rotate"
            } else {
                "Click to set rotation center"
            }
        } else if !self.rotate.reference_set {
            "Click to set start angle"
        } else {
            "Move to rotate, click to finish, or type an angle"
        }
    }
    fn rotate_command(&self, command: Command) -> Result<Option<Org>, cindral_core::KernelError> {
        if !self.rotate.others.is_empty() {
            let mut sources = self.rotate.others.clone();
            if let Some(source) = self.rotate.instance.clone() {
                sources.push(source);
            }
            sources.sort_by(|a, b| a.0.cmp(&b.0));
            let (matrices, copy) = match &command {
                Command::Transform { matrix, copy, .. } => (vec![*matrix], *copy),
                Command::Batch(commands) => (
                    commands
                        .iter()
                        .filter_map(|c| match c {
                            Command::Transform { matrix, .. } => Some(*matrix),
                            _ => None,
                        })
                        .collect(),
                    true,
                ),
                _ => return Ok(None),
            };
            let placements = sources
                .into_iter()
                .map(|(path, before, owner)| {
                    (
                        path,
                        matrices
                            .iter()
                            .map(|m| owner.inverse() * *m * owner * before)
                            .collect(),
                    )
                })
                .collect();
            return Ok(Some(Org::TransformSet { placements, copy }));
        }
        if let Some((path, before, owner)) = &self.rotate.instance {
            if let Command::Batch(commands) = &command {
                let transforms = commands
                    .iter()
                    .filter_map(|c| match c {
                        Command::Transform { matrix, .. } => {
                            Some(owner.inverse() * *matrix * *owner * *before)
                        }
                        _ => None,
                    })
                    .collect();
                return Ok(Some(Org::DuplicateMany {
                    path: path.clone(),
                    transforms,
                }));
            }
            if let Command::Transform { matrix, copy, .. } = command {
                let transform = owner.inverse() * matrix * *owner * *before;
                return Ok(Some(if copy {
                    Org::Duplicate {
                        path: path.clone(),
                        transform,
                    }
                } else {
                    Org::Transform {
                        path: path.clone(),
                        transform,
                    }
                }));
            }
        }
        Ok(None)
    }
    fn rotate_local_command(&self, command: Command) -> Command {
        let context = self.history.world_transform();
        match command {
            Command::Transform {
                vertices,
                matrix,
                copy,
            } => Command::Transform {
                vertices,
                matrix: context.inverse() * matrix * context,
                copy,
            },
            Command::Batch(commands) => Command::Batch(
                commands
                    .into_iter()
                    .map(|c| self.rotate_local_command(c))
                    .collect(),
            ),
            c => c,
        }
    }
    fn rotate_preview(&mut self, command: Command) {
        let result = match self.rotate_command(command.clone()) {
            Ok(Some(placement)) if self.rotate.has_loose => self
                .history
                .preview_selection_transform(self.rotate_local_command(command), placement),
            Ok(Some(command @ Org::TransformSet { .. })) => {
                self.history.preview_placement_set(command)
            }
            Ok(Some(Org::Transform { path, transform })) => {
                self.history.preview_instance_transform(&path, transform)
            }
            Ok(Some(Org::Duplicate { path, transform })) => {
                self.history.preview_duplicate(&path, transform)
            }
            Ok(None) => self.history.preview(self.rotate_local_command(command)),
            Err(e) => Err(e),
            _ => unreachable!(),
        };
        if let Err(error) = result {
            self.cancel_modify();
            self.measurements.reset();
            self.error = Some(format!("Rotation canceled: {error}"));
        } else {
            self.error = None;
        }
        self.refresh_mesh();
    }
    pub(crate) fn rotate_measurement(&mut self) {
        let before = self.modify.clone();
        let replacing = self.modify.editing_completed();
        let result = self
            .modify
            .measure_with_settings(&self.measurements.text, self.presentation.measurements);
        match result {
            Ok(Some(command)) => {
                let result = match self.rotate_command(command.clone()) {
                    Ok(Some(placement)) if self.rotate.has_loose && replacing => self
                        .history
                        .replace_selection_transform(self.rotate_local_command(command), placement),
                    Ok(Some(placement)) if self.rotate.has_loose => self
                        .history
                        .preview_selection_transform(self.rotate_local_command(command), placement)
                        .and_then(|_| self.history.commit_preview()),
                    Ok(Some(command)) if replacing => self.history.replace_organization(command),
                    Ok(Some(command)) => self.history.organization(command),
                    Ok(None) if replacing => self
                        .history
                        .replace_last(self.rotate_local_command(command)),
                    Ok(None) => self
                        .history
                        .preview_atomic(self.rotate_local_command(command))
                        .and_then(|_| self.history.commit_preview()),
                    Err(e) => Err(e),
                };
                match result {
                    Ok(true) => {
                        self.modify.finish();
                        self.measurements.completed();
                        self.error = None;
                        self.refresh_mesh();
                    }
                    result => {
                        self.modify = before;
                        self.error = Some(
                            result
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "No change".into()),
                        );
                    }
                }
            }
            Err(e) => {
                self.modify = before;
                self.error = Some(e.to_string());
            }
            _ => {}
        }
        self.measurements.invalid = self.error.is_some();
    }
    pub(crate) fn rotate_pointer(&mut self, response: &egui::Response) {
        let ctx = &response.ctx;
        if !ctx.wants_keyboard_input() && self.measurements.text.is_empty() {
            if ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel)) {
                self.cancel_modify();
                self.measurements.reset();
                self.error = None;
                return;
            }
        }
        let copy_down = ctx.input(|i| {
            if cfg!(target_os = "macos") {
                i.modifiers.alt
            } else {
                i.modifiers.ctrl
            }
        });
        if !ctx.wants_keyboard_input()
            && self.measurements.text.is_empty()
            && copy_down
            && !self.rotate.copy_key_down
        {
            self.rotate.copy = !self.rotate.copy;
            self.modify.set_rotation_copy(self.rotate.copy);
        }
        self.rotate.copy_key_down = copy_down;
        if !response.hovered() {
            return;
        }
        let rect = response.rect;
        let Some(position) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let cursor = DVec2::new(
            (position.x - rect.left()) as f64,
            (position.y - rect.top()) as f64,
        );
        let aspect = rect.width() / rect.height().max(1.);
        let view_world = View {
            matrix: self.camera.view_projection(aspect).as_dmat4(),
            size: DVec2::new(rect.width() as f64, rect.height() as f64),
        };
        let context = self.history.world_transform();
        let inverse = context.inverse();
        let view = View {
            matrix: view_world.matrix * context,
            size: view_world.size,
        };
        let (origin, direction) = self.camera.ray(
            Vec2::new(
                2. * cursor.x as f32 / rect.width() - 1.,
                1. - 2. * cursor.y as f32 / rect.height(),
            ),
            aspect,
        );
        let world_ray = Ray {
            origin: origin.as_dvec3(),
            direction: direction.as_dvec3(),
        };
        let ray = Ray {
            origin: inverse.transform_point3(world_ray.origin),
            direction: inverse.transform_vector3(world_ray.direction),
        };
        let hit = cindral_tools::pick_section(
            self.history.model(),
            ray,
            view,
            cursor,
            10.,
            self.local_section().as_ref(),
        );
        let scene_hit = if !self.modify.active() || !self.rotate.reference_set {
            self.scene_hit(world_ray, view_world, cursor)
                .filter(|h| h.path.starts_with(self.history.context()))
        } else {
            None
        };
        let surface_normal = scene_hit.as_ref().and_then(|h| {
            let cindral_tools::Hit::Face(id) = h.hit else {
                return None;
            };
            let occurrence = self.scene.occurrences().iter().find(|o| o.path == h.path)?;
            let face = self
                .history
                .definition_model(h.definition)?
                .faces()
                .get(&id)?;
            Some(
                occurrence
                    .world
                    .inverse()
                    .transpose()
                    .transform_vector3(face.plane.normal)
                    .normalize(),
            )
        });
        let normal = if self.modify.active() {
            self.modify.axis()
        } else if let Some(n) = surface_normal {
            n
        } else {
            match hit {
                Some(cindral_tools::Hit::Face(id)) => inverse
                    .transpose()
                    .transform_vector3(self.history.model().faces()[&id].plane.normal)
                    .normalize(),
                _ => DVec3::Z,
            }
        };
        if !ctx.wants_keyboard_input() && self.measurements.text.is_empty() && !self.modify.active()
        {
            ctx.input(|i| {
                for (key, axis) in [
                    (shortcuts::Action::AxisZ, DVec3::Z),
                    (shortcuts::Action::AxisX, DVec3::X),
                    (shortcuts::Action::AxisY, DVec3::Y),
                ] {
                    if shortcuts::pressed(i, key) {
                        self.rotate.lock = if self.rotate.lock.is_some_and(|n| {
                            n.abs_diff_eq(axis, 1e-9) || n.abs_diff_eq(-axis, 1e-9)
                        }) {
                            None
                        } else {
                            Some(axis)
                        };
                    }
                }
                if shortcuts::pressed(i, shortcuts::Action::AxisClear) {
                    self.rotate.lock = if self.rotate.lock.is_some() {
                        None
                    } else {
                        self.rotate.normal.or(Some(normal))
                    };
                }
                if i.modifiers.shift {
                    if self.rotate.held.is_none() {
                        self.rotate.held = self.rotate.normal.or(Some(normal));
                    }
                } else {
                    self.rotate.held = None;
                }
            });
        }
        let normal = self
            .rotate
            .lock
            .or(self.rotate.held)
            .filter(|_| !self.modify.active())
            .unwrap_or(normal);
        let normal = if !self.modify.active() && normal.dot(world_ray.direction) > 0. {
            -normal
        } else {
            normal
        };
        if !self.modify.active() {
            self.rotate.inferred = self.rotate.lock.is_none()
                && (surface_normal.is_some() || matches!(hit, Some(cindral_tools::Hit::Face(_))));
        }
        let center = self.modify.anchor().unwrap_or(DVec3::ZERO);
        let local_normal = context.transpose().transform_vector3(normal).normalize();
        let mut query = Query::new(cursor, ray, view);
        query.plane = DrawingPlane {
            origin: inverse.transform_point3(center),
            normal: local_normal,
        };
        if self.modify.active() && !self.rotate.reference_set {
            if let Some(cindral_tools::Hit::Edge(id)) = hit {
                if let Some(edge) = self.history.model().edges().get(&id) {
                    let a = self.history.model().vertices()[&edge.vertices[0]].position;
                    let b = self.history.model().vertices()[&edge.vertices[1]].position;
                    self.rotate.inference_direction = Some(context.transform_vector3(b - a));
                }
            }
            if let Some(hit) = &scene_hit {
                if let cindral_tools::Hit::Edge(id) = hit.hit {
                    if let (Some(model), Some(occurrence)) = (
                        self.history.definition_model(hit.definition),
                        self.scene.occurrences().iter().find(|o| o.path == hit.path),
                    ) {
                        if let Some(edge) = model.edges().get(&id) {
                            self.rotate.inference_direction =
                                Some(occurrence.world.transform_vector3(
                                    model.vertices()[&edge.vertices[1]].position
                                        - model.vertices()[&edge.vertices[0]].position,
                                ));
                        }
                    }
                }
            }
        }
        query.previous_direction = self
            .rotate
            .inference_direction
            .map(|d| inverse.transform_vector3(d));
        query.anchor = self.modify.anchor().map(|p| inverse.transform_point3(p));
        query.section = self.local_section();
        self.snap = self.engine.infer(self.history.model(), &query);
        if let Some(hit) = scene_hit
            .as_ref()
            .filter(|h| h.path != self.history.context())
        {
            if let Some(occurrence) = self.scene.occurrences().iter().find(|o| o.path == hit.path) {
                let world = occurrence.world;
                let inv = world.inverse();
                let mut child = Query::new(
                    cursor,
                    Ray {
                        origin: inv.transform_point3(world_ray.origin),
                        direction: inv.transform_vector3(world_ray.direction),
                    },
                    View {
                        matrix: view_world.matrix * world,
                        size: view_world.size,
                    },
                );
                child.plane = DrawingPlane {
                    origin: inv.transform_point3(center),
                    normal: world.transpose().transform_vector3(normal).normalize(),
                };
                child.anchor = self.modify.anchor().map(|p| inv.transform_point3(p));
                child.previous_direction = self
                    .rotate
                    .inference_direction
                    .map(|d| inv.transform_vector3(d));
                child.section = self
                    .presentation
                    .sections
                    .cut()
                    .and_then(|p| p.in_frame(world));
                if let Some(model) = self.history.definition_model(hit.definition) {
                    if let Some(mut snap) = self.engine.infer(model, &child) {
                        snap.position =
                            inverse.transform_point3(world.transform_point3(snap.position));
                        snap.guide = snap.guide.map(|g| {
                            g.map(|p| inverse.transform_point3(world.transform_point3(p)))
                        });
                        snap.entity = None;
                        self.snap = Some(snap);
                    }
                }
            }
        }
        let point = self
            .snap
            .as_ref()
            .and_then(|s| {
                let own = self.modify.active()
                    && self.rotate.reference_set
                    && match s.entity {
                        Some(SnapEntity::Vertex(id)) => {
                            self.history.changes().vertices.contains(&id)
                        }
                        Some(SnapEntity::Edge(id)) => self.history.changes().edges.contains(&id),
                        Some(SnapEntity::Face(id)) => self.history.changes().faces.contains(&id),
                        None => false,
                    };
                (!own).then_some(context.transform_point3(s.position))
            })
            .or_else(|| {
                DrawingPlane {
                    origin: center,
                    normal,
                }
                .intersect(world_ray)
            })
            .or_else(|| {
                if !self.modify.active() {
                    DrawingPlane {
                        origin: context.transform_point3(self.work_plane.origin),
                        normal: world_ray.direction,
                    }
                    .intersect(world_ray)
                } else {
                    None
                }
            });
        self.rotate.point = point;
        if let Some(c) = self.modify.anchor().or(point) {
            let (u, v) = rotate_basis(normal);
            if let Some(p) = view_world.project(c) {
                let scale = [u, v]
                    .into_iter()
                    .filter_map(|d| view_world.project(c + d).map(|q| q.distance(p)))
                    .fold(0_f64, f64::max);
                self.rotate.radius = 55. / scale.max(1e-6);
            }
        }
        let clicked = response.clicked_by(PointerButton::Primary);
        if !self.modify.active() {
            self.rotate.normal = Some(normal);
            if clicked {
                let vertices = self.selection.vertex_ids(self.history.model());
                let instance = self
                    .organization
                    .selected
                    .as_ref()
                    .filter(|p| {
                        p.starts_with(self.history.context())
                            && p.len() == self.history.context().len() + 1
                    })
                    .and_then(|path| {
                        let instance = self.history.instance(path)?;
                        Some((path.clone(), instance.transform, context))
                    });
                if vertices.is_empty() && instance.is_none() {
                    self.error = Some("Select objects to rotate".into());
                    return;
                }
                if let Some(p) = point {
                    self.rotate.others = self
                        .organization
                        .multiple
                        .iter()
                        .filter(|path| {
                            Some(*path) != instance.as_ref().map(|i| &i.0)
                                && path.starts_with(self.history.context())
                                && path.len() == self.history.context().len() + 1
                        })
                        .filter_map(|path| {
                            self.history
                                .instance(path)
                                .map(|i| (path.clone(), i.transform, context))
                        })
                        .collect();
                    self.rotate.has_loose = !vertices.is_empty();
                    self.rotate.instance = instance;
                    self.rotate.reference_set = false;
                    self.rotate.inference_direction = None;
                    self.measurements.reset();
                    self.error = self
                        .modify
                        .begin_rotation(vertices, p, normal)
                        .err()
                        .map(|e| e.to_string());
                    self.modify.set_rotation_copy(self.rotate.copy);
                }
            }
            return;
        }
        if !self.rotate.reference_set {
            if clicked {
                if let Some(p) = point {
                    match self.modify.set_reference(p) {
                        Ok(()) => {
                            self.rotate.reference_set = true;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }
            return;
        }
        if let Some(p) = point {
            match self.modify.update_rotation(
                p,
                self.rotate.radius,
                self.presentation.measurements.angle_snap_degrees,
            ) {
                Ok(Some(c)) => self.rotate_preview(c),
                Err(e) => self.error = Some(e.to_string()),
                _ => {}
            }
        }
        if clicked && self.error.is_none() {
            match self.history.commit_preview() {
                Ok(true) => {
                    self.modify.finish();
                    self.measurements.completed();
                }
                Err(e) => self.error = Some(e.to_string()),
                _ => {}
            }
        }
    }
    pub(crate) fn rotate_overlay(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !self.modifying || self.modify.kind() != ModifyKind::Rotate {
            return;
        }
        let Some(center) = self.modify.anchor().or(self.rotate.point) else {
            return;
        };
        let normal = self.rotate.normal.unwrap_or(DVec3::Z);
        let (u, v) = rotate_basis(normal);
        let radius = self.rotate.radius;
        let view = View {
            matrix: self
                .camera
                .view_projection(rect.width() / rect.height().max(1.))
                .as_dmat4(),
            size: DVec2::new(rect.width() as f64, rect.height() as f64),
        };
        let screen = |p| {
            view.project(p)
                .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
        };
        let color = self.viewport_overlay(if self.rotate.inferred {
            self.viewport_ink()
        } else if normal.z.abs() > 0.999 {
            Color32::from_rgb(55, 125, 240)
        } else if normal.x.abs() > 0.999 {
            Color32::from_rgb(230, 65, 65)
        } else if normal.y.abs() > 0.999 {
            Color32::from_rgb(45, 175, 85)
        } else {
            self.viewport_ink()
        });
        let painter = ui.painter().with_clip_rect(rect);
        let at = |angle: f64, r: f64| center + (u * angle.cos() + v * angle.sin()) * r;
        for i in 0..72 {
            let a = i as f64 * std::f64::consts::TAU / 72.;
            let b = (i + 1) as f64 * std::f64::consts::TAU / 72.;
            if let (Some(a), Some(b)) = (screen(at(a, radius)), screen(at(b, radius))) {
                painter.line_segment([a, b], egui::Stroke::new(1.5_f32, color));
            }
            if i % 3 == 0 {
                if let (Some(a), Some(b)) = (
                    screen(at(a, radius)),
                    screen(at(a, radius * if i % 18 == 0 { 0.8 } else { 0.9 })),
                ) {
                    painter.line_segment([a, b], egui::Stroke::new(1_f32, color));
                }
            }
        }
        if self.modify.active() && self.rotate.reference_set {
            if let (Some(c), Some((reference, _)), Some(degrees)) = (
                screen(center),
                self.modify.rotation_reference(),
                self.modify.measurement(),
            ) {
                let angle = degrees.to_radians();
                for i in 0..48 {
                    let a = angle * i as f64 / 48.;
                    let b = angle * (i + 1) as f64 / 48.;
                    let at = |a: f64| {
                        center + (reference * a.cos() + normal.cross(reference) * a.sin()) * radius
                    };
                    if let (Some(a), Some(b)) = (screen(at(a)), screen(at(b))) {
                        painter.add(egui::Shape::convex_polygon(
                            vec![c, a, b],
                            color.gamma_multiply(0.18),
                            egui::Stroke::NONE,
                        ));
                    }
                }
            }
        }
        if let Some(c) = screen(center) {
            painter.circle_filled(c, 3., color);
            if self.modify.active() {
                let reference = if self.rotate.reference_set {
                    self.modify
                        .rotation_reference()
                        .map(|(u, _)| center + u * radius)
                } else {
                    self.rotate.point
                };
                if let Some(p) = reference.and_then(screen) {
                    painter.line_segment([c, p], egui::Stroke::new(2_f32, color));
                }
            }
        }
    }
}

fn rotate_basis(normal: DVec3) -> (DVec3, DVec3) {
    let u = normal
        .cross(if normal.z.abs() < 0.9 {
            DVec3::Z
        } else {
            DVec3::Y
        })
        .normalize();
    (u, normal.cross(u))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{DefinitionId, DefinitionKind};
    fn fixture() -> (DrafterApp, egui::Context) {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        app.selection.vertices = app.model().vertices().keys().copied().collect();
        app.camera.set_view(StandardView::Top);
        app.choose_modify(ModifyKind::Rotate);
        (app, egui::Context::default())
    }
    fn frame(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        crate::input_tests::frame(app, ctx, events, modifiers);
    }
    fn position(app: &DrafterApp, p: DVec3) -> egui::Pos2 {
        let p = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: DVec2::new(600., 400.),
        }
        .project(p)
        .unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    fn click(app: &mut DrafterApp, ctx: &egui::Context, p: DVec3) {
        let pos = position(app, p);
        frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(pos)],
            egui::Modifiers::NONE,
        );
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
    }
    fn key(app: &mut DrafterApp, ctx: &egui::Context, key: Key) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
    }
    fn typed(app: &mut DrafterApp, text: &str) {
        app.measurements.text = text.into();
        app.apply_measurement();
        assert!(app.error.is_none(), "{text}: {:?}", app.error);
    }
    #[test]
    fn three_clicks_center_only_typing_reentry_and_escape() {
        let (mut app, ctx) = fixture();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        click(&mut app, &ctx, DVec3::ZERO);
        assert!(app.modify.active());
        assert!(!app.rotate.reference_set);
        assert_eq!(app.model(), &before);
        click(&mut app, &ctx, DVec3::X);
        assert!(app.rotate.reference_set);
        assert_eq!(app.model(), &before);
        let p = position(&app, DVec3::Y * 3.);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p)],
            egui::Modifiers::NONE,
        );
        assert_ne!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        click(&mut app, &ctx, DVec3::Y * 3.);
        assert!(!app.modify.active());
        assert_eq!(app.history.undo_depth(), depth + 1);
        typed(&mut app, "45");
        let result = app.model().clone();
        typed(&mut app, "45");
        assert_eq!(app.model(), &result);
        app.history_step(false);
        assert_eq!(app.model(), &before);
        app.history_step(true);
        assert_eq!(app.model(), &result);
        app.selection.vertices = app.model().vertices().keys().copied().collect();
        app.choose_modify(ModifyKind::Rotate);
        click(&mut app, &ctx, DVec3::ZERO);
        typed(&mut app, "-22.5");
        assert_eq!(app.history.undo_depth(), depth + 2);
        for stage in 0..3 {
            app.choose_modify(ModifyKind::Rotate);
            let before = app.model().clone();
            let depth = app.history.undo_depth();
            if stage > 0 {
                click(&mut app, &ctx, DVec3::ZERO);
            }
            if stage > 1 {
                click(&mut app, &ctx, DVec3::X * 3.);
                let p = position(&app, DVec3::Y * 3.);
                frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(p)],
                    egui::Modifiers::NONE,
                );
            }
            app.measurements.text = "garbage".into();
            key(&mut app, &ctx, Key::Escape);
            assert!(app.measurements.text.is_empty());
            key(&mut app, &ctx, Key::Escape);
            assert_eq!(app.model(), &before);
            assert_eq!(app.history.undo_depth(), depth);
            assert!(!app.modify.active());
        }
    }
    #[test]
    fn locks_copy_toggle_and_no_hover_target() {
        let (mut app, ctx) = fixture();
        let pos = position(&app, DVec3::ZERO);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos)],
            egui::Modifiers::NONE,
        );
        for (k, n) in [
            (Key::ArrowUp, DVec3::Z),
            (Key::ArrowRight, DVec3::X),
            (Key::ArrowLeft, DVec3::Y),
        ] {
            key(&mut app, &ctx, k);
            assert!(app.rotate.lock.unwrap().abs_diff_eq(n, 1e-9));
            key(&mut app, &ctx, k);
            assert!(app.rotate.lock.is_none());
        }
        key(&mut app, &ctx, Key::ArrowDown);
        assert!(app.rotate.lock.is_some());
        key(&mut app, &ctx, Key::ArrowDown);
        assert!(app.rotate.lock.is_none());
        let copy = if cfg!(target_os = "macos") {
            egui::Modifiers {
                alt: true,
                ..Default::default()
            }
        } else {
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        };
        frame(&mut app, &ctx, vec![], copy);
        assert!(app.rotate.copy);
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        frame(&mut app, &ctx, vec![], copy);
        assert!(!app.rotate.copy);
        app.selection.clear();
        click(&mut app, &ctx, DVec3::ZERO);
        assert!(!app.modify.active());
        assert_eq!(app.rotate_hint(), "Select objects to rotate");
        key(&mut app, &ctx, Key::Space);
        assert_eq!(app.modify.kind(), ModifyKind::Select);
        key(&mut app, &ctx, Key::Q);
        assert_eq!(app.modify.kind(), ModifyKind::Rotate);
        key(&mut app, &ctx, Key::L);
        assert!(!app.modifying);
    }
    #[test]
    fn mixed_loose_and_mirrored_component_use_one_history_entry() {
        let (mut app, _) = fixture();
        let path = app.create_container(DefinitionKind::Component).unwrap();
        let definition = app.history.instance(&path).unwrap().definition;
        let mirror = DMat4::from_scale(DVec3::new(-2., 3., 1.));
        app.history
            .organization(Org::Transform {
                path: path.clone(),
                transform: mirror,
            })
            .unwrap();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::X * 5.,
                DVec3::X * 6.,
                DVec3::X * 6. + DVec3::Y,
                DVec3::X * 5. + DVec3::Y,
            ],
            closed: true,
        })
        .unwrap();
        let second = DMat4::from_translation(DVec3::X * 10.);
        app.history
            .organization(Org::AddInstance {
                definition,
                name: "second".into(),
                transform: second,
            })
            .unwrap();
        let second_path = vec![*app
            .history
            .instances(DefinitionId(0))
            .unwrap()
            .keys()
            .next_back()
            .unwrap()];
        let before = app.history.to_data();
        let def = app.history.definition_model(definition).unwrap().clone();
        let depth = app.history.undo_depth();
        app.choose_modify(ModifyKind::Rotate);
        app.rotate.instance = Some((path.clone(), mirror, DMat4::IDENTITY));
        app.rotate.others = vec![(second_path.clone(), second, DMat4::IDENTITY)];
        app.rotate.has_loose = true;
        app.modify
            .begin_rotation(
                app.model().vertices().keys().copied().collect(),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        typed(&mut app, "30");
        typed(&mut app, "60");
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!(app
            .history
            .instance(&second_path)
            .unwrap()
            .transform
            .abs_diff_eq(
                cindral_tools::rotation::matrix(DVec3::ZERO, DVec3::Z, 60_f64.to_radians())
                    * second,
                1e-10
            ));
        assert_eq!(app.history.definition_model(definition).unwrap(), &def);
        let expected =
            cindral_tools::rotation::matrix(DVec3::ZERO, DVec3::Z, 60_f64.to_radians()) * mirror;
        assert!(app
            .history
            .instance(&path)
            .unwrap()
            .transform
            .abs_diff_eq(expected, 1e-10));
        app.history.undo().unwrap();
        assert_eq!(app.history.to_data(), before);
        app.history.redo().unwrap();
        app.history.model().validate().unwrap();
        assert!(app
            .history
            .instances(DefinitionId(0))
            .unwrap()
            .contains_key(path.last().unwrap()));
    }
    #[test]
    fn nested_scaled_context_uses_world_rotation_and_preserves_other_definitions() {
        let (mut app, _) = fixture();
        let parent = app.create_container(DefinitionKind::Group).unwrap();
        let placement = DMat4::from_rotation_x(0.4) * DMat4::from_scale(DVec3::new(-2., 3., 0.5));
        app.history
            .organization(Org::Transform {
                path: parent.clone(),
                transform: placement,
            })
            .unwrap();
        app.enter_context(parent.clone()).unwrap();
        app.selection.vertices = app.model().vertices().keys().copied().collect();
        let child = app.create_container(DefinitionKind::Component).unwrap();
        app.refresh_mesh();
        let def = app.history.instance(&child).unwrap().definition;
        let before_def = app.history.definition_model(def).unwrap().clone();
        let root = app
            .history
            .definition_model(DefinitionId(0))
            .unwrap()
            .clone();
        app.choose_modify(ModifyKind::Rotate);
        app.rotate.instance = Some((child.clone(), DMat4::IDENTITY, placement));
        let center = DVec3::new(2., 4., 3.);
        let normal = DVec3::new(1., 2., 3.).normalize();
        app.modify.begin_rotation(vec![], center, normal).unwrap();
        typed(&mut app, "37");
        let expected = placement.inverse()
            * cindral_tools::rotation::matrix(center, normal, 37_f64.to_radians())
            * placement;
        assert!(app
            .history
            .instance(&child)
            .unwrap()
            .transform
            .abs_diff_eq(expected, 1e-9));
        assert_eq!(app.history.definition_model(def).unwrap(), &before_def);
        assert_eq!(
            app.history.definition_model(DefinitionId(0)).unwrap(),
            &root
        );
        app.history.undo().unwrap();
        assert_eq!(
            app.history.instance(&child).unwrap().transform,
            DMat4::IDENTITY
        );
        // Loose geometry inside the same mirrored/nonuniform context uses conjugation too.
        app.enter_context(child).unwrap();
        let before = app.model().clone();
        let world = app.history.world_transform();
        app.choose_modify(ModifyKind::Rotate);
        app.modify
            .begin_rotation(before.vertices().keys().copied().collect(), center, normal)
            .unwrap();
        typed(&mut app, "37");
        for (id, v) in before.vertices() {
            let expected = cindral_tools::rotation::matrix(center, normal, 37_f64.to_radians())
                .transform_point3(world.transform_point3(v.position));
            assert!(
                world
                    .transform_point3(app.model().vertices()[id].position)
                    .distance(expected)
                    < 1e-8
            );
        }
        app.model().validate().unwrap();
        app.history.undo().unwrap();
        assert_eq!(app.model(), &before);
    }

    #[test]
    fn pointer_copy_group_arrays_and_invalid_reentry_are_atomic() {
        let (mut app, ctx) = fixture();
        let path = app.create_container(DefinitionKind::Group).unwrap();
        let before = app.history.to_data();
        let depth = app.history.undo_depth();
        app.choose_modify(ModifyKind::Rotate);
        let pos = position(&app, DVec3::new(-5., 0., 0.));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos)],
            egui::Modifiers::NONE,
        );
        let copy = if cfg!(target_os = "macos") {
            egui::Modifiers {
                alt: true,
                ..Default::default()
            }
        } else {
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        };
        frame(&mut app, &ctx, vec![], copy);
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert!(app.rotate.copy);
        click(&mut app, &ctx, DVec3::new(-5., 0., 0.));
        click(&mut app, &ctx, DVec3::new(-4., 0., 0.));
        typed(&mut app, "30");
        for (text, count) in [("3x", 4), ("/3", 4), ("*2", 3)] {
            typed(&mut app, text);
            assert_eq!(app.history.instances(DefinitionId(0)).unwrap().len(), count);
            assert_eq!(app.history.undo_depth(), depth + 1);
        }
        let after = app.history.to_data();
        app.measurements.text = "1:0".into();
        app.apply_measurement();
        assert!(app.measurements.invalid);
        assert_eq!(app.history.to_data(), after);
        app.history.undo().unwrap();
        let mut restored = app.history.to_data();
        restored.next_id = before.next_id;
        assert_eq!(restored, before);
        app.history.redo().unwrap();
        assert_eq!(app.history.to_data(), after);
        assert!(app.history.instance(&path).is_some());
    }
    #[test]
    fn cached_whole_rotate_updates_placements_without_mesh_rebuild() {
        let (mut app, _) = fixture();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        app.pending = None;
        app.modify
            .begin_rotation(
                before.vertices().keys().copied().collect(),
                DVec3::ZERO,
                DVec3::Y,
            )
            .unwrap();
        for text in ["15", "30", "45"] {
            let command = app.modify.measure(text).unwrap().unwrap();
            app.rotate_preview(command);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(app.pending.as_ref().unwrap().geometry.is_empty());
            assert!(app.pending.as_ref().unwrap().batches.is_some());
            assert_eq!(app.history.undo_depth(), depth);
            app.model().validate().unwrap();
            app.pending = None;
        }
        app.cancel_modify();
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(app.pending.as_ref().unwrap().geometry.is_empty());
    }

    #[test]
    fn zero_multi_preview_preserves_prior_undo_and_destinations_are_cached() {
        let (mut app, _) = fixture();
        let first = app.create_container(DefinitionKind::Group).unwrap();
        app.history
            .organization(Org::Duplicate {
                path: first.clone(),
                transform: DMat4::from_translation(DVec3::X * 5.),
            })
            .unwrap();
        let second = vec![*app
            .history
            .instances(DefinitionId(0))
            .unwrap()
            .keys()
            .next_back()
            .unwrap()];
        let before = app.history.to_data();
        let depth = app.history.undo_depth();
        let original = vec![
            (
                first.clone(),
                vec![app.history.instance(&first).unwrap().transform],
            ),
            (
                second.clone(),
                vec![app.history.instance(&second).unwrap().transform],
            ),
        ];
        assert!(!app
            .history
            .preview_placement_set(Org::TransformSet {
                placements: original.clone(),
                copy: false
            })
            .unwrap());
        assert_eq!(app.history.undo_depth(), depth);
        let command = |a: f64| Org::TransformSet {
            placements: original
                .iter()
                .map(|(p, m)| (p.clone(), vec![DMat4::from_rotation_z(a) * m[0]]))
                .collect(),
            copy: true,
        };
        app.history.preview_placement_set(command(0.2)).unwrap();
        let ids = app
            .history
            .instances(DefinitionId(0))
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        app.history.preview_placement_set(command(0.4)).unwrap();
        assert_eq!(
            app.history
                .instances(DefinitionId(0))
                .unwrap()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(app.history.undo_depth(), depth);
        app.history.commit_preview().unwrap();
        assert_eq!(app.history.undo_depth(), depth + 1);
        app.history.undo().unwrap();
        let mut restored = app.history.to_data();
        restored.next_id = before.next_id;
        assert_eq!(restored, before);
    }
}
