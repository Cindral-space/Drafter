//! Modify input routing and screen overlays; kernel previews use ordinary dirty deltas.
use super::*;
use cindral_tools::{Hit, ModifyKind};
impl DrafterApp {
    pub(crate) fn choose_modify(&mut self, kind: ModifyKind) {
        self.cancel_section();
        self.cancel_modify();
        self.modifying = true;
        self.modify.set_kind(kind);
        if kind == ModifyKind::Rotate {
            self.axis_lock = None;
        }
        self.modify.set_color(self.paint_color);
        self.tool.cancel();
        self.measurements.reset();
        self.draw_press = None;
        self.draw_drag = None;
    }
    pub(crate) fn cancel_modify(&mut self) {
        self.tape_resize = None;
        self.tape_target = None;
        self.tape_resized = false;
        self.organization.drag = None;
        self.organization.completed = None;
        match self.history.cancel_preview() {
            Ok(true) => self.refresh_mesh(),
            Err(e) => self.error = Some(e.to_string()),
            _ => {}
        }
        self.modify.cancel();
        self.rotate = Default::default();
        self.modify_press = None;
        self.selection_start = None;
    }
    fn modify_preview(&mut self, command: Command) {
        let _phase = cindral_core::profile::Scope::new(3);
        if let Err(e) = self.history.preview(command) {
            self.error = Some(e.to_string());
        } else {
            self.error = None;
        }
        self.refresh_mesh();
    }
    fn finish_modify(&mut self) {
        self.modify.finish();
        self.modify_press = None;
        match self.history.commit_preview() {
            Ok(true) => {
                self.error = None;
                self.measurements.completed();
            }
            Ok(false) => {
                if self.error.is_none() {
                    self.error = Some(
                        "No change: tool movement is zero or below the geometry tolerance".into(),
                    );
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.selection
            .vertices
            .retain(|v| self.history.model().vertices().contains_key(v));
        self.selection
            .edges
            .retain(|e| self.history.model().edges().contains_key(e));
        self.selection
            .faces
            .retain(|f| self.history.model().faces().contains_key(f));
    }
    pub(crate) fn modify_measurement(&mut self) {
        let before = self.modify.clone();
        let replacing = self.modify.editing_completed();
        match self
            .modify
            .measure_with_settings(&self.measurements.text, self.presentation.measurements)
        {
            Ok(Some(command)) => {
                let result = if replacing {
                    self.history.replace_last(command)
                } else {
                    self.history
                        .preview_atomic(command)
                        .and_then(|_| self.history.commit_preview())
                };
                match result {
                    Ok(true) => {
                        self.modify.finish();
                        self.modify_press = None;
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
            Ok(None) => {
                if self.modify.kind() == ModifyKind::TapeMeasure {
                    self.last_tape = self.modify.measurement();
                    self.modify.finish();
                    self.measurements.completed();
                    self.error = None;
                }
            }
            Err(e) => {
                self.modify = before;
                self.error = Some(e.to_string());
            }
        }
        self.measurements.invalid = self.error.is_some();
    }

    pub(crate) fn modify_viewport_input(&mut self, response: &egui::Response) {
        let rect = response.rect;
        let view = View {
            matrix: self
                .camera
                .view_projection(rect.width() / rect.height().max(1.))
                .as_dmat4()
                * self.history.world_transform(),
            size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
        };
        let keyboard_free = !response.ctx.wants_keyboard_input();
        let escape = response.ctx.input(|i| i.key_pressed(Key::Escape));
        if keyboard_free && escape {
            self.cancel_modify();
            self.measurements.reset();
            self.error = None;
            return;
        }
        if keyboard_free {
            let undo = response
                .ctx
                .input(|i| i.modifiers.command && i.key_pressed(Key::Z));
            let redo = response.ctx.input(|i| {
                i.modifiers.command
                    && (i.key_pressed(Key::Y) || i.modifiers.shift && i.key_pressed(Key::Z))
            });
            if undo || redo {
                self.cancel_modify();
                self.history_step(redo);
                return;
            }
            response.ctx.input(|i| {
                for (key, axis) in [
                    (Key::ArrowRight, Axis::X),
                    (Key::ArrowLeft, Axis::Y),
                    (Key::ArrowUp, Axis::Z),
                ] {
                    if i.key_pressed(key) {
                        self.axis_lock = Some(axis);
                    }
                }
                if i.key_pressed(Key::ArrowDown) {
                    self.axis_lock = None;
                }
            });
        }
        let Some(position) = response.ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let cursor = glam::DVec2::new(
            (position.x - rect.left()) as f64,
            (position.y - rect.top()) as f64,
        );
        let ndc = Vec2::new(
            2. * cursor.x as f32 / rect.width() - 1.,
            1. - 2. * cursor.y as f32 / rect.height(),
        );
        let (origin, direction) = self.camera.ray(ndc, rect.width() / rect.height());
        let ray = Ray {
            origin: self
                .history
                .world_transform()
                .inverse()
                .transform_point3(origin.as_dvec3()),
            direction: self
                .history
                .world_transform()
                .inverse()
                .transform_vector3(direction.as_dvec3()),
        };
        let pick = if self.modify.kind() == ModifyKind::Eraser {
            cindral_tools::pick_eraser_section
        } else {
            cindral_tools::pick_section
        };
        let hit = pick(
            self.history.model(),
            ray,
            view,
            cursor,
            10.,
            self.local_section().as_ref(),
        );
        let mut query = Query::new(cursor, ray, view);
        query.plane = DrawingPlane {
            origin: self.modify.anchor().unwrap_or(self.work_plane.origin),
            normal: if self.modify.active()
                && matches!(self.modify.kind(), ModifyKind::Rotate | ModifyKind::Scale)
            {
                self.drag_axis
            } else {
                self.work_plane.normal
            },
        };
        query.section = self.local_section();
        query.anchor = self.modify.anchor();
        query.axis_lock = self.axis_lock;
        self.snap = self.engine.infer(self.history.model(), &query);
        let mut point = self
            .snap
            .as_ref()
            .and_then(|s| {
                let own = self.modify.active()
                    && match s.entity {
                        Some(SnapEntity::Vertex(id)) => {
                            self.history.changes().vertices.contains(&id)
                        }
                        Some(SnapEntity::Edge(id)) => self.history.changes().edges.contains(&id),
                        Some(SnapEntity::Face(id)) => self.history.changes().faces.contains(&id),
                        None => false,
                    };
                (!own).then_some(s.position)
            })
            .or_else(|| query.plane.intersect(ray));
        let pressed = response
            .ctx
            .input(|i| i.pointer.button_pressed(PointerButton::Primary));
        let released = response
            .ctx
            .input(|i| i.pointer.button_released(PointerButton::Primary));
        let down = response
            .ctx
            .input(|i| i.pointer.button_down(PointerButton::Primary));
        let additive = response.ctx.input(|i| i.modifiers.shift);
        if self.modify.kind() == ModifyKind::Select {
            if pressed {
                self.selection_start = Some(cursor);
            }
            if released {
                if let Some(start) = self.selection_start.take() {
                    if start.distance(cursor) > 4. {
                        self.selection
                            .region(self.history.model(), view, start, cursor, additive);
                    } else if let Some(hit) = hit {
                        if response.triple_clicked() {
                            self.selection
                                .connected(self.history.model(), hit, additive);
                        } else {
                            self.selection.select(self.history.model(), hit, additive);
                        }
                    } else if !additive {
                        self.selection.clear();
                    }
                }
            }
            return;
        }
        if pressed && self.modify.kind() == ModifyKind::Eraser && hit.is_none() {
            return;
        }
        if pressed && !self.modify.active() {
            self.modify.cancel();
            self.measurements.reset();
            self.tape_resize = None;
            self.tape_target = None;
            self.tape_resized = false;
            if point.is_none() {
                self.error = Some(
                    "Cannot start tool: pointer ray does not intersect the editing plane or a face"
                        .into(),
                );
                return;
            }
            self.modify_press = Some(cursor);
            if let Some(p) = point {
                let face = match hit {
                    Some(Hit::Face(id)) => Some(id),
                    _ => self.selection.faces.first().copied(),
                };
                let axis = face
                    .and_then(|id| self.history.model().faces().get(&id))
                    .map_or(self.work_plane.normal, |f| f.plane.normal);
                self.drag_axis = axis;
                let pivot = if matches!(self.modify.kind(), ModifyKind::Rotate | ModifyKind::Scale)
                {
                    let mut selection = self.selection.clone();
                    if let Some(hit) = hit {
                        selection.select(self.history.model(), hit, true);
                    }
                    let points: Vec<_> = selection
                        .vertex_ids(self.history.model())
                        .into_iter()
                        .filter_map(|id| {
                            self.history.model().vertices().get(&id).map(|v| v.position)
                        })
                        .collect();
                    if points.is_empty() {
                        p
                    } else {
                        points.iter().copied().sum::<DVec3>() / points.len() as f64
                    }
                } else {
                    p
                };
                if let Err(e) =
                    self.modify
                        .begin(self.history.model(), &self.selection, hit, pivot, axis)
                {
                    self.error = Some(e.to_string());
                    return;
                }
                self.drag_axis = self.modify.axis();
                if matches!(self.modify.kind(), ModifyKind::Rotate | ModifyKind::Scale) {
                    let _ = self.modify.set_reference(p);
                }
                if matches!(
                    self.modify.kind(),
                    ModifyKind::Eraser | ModifyKind::PaintBucket
                ) {
                    match self.modify.update(p) {
                        Ok(Some(c)) => {
                            self.commit(c);
                        }
                        Err(e) => self.error = Some(e.to_string()),
                        _ => {}
                    }
                    self.modify.finish();
                    return;
                }
            }
        }
        if self.modify.active() {
            if matches!(
                self.modify.kind(),
                ModifyKind::PushPull | ModifyKind::FollowMe
            ) {
                if let Some(anchor) = self.modify.anchor() {
                    let axis = self.drag_axis;
                    point = axis_ray_point(ray, anchor, axis).or(point);
                }
            }
            if down || released || !pressed {
                if let Some(p) = point {
                    match self.modify.update(p) {
                        Ok(Some(c)) => self.modify_preview(c),
                        Err(e) => self.error = Some(e.to_string()),
                        _ => {}
                    }
                }
            }
            if released {
                if self.modify.kind() == ModifyKind::TapeMeasure
                    && self
                        .modify_press
                        .is_some_and(|start| start.distance(cursor) > 3.)
                    && self
                        .modify
                        .measurement()
                        .is_some_and(|d| d > cindral_core::TOLERANCE)
                {
                    self.last_tape = self.modify.measurement();
                    if let Err(e) = self.modify.add_guide() {
                        self.error = Some(e.to_string());
                    }
                    self.modify.finish();
                    self.measurements.completed();
                } else if self.error.is_none()
                    && self
                        .modify_press
                        .is_some_and(|start| start.distance(cursor) > 3.)
                {
                    self.finish_modify();
                }
            }
        }
    }
    pub(crate) fn modify_overlay(&self, ui: &egui::Ui, rect: egui::Rect, view: View) {
        let screen = |p| {
            view.project(p)
                .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
        };
        let stroke = egui::Stroke::new(
            3_f32,
            self.viewport_overlay(Color32::from_rgb(255, 184, 68)),
        );
        let model = self.history.model();
        let draw_edge = |a, b| {
            if let (Some(a), Some(b)) = (screen(a), screen(b)) {
                ui.painter().line_segment([a, b], stroke);
            }
        };
        for id in &self.selection.edges {
            if let Some(e) = model.edges().get(id) {
                draw_edge(
                    model.vertices()[&e.vertices[0]].position,
                    model.vertices()[&e.vertices[1]].position,
                );
            }
        }
        for id in &self.selection.faces {
            if let Some(f) = model.faces().get(id) {
                for ring in std::iter::once(&f.outer).chain(&f.holes) {
                    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        draw_edge(model.vertices()[a].position, model.vertices()[b].position);
                    }
                }
            }
        }
        for id in &self.selection.vertices {
            if let Some(v) = model.vertices().get(id) {
                if let Some(p) = screen(v.position) {
                    ui.painter().circle_filled(p, 4_f32, stroke.color);
                }
            }
        }
        if let Some(start) = self.selection_start {
            if let Some(cursor) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                let origin = rect.min + egui::vec2(start.x as f32, start.y as f32);
                let selection = egui::Rect::from_two_pos(origin, cursor);
                ui.painter().rect_stroke(
                    selection,
                    0_f32,
                    egui::Stroke::new(
                        1_f32,
                        if cursor.x >= origin.x {
                            self.viewport_overlay(Color32::LIGHT_BLUE)
                        } else {
                            self.viewport_overlay(Color32::LIGHT_GREEN)
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
            }
        }
        for guide in self.modify.guides() {
            if let Some((a, b)) = clip_guide(view, guide.origin, guide.direction) {
                let a = rect.min + egui::vec2(a.x as f32, a.y as f32);
                let b = rect.min + egui::vec2(b.x as f32, b.y as f32);
                ui.painter().line_segment(
                    [a, b],
                    egui::Stroke::new(
                        1_f32,
                        self.viewport_overlay(Color32::from_rgb(85, 185, 210)),
                    ),
                );
            }
        }
        if let Some(distance) = self.modify.measurement().or_else(|| {
            (self.modify.kind() == ModifyKind::TapeMeasure)
                .then_some(self.last_tape)
                .flatten()
        }) {
            ui.painter().text(
                rect.left_top() + egui::vec2(12., 12.),
                egui::Align2::LEFT_TOP,
                format!(
                    "Measured: {distance:.4} {}",
                    match self.modify.kind() {
                        ModifyKind::Rotate => "deg",
                        ModifyKind::Scale => "x",
                        _ => "m",
                    }
                ),
                egui::FontId::monospace(15.),
                self.viewport_ink(),
            );
        }
    }
}
fn axis_ray_point(ray: Ray, anchor: DVec3, axis: DVec3) -> Option<DVec3> {
    let direction = ray.direction.try_normalize()?;
    let axis = axis.try_normalize()?;
    let dot = direction.dot(axis);
    let denominator = 1. - dot * dot;
    if denominator < 1e-10 {
        return None;
    }
    let offset = ray.origin - anchor;
    Some(anchor + axis * ((offset.dot(axis) - offset.dot(direction) * dot) / denominator))
}

fn clip_guide(view: View, origin: DVec3, direction: DVec3) -> Option<(glam::DVec2, glam::DVec2)> {
    let a = view.matrix * (origin - direction * 1e6).extend(1.);
    let b = view.matrix * (origin + direction * 1e6).extend(1.);
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let mut low = 0_f64;
    let mut high = 1_f64;
    for (start, end) in [
        (a.w + a.x, b.w + b.x),
        (a.w - a.x, b.w - b.x),
        (a.w + a.y, b.w + b.y),
        (a.w - a.y, b.w - b.y),
        (a.z, b.z),
        (a.w - a.z, b.w - b.z),
        (a.w - 1e-9, b.w - 1e-9),
    ] {
        if start < 0. && end < 0. {
            return None;
        }
        if (start < 0.) != (end < 0.) {
            let t = -start / (end - start);
            if start < 0. {
                low = low.max(t);
            } else {
                high = high.min(t);
            }
        }
    }
    if low > high {
        return None;
    }
    let project = |t| {
        let p = a.lerp(b, t);
        let n = p.truncate() / p.w;
        glam::DVec2::new(
            (n.x + 1.) * 0.5 * view.size.x,
            (1. - n.y) * 0.5 * view.size.y,
        )
    };
    Some((project(low), project(high)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rectangle(app: &mut DrafterApp) {
        let mut tool = ToolSession::new(ToolKind::Rectangle);
        tool.click(DVec3::ZERO).unwrap();
        let command = tool.measure("2m,2m").unwrap().unwrap();
        app.execute(command).unwrap();
    }
    fn position(app: &DrafterApp, p: DVec3) -> egui::Pos2 {
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let p = view.project(p).unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    fn frame(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        crate::input_tests::frame(app, ctx, events, egui::Modifiers::NONE);
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn scripted_draw_pushpull_move_two_undos_and_redo() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        let drawn = app.model().clone();
        let face = *app.model().faces().keys().next().unwrap();
        app.execute(Command::PushPull { face, distance: 2. })
            .unwrap();
        let solid = app.model().clone();
        assert_eq!(solid.faces().len(), 6);
        let vertices = app.model().vertices().keys().copied().collect();
        app.execute(Command::Transform {
            vertices,
            matrix: glam::DMat4::from_translation(DVec3::X * 3.),
            copy: false,
        })
        .unwrap();
        assert_eq!(app.model().bounds().unwrap().0.x, 3.);
        app.history_step(false);
        assert_eq!(app.model(), &solid);
        app.history_step(false);
        assert_eq!(app.model(), &drawn);
        app.history_step(true);
        assert_eq!(app.model(), &solid);
    }
    #[test]
    fn live_move_drag_commits_once_and_escape_restores_geometry_and_mesh() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        app.camera.set_view(StandardView::Top);
        app.camera.projection = Projection::Orthographic;
        let face = *app.model().faces().keys().next().unwrap();
        let mut selection = Selection::default();
        selection.connected(app.model(), Hit::Face(face), false);
        app.selection = selection;
        app.choose_modify(ModifyKind::Move);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let start = position(&app, DVec3::new(1., 1., 0.));
        let end = position(&app, DVec3::new(4., 1., 0.));
        let depth = app.history.undo_depth();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(app.model().bounds().unwrap().0.x > 2.9);
        frame(&mut app, &ctx, vec![button(end, false)]);
        assert_eq!(app.history.undo_depth(), depth + 1);
        let committed = app.model().clone();
        let next = position(&app, DVec3::new(7., 1., 0.));
        frame(&mut app, &ctx, vec![button(end, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(next)]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.model(), &committed);
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!(!app.modify.active());
        assert!(app.pending.is_some());
    }
    #[test]
    fn click_then_typed_pushpull_and_typed_rotation_scale_commit_once_each() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        let ctx = egui::Context::default();
        app.choose_modify(ModifyKind::PushPull);
        frame(&mut app, &ctx, vec![]);
        let p = position(&app, DVec3::new(1., 1., 0.));
        let depth = app.history.undo_depth();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p), button(p, true)],
        );
        frame(&mut app, &ctx, vec![button(p, false)]);
        assert!(app.modify.active());
        assert_eq!(app.history.undo_depth(), depth);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("2m".into()),
                egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(app.model().faces().len(), 6);
        assert!((app.model().bounds().unwrap().1.z - 2.).abs() < 1e-4);
        assert_eq!(app.history.undo_depth(), depth + 1);
        for (kind, text) in [(ModifyKind::Rotate, "90deg"), (ModifyKind::Scale, "2x")] {
            app.choose_modify(kind);
            let selection = Selection {
                vertices: app.model().vertices().keys().copied().collect(),
                ..Selection::default()
            };
            app.modify
                .begin(app.history.model(), &selection, None, DVec3::ZERO, DVec3::Z)
                .unwrap();
            app.measurements.text = text.into();
            let depth = app.history.undo_depth();
            app.apply_measurement();
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.history.undo_depth(), depth + 1);
        }
        let (min, max) = app.model().bounds().unwrap();
        assert!((min.x + 4.).abs() < 1e-4);
        assert!((max.z - 4.).abs() < 1e-4);
    }
    #[test]
    fn triple_click_selection_and_pending_tape_resize_preserve_history() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        let ctx = egui::Context::default();
        app.choose_modify(ModifyKind::Select);
        frame(&mut app, &ctx, vec![]);
        let p = position(&app, DVec3::new(1., 1., 0.));
        for _ in 0..3 {
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(p), button(p, true)],
            );
            frame(&mut app, &ctx, vec![button(p, false)]);
        }
        assert_eq!(app.selection.faces.len(), 1);
        assert_eq!(app.selection.edges.len(), 4);
        assert_eq!(app.selection.vertices.len(), 4);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        app.choose_modify(ModifyKind::TapeMeasure);
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.update(DVec3::X * 3.).unwrap();
        app.measurements.text = "5m".into();
        app.apply_measurement();
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(app.tape_resize.is_some());
        assert!(app.modify.guides().is_empty());
        app.confirm_tape_resize(false);
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
    }
    #[test]
    fn guide_lines_clip_across_the_view_even_with_an_endpoint_behind_camera() {
        let camera = Camera::default();
        let view = View {
            matrix: camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let (a, b) = clip_guide(view, DVec3::ZERO, DVec3::X).unwrap();
        for p in [a, b] {
            assert!(p.is_finite());
            assert!(p.x >= -1e-4 && p.x <= 600.0001 && p.y >= -1e-4 && p.y <= 400.0001);
        }
        assert!(a.distance(b) > 100.);
    }

    #[test]
    fn push_pull_parity_drag_and_click_move_click() {
        for drag in [false, true] {
            let mut app = DrafterApp::default();
            rectangle(&mut app);
            app.choose_modify(ModifyKind::PushPull);
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let start = position(&app, DVec3::new(1., 1., 0.));
            let end = position(&app, DVec3::new(1., 1., 1.));
            let depth = app.history.undo_depth();
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            if !drag {
                frame(&mut app, &ctx, vec![button(start, false)]);
                assert!(app.modify.active());
            }
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
            assert_eq!(app.history.undo_depth(), depth);
            if !drag {
                frame(&mut app, &ctx, vec![button(end, true)]);
            }
            frame(&mut app, &ctx, vec![button(end, false)]);
            assert!(!app.modify.active());
            assert_eq!(app.history.undo_depth(), depth + 1);
            assert!((app.model().bounds().unwrap().1.z - 1.).abs() < 1e-4);
            app.model().validate().unwrap();
        }
    }

    fn parity_key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn push_pull_parity_escape_and_history_reset() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        let ctx = egui::Context::default();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
        );
        app.choose_modify(ModifyKind::PushPull);
        let face = *app.model().faces().keys().next().unwrap();
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        let command = app.modify.measure("150cm").unwrap().unwrap();
        app.modify_preview(command);
        assert_ne!(app.model(), &before);
        frame(
            &mut app,
            &ctx,
            vec![parity_key(Key::Escape, egui::Modifiers::NONE)],
        );
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(!app.modify.active());
        app.model().validate().unwrap();

        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.measurements.text = "2m".into();
        app.apply_measurement();
        let solid = app.model().clone();
        assert_eq!(app.history.undo_depth(), depth + 1);
        let modifiers = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        crate::input_tests::frame(
            &mut app,
            &ctx,
            vec![parity_key(Key::Z, modifiers)],
            modifiers,
        );
        assert_eq!(app.model(), &before);
        assert!(!app.modify.active());
        crate::input_tests::frame(
            &mut app,
            &ctx,
            vec![parity_key(Key::Y, modifiers)],
            modifiers,
        );
        assert_eq!(app.model(), &solid);
        assert!(!app.modify.active());
        app.model().validate().unwrap();
    }

    #[test]
    #[ignore = "Push/Pull parity: P currently activates Polygon"]
    fn push_pull_parity_p_activates_push_pull() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
        );
        frame(
            &mut app,
            &ctx,
            vec![parity_key(Key::P, egui::Modifiers::NONE)],
        );
        assert!(app.modifying);
        assert_eq!(app.modify.kind(), ModifyKind::PushPull);
    }

    #[test]
    #[ignore = "Push/Pull parity: completed operation cannot be remeasured; session is NotReady"]
    fn push_pull_parity_retype_after_commit_changes_same_extrusion() {
        let mut app = DrafterApp::default();
        rectangle(&mut app);
        app.choose_modify(ModifyKind::PushPull);
        let face = *app.model().faces().keys().next().unwrap();
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.measurements.text = "2m".into();
        app.apply_measurement();
        app.measurements.text = "150cm".into();
        app.apply_measurement();
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!((app.model().bounds().unwrap().1.z - 1.5).abs() < 1e-7);
        app.model().validate().unwrap();
    }
}

#[cfg(test)]
mod select_parity {
    use super::*;

    fn fixture() -> (DrafterApp, egui::Context) {
        let mut app = DrafterApp::default();
        for x in [0., 4.] {
            app.execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(x, 0., 0.),
                    DVec3::new(x + 2., 0., 0.),
                    DVec3::new(x + 2., 2., 0.),
                    DVec3::new(x, 2., 0.),
                ],
                closed: true,
            })
            .unwrap();
        }
        app.choose_modify(ModifyKind::Select);
        let ctx = egui::Context::default();
        send(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        (app, ctx)
    }

    fn point(app: &DrafterApp, world: DVec3) -> egui::Pos2 {
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let p = view.project(world).unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }

    fn send(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mods: egui::Modifiers,
    ) {
        crate::input_tests::frame(app, ctx, events, mods);
    }

    fn button(p: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn click(app: &mut DrafterApp, ctx: &egui::Context, p: egui::Pos2, mods: egui::Modifiers) {
        send(
            app,
            ctx,
            vec![egui::Event::PointerMoved(p), button(p, true, mods)],
            mods,
        );
        send(app, ctx, vec![button(p, false, mods)], mods);
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn select_parity_units_and_clicks_do_not_edit_geometry() {
        let (mut app, ctx) = fixture();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        let p = point(&app, DVec3::new(1., 1., 0.));
        for clicked in [false, true] {
            if clicked {
                click(&mut app, &ctx, p, egui::Modifiers::NONE);
                assert_eq!(app.selection.faces.len(), 1);
                assert!(app.selection.edges.is_empty());
            }
            for text in ["2m", "150cm", "3'6\""] {
                send(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::Text(text.into()),
                        key(Key::Enter, egui::Modifiers::NONE),
                    ],
                    egui::Modifiers::NONE,
                );
                assert!(app.measurements.text.is_empty());
                assert_eq!(app.model(), &before);
                assert_eq!(app.history.undo_depth(), depth);
                assert!(!app.modify.active());
            }
        }
        app.model().validate().unwrap();
    }

    #[test]
    fn select_parity_empty_click_clears() {
        let (mut app, ctx) = fixture();
        let p = point(&app, DVec3::new(1., 1., 0.));
        click(&mut app, &ctx, p, egui::Modifiers::NONE);
        assert_eq!(app.selection.faces.len(), 1);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        click(&mut app, &ctx, egui::pos2(40., 40.), egui::Modifiers::NONE);
        assert_eq!(app.selection, Selection::default());
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(app.error.is_none());
    }

    #[test]
    fn select_parity_escape_cancels_box_without_model_change() {
        let (mut app, ctx) = fixture();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        let p = egui::pos2(40., 40.);
        send(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(p),
                button(p, true, egui::Modifiers::NONE),
            ],
            egui::Modifiers::NONE,
        );
        send(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(500., 300.))],
            egui::Modifiers::NONE,
        );
        assert!(app.selection_start.is_some());
        send(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        send(
            &mut app,
            &ctx,
            vec![button(egui::pos2(500., 300.), false, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.selection_start.is_none());
        assert!(!app.modify.active());
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.selection, Selection::default());
    }

    #[test]
    fn select_parity_undo_redo_resets_box() {
        let (mut app, ctx) = fixture();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        let p = point(&app, DVec3::new(1., 1., 0.));
        click(&mut app, &ctx, p, egui::Modifiers::NONE);
        app.selection_start = Some(glam::DVec2::new(40., 40.));
        let mods = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        send(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p), key(Key::Z, mods)],
            mods,
        );
        assert_eq!(app.model().faces().len(), 1);
        assert_eq!(app.history.undo_depth(), depth - 1);
        assert!(app.selection_start.is_none());
        assert_eq!(app.selection, Selection::default());
        send(&mut app, &ctx, vec![key(Key::Y, mods)], mods);
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert!(!app.modify.active());
        app.model().validate().unwrap();
    }

    #[test]
    #[ignore = "Select has no face double-click expansion; routes to single select"]
    fn select_parity_double_face_expands() {
        let (mut app, ctx) = fixture();
        let p = point(&app, DVec3::new(1., 1., 0.));
        for _ in 0..2 {
            click(&mut app, &ctx, p, egui::Modifiers::NONE);
        }
        assert_eq!(app.selection.faces.len(), 1);
        assert_eq!(app.selection.edges.len(), 4);
    }

    #[test]
    #[ignore = "Select has no edge double-click expansion; routes to single select"]
    fn select_parity_double_edge_expands() {
        let (mut app, ctx) = fixture();
        let p = point(&app, DVec3::new(1., 0., 0.));
        for _ in 0..2 {
            click(&mut app, &ctx, p, egui::Modifiers::NONE);
        }
        assert_eq!(app.selection.edges.len(), 1);
        assert_eq!(app.selection.faces.len(), 1);
    }

    #[test]
    #[ignore = "Ctrl is not read for additive raw-geometry selection"]
    fn select_parity_ctrl_adds() {
        let (mut app, ctx) = fixture();
        let a = point(&app, DVec3::new(1., 1., 0.));
        let b = point(&app, DVec3::new(5., 1., 0.));
        click(&mut app, &ctx, a, egui::Modifiers::NONE);
        click(
            &mut app,
            &ctx,
            b,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        assert_eq!(app.selection.faces.len(), 2);
    }

    #[test]
    #[ignore = "Shift inserts selected entities instead of toggling membership"]
    fn select_parity_shift_toggles() {
        let (mut app, ctx) = fixture();
        let p = point(&app, DVec3::new(1., 1., 0.));
        click(&mut app, &ctx, p, egui::Modifiers::NONE);
        click(&mut app, &ctx, p, egui::Modifiers::SHIFT);
        assert!(app.selection.faces.is_empty());
    }

    #[test]
    #[ignore = "Ctrl+Shift adds rather than subtracting raw geometry"]
    fn select_parity_ctrl_shift_subtracts() {
        let (mut app, ctx) = fixture();
        let a = point(&app, DVec3::new(1., 1., 0.));
        let b = point(&app, DVec3::new(5., 1., 0.));
        click(&mut app, &ctx, a, egui::Modifiers::NONE);
        click(&mut app, &ctx, b, egui::Modifiers::SHIFT);
        assert_eq!(app.selection.faces.len(), 2);
        click(
            &mut app,
            &ctx,
            a,
            egui::Modifiers {
                ctrl: true,
                command: true,
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(app.selection.faces.len(), 1);
    }

    #[test]
    #[ignore = "Ctrl+A has no Select-all routing"]
    fn select_parity_ctrl_a_selects_all() {
        let (mut app, ctx) = fixture();
        let mods = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        send(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(40., 40.)),
                key(Key::A, mods),
            ],
            mods,
        );
        assert_eq!(app.selection.faces.len(), 2);
        assert_eq!(app.selection.edges.len(), 8);
    }
}

#[cfg(test)]
mod transform_parity_tests {
    use super::*;
    fn fixture(kind: ModifyKind) -> (DrafterApp, egui::Context) {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 2.,
                (DVec3::X + DVec3::Y) * 2.,
                DVec3::Y * 2.,
            ],
            closed: true,
        })
        .unwrap();
        app.camera.set_view(StandardView::Top);
        app.camera.projection = Projection::Orthographic;
        app.camera.target = glam::Vec3::new(1., 1., 0.);
        app.camera.distance = 10.;
        app.selection.vertices = app.model().vertices().keys().copied().collect();
        app.choose_modify(kind);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        (app, ctx)
    }
    fn frame(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        crate::input_tests::frame(app, ctx, events, modifiers);
    }
    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn position(app: &DrafterApp, point: DVec3) -> egui::Pos2 {
        let p = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        }
        .project(point)
        .unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn begin(app: &mut DrafterApp) {
        app.modify
            .begin(
                app.history.model(),
                &app.selection,
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.set_reference(DVec3::X).unwrap();
        app.modify
            .update(if app.modify.kind() == ModifyKind::Copy {
                DVec3::Z * 3.
            } else {
                DVec3::X * 3.
            })
            .unwrap();
    }
    #[test]
    fn transform_parity_keyboard_measurements_escape_and_undo_reset_all_kinds() {
        for (kind, text) in [
            (ModifyKind::Move, "2m"),
            (ModifyKind::Copy, "150cm"),
            (ModifyKind::Rotate, "90"),
            (ModifyKind::Scale, "2"),
        ] {
            let (mut app, ctx) = fixture(kind);
            let before = app.model().clone();
            let depth = app.history.undo_depth();
            let hover = position(&app, DVec3::ZERO);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(hover)],
                egui::Modifiers::NONE,
            );
            begin(&mut app);
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::Text(text.into()),
                    key(Key::Enter, egui::Modifiers::NONE),
                ],
                egui::Modifiers::NONE,
            );
            assert!(app.error.is_none(), "{kind:?}: {:?}", app.error);
            assert_eq!(app.history.undo_depth(), depth + 1);
            assert!(!app.modify.active());
            let after = app.model().clone();
            assert_ne!(after, before);
            begin(&mut app);
            let command = app
                .modify
                .measure(if kind == ModifyKind::Rotate {
                    "45"
                } else {
                    "3"
                })
                .unwrap()
                .unwrap();
            app.modify_preview(command);
            frame(
                &mut app,
                &ctx,
                vec![key(Key::Escape, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.model(), &after);
            assert!(!app.modify.active());
            let command = egui::Modifiers {
                ctrl: true,
                command: true,
                ..egui::Modifiers::NONE
            };
            begin(&mut app);
            frame(&mut app, &ctx, vec![key(Key::Z, command)], command);
            assert_eq!(app.model(), &before);
            assert!(!app.modify.active());
            frame(&mut app, &ctx, vec![key(Key::Y, command)], command);
            assert_eq!(app.model(), &after);
        }
    }
    #[test]
    fn transform_parity_move_two_clicks_and_axis_lock_preview() {
        let (mut app, ctx) = fixture(ModifyKind::Move);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        let start = position(&app, DVec3::ZERO);
        let end = position(&app, DVec3::new(3., 1., 0.));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![button(start, false)],
            egui::Modifiers::NONE,
        );
        assert!(app.modify.active());
        frame(
            &mut app,
            &ctx,
            vec![
                key(Key::ArrowRight, egui::Modifiers::NONE),
                egui::Event::PointerMoved(end),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.axis_lock, Some(Axis::X));
        let (lo, hi) = app.model().bounds().unwrap();
        assert!(lo.y.abs() < 1e-6 && (hi.y - 2.).abs() < 1e-6);
        assert!(lo.x > 1.);
        assert_eq!(app.history.undo_depth(), depth);
        frame(
            &mut app,
            &ctx,
            vec![button(end, true)],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![button(end, false)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!(!app.modify.active());
        app.history_step(false);
        assert_eq!(app.model(), &before);
    }
    fn shortcut(keyboard_key: Key, kind: ModifyKind) {
        let (mut app, ctx) = fixture(ModifyKind::Select);
        let hover = position(&app, DVec3::ZERO);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(hover)],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![key(keyboard_key, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.modify.kind(), kind);
    }
    #[test]
    #[ignore = "gap: M shortcut is not routed to Move"]
    fn transform_parity_m_shortcut() {
        shortcut(Key::M, ModifyKind::Move);
    }
    #[test]
    fn transform_parity_q_shortcut() {
        shortcut(Key::Q, ModifyKind::Rotate);
    }
    #[test]
    #[ignore = "gap: S shortcut is not routed to Scale"]
    fn transform_parity_s_shortcut() {
        shortcut(Key::S, ModifyKind::Scale);
    }
    #[test]
    #[ignore = "gap: Ctrl does not toggle Move copy mode"]
    fn transform_parity_ctrl_copy_toggle() {
        let (mut app, ctx) = fixture(ModifyKind::Move);
        let start = position(&app, DVec3::ZERO);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![button(start, false)],
            egui::Modifiers::NONE,
        );
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..egui::Modifiers::NONE
        };
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("3m".into()), key(Key::Enter, ctrl)],
            ctrl,
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.model().faces().len(), 2);
    }
    #[test]
    fn transform_parity_rotate_clicked_pivot() {
        let (mut app, ctx) = fixture(ModifyKind::Rotate);
        let p = position(&app, DVec3::ZERO);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p), button(p, true)],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![button(p, false)],
            egui::Modifiers::NONE,
        );
        assert!(app.modify.anchor().unwrap().distance(DVec3::ZERO) < 1e-6);
    }
    #[test]
    #[ignore = "gap: Scale has no corner grips/opposite-corner anchor; uses centroid"]
    fn transform_parity_scale_opposite_corner_anchor() {
        let (mut app, ctx) = fixture(ModifyKind::Scale);
        let p = position(&app, DVec3::ZERO);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p), button(p, true)],
            egui::Modifiers::NONE,
        );
        assert!(
            app.modify
                .anchor()
                .unwrap()
                .distance(DVec3::new(2., 2., 0.))
                < 1e-6
        );
    }
}

#[cfg(test)]
mod offset_follow_me_parity_tests {
    use super::*;
    use cindral_core::FaceId;
    fn fixture(kind: ModifyKind) -> (DrafterApp, egui::Context, FaceId) {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 10.,
                (DVec3::X + DVec3::Y) * 10.,
                DVec3::Y * 10.,
            ],
            closed: true,
        })
        .unwrap();
        let face = *app.model().faces().keys().next().unwrap();
        app.camera.set_view(StandardView::Top);
        app.camera.projection = Projection::Orthographic;
        app.camera.target = glam::Vec3::new(5., 5., 0.);
        app.camera.distance = 30.;
        app.choose_modify(kind);
        let ctx = egui::Context::default();
        send(&mut app, &ctx, vec![]);
        send(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(308., 208.))],
        );
        (app, ctx, face)
    }
    fn send(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        crate::input_tests::frame(app, ctx, events, modifiers);
    }
    fn key(key: Key, command: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: command,
                command,
                ..Default::default()
            },
        }
    }
    fn begin(app: &mut DrafterApp, face: FaceId) {
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.drag_axis = DVec3::Z;
        let command = if app.modify.kind() == ModifyKind::Offset {
            app.modify.measure("1m").unwrap().unwrap()
        } else {
            app.modify.update(DVec3::Z).unwrap().unwrap()
        };
        app.modify_preview(command);
        assert!(app.error.is_none(), "{:?}", app.error);
    }
    #[test]
    fn offset_follow_me_parity_escape_keyboard_measurement_and_undo_reset() {
        for kind in [ModifyKind::Offset, ModifyKind::FollowMe] {
            let (mut app, ctx, face) = fixture(kind);
            let before = app.model().clone();
            let depth = app.history.undo_depth();
            begin(&mut app, face);
            send(&mut app, &ctx, vec![key(Key::Escape, false)]);
            assert_eq!(app.model(), &before);
            assert_eq!(app.history.undo_depth(), depth);
            assert!(!app.modify.active());
            begin(&mut app, face);
            // Follow Me typed behavior is a Cindral routing contract, not a parity claim (Q7).
            send(
                &mut app,
                &ctx,
                vec![egui::Event::Text("150cm".into()), key(Key::Enter, false)],
            );
            assert!(app.error.is_none(), "{kind:?}: {:?}", app.error);
            assert_eq!(app.history.undo_depth(), depth + 1);
            assert!(!app.modify.active());
            let after = app.model().clone();
            let next_face = *app
                .model()
                .faces()
                .iter()
                .find(|(_, f)| f.plane.normal.z.abs() > 0.9)
                .unwrap()
                .0;
            begin(&mut app, next_face);
            send(&mut app, &ctx, vec![key(Key::Z, true)]);
            assert_eq!(app.model(), &before);
            assert!(!app.modify.active());
            send(&mut app, &ctx, vec![key(Key::Y, true)]);
            assert_eq!(app.model(), &after);
            app.model().validate().unwrap();
        }
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn position(app: &DrafterApp, point: DVec3) -> egui::Pos2 {
        let p = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        }
        .project(point)
        .unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    #[test]
    fn offset_parity_two_clicks_commit_a_valid_preview() {
        let (mut app, ctx, _) = fixture(ModifyKind::Offset);
        let start = position(&app, DVec3::new(0., 5., 0.));
        let end = position(&app, DVec3::new(1., 5., 0.));
        let depth = app.history.undo_depth();
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        send(&mut app, &ctx, vec![button(start, true)]);
        send(&mut app, &ctx, vec![button(start, false)]);
        assert!(app.modify.active());
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        assert!(app.error.is_none(), "{:?}", app.error);
        send(&mut app, &ctx, vec![button(end, true)]);
        send(&mut app, &ctx, vec![button(end, false)]);
        assert!(!app.modify.active());
        assert_eq!(app.history.undo_depth(), depth + 1);
        app.model().validate().unwrap();
    }
    #[test]
    #[ignore = "gap: F fits view instead of activating Offset"]
    fn offset_parity_f_shortcut() {
        let (mut app, ctx, _) = fixture(ModifyKind::Select);
        send(&mut app, &ctx, vec![key(Key::F, false)]);
        assert!(app.modifying && app.modify.kind() == ModifyKind::Offset);
    }
    #[test]
    #[ignore = "gap: Follow Me preselected path does not commit on a stationary profile click"]
    fn follow_me_parity_preselected_path_completes_on_profile_click() {
        let (mut app, ctx, _) = fixture(ModifyKind::FollowMe);
        app.execute(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::Z * 3.,
        })
        .unwrap();
        app.selection.edges = app
            .model()
            .edges()
            .iter()
            .filter(|(_, e)| {
                e.vertices
                    .iter()
                    .any(|v| app.model().vertices()[v].position.z > 0.)
            })
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(app.selection.edges.len(), 1);
        let depth = app.history.undo_depth();
        let p = position(&app, DVec3::new(5., 5., 0.));
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        send(&mut app, &ctx, vec![button(p, true)]);
        send(&mut app, &ctx, vec![button(p, false)]);
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!(!app.modify.active());
    }

    #[cfg(feature = "kernel-benchmark")]
    #[test]
    #[ignore = "release-only opt-in 20k-face CPU subset; excludes picking, inference, mesh, GPU and UI"]
    #[allow(clippy::assertions_on_constants)]
    fn offset_follow_me_parity_release_cpu_probe() {
        assert!(!cfg!(debug_assertions), "run --release");
        let mut within_budget = true;
        for kind in [ModifyKind::Offset, ModifyKind::FollowMe] {
            let mut h = cindral_core::benchmark::tiled_squares(100, 100)
                .unwrap()
                .history;
            h.execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-20., -20., 0.),
                    DVec3::new(-10., -20., 0.),
                    DVec3::new(-10., -10., 0.),
                    DVec3::new(-20., -10., 0.),
                ],
                closed: true,
            })
            .unwrap();
            let face = *h
                .model()
                .faces()
                .iter()
                .find(|(id, _)| {
                    h.model()
                        .face_area(**id)
                        .is_ok_and(|a| (a - 100.).abs() < 1e-7)
                })
                .unwrap()
                .0;
            assert_eq!(h.model().faces().len(), 20_001);
            let mut s = ModifySession::new(kind);
            s.begin(
                h.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::new(-20., -20., 0.),
                DVec3::Z,
            )
            .unwrap();
            let mut max_ms = 0_f64;
            for i in 0..10 {
                let start = std::time::Instant::now();
                let command = if kind == ModifyKind::Offset {
                    s.measure(&format!("{}m", 0.5 + i as f64 * 0.05))
                        .unwrap()
                        .unwrap()
                } else {
                    s.update(DVec3::new(-20., -20., 1. + i as f64 * 0.05))
                        .unwrap()
                        .unwrap()
                };
                h.preview(command).unwrap();
                max_ms = max_ms.max(start.elapsed().as_secs_f64() * 1000.);
            }
            let start = std::time::Instant::now();
            assert!(h.commit_preview().unwrap());
            let commit_ms = start.elapsed().as_secs_f64() * 1000.;
            println!(
                "{kind:?}: faces={} updates=10 max_preview_ms={max_ms:.6} commit_ms={commit_ms:.6}",
                20_001
            );
            within_budget &= max_ms < 16. && commit_ms < 100.;
        }
        assert!(
            within_budget,
            "CPU subset exceeds preview or commit budget; see printed measurements"
        );
    }
}

#[cfg(test)]
mod utility_parity_tests {
    use super::*;
    fn send(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. }
                | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        crate::input_tests::frame(app, ctx, events, modifiers);
    }
    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn pos(app: &DrafterApp, point: DVec3) -> egui::Pos2 {
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let p = view.project(point).unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    fn button(p: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        }
    }
    fn setup(kind: ModifyKind) -> (DrafterApp, egui::Context) {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 10.,
                (DVec3::X + DVec3::Y) * 10.,
                DVec3::Y * 10.,
            ],
            closed: true,
        })
        .unwrap();
        app.camera.set_view(StandardView::Top);
        app.camera.projection = Projection::Orthographic;
        app.camera.target = glam::Vec3::new(5., 5., 0.);
        app.camera.distance = 30.;
        app.choose_modify(kind);
        let ctx = egui::Context::default();
        send(&mut app, &ctx, vec![]);
        send(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(308., 208.))],
        );
        (app, ctx)
    }
    fn click(app: &mut DrafterApp, ctx: &egui::Context, point: DVec3) {
        let p = pos(app, point);
        send(app, ctx, vec![egui::Event::PointerMoved(p)]);
        send(app, ctx, vec![button(p, true, egui::Modifiers::NONE)]);
        send(app, ctx, vec![button(p, false, egui::Modifiers::NONE)]);
    }
    #[test]
    fn utility_parity_tape_two_click_distance_and_escape() {
        let (mut app, ctx) = setup(ModifyKind::TapeMeasure);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        click(&mut app, &ctx, DVec3::ZERO);
        assert!(
            app.modify.active(),
            "error={:?} last={:?} guides={:?}",
            app.error,
            app.last_tape,
            app.modify.guides()
        );
        let p = pos(&app, DVec3::new(10., 10., 0.));
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        assert!((app.modify.measurement().unwrap() - 200_f64.sqrt()).abs() < 1e-5);
        send(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(!app.modify.active());
        assert!(app.modify.guides().is_empty());
        assert_eq!(app.model(), &before);
        click(&mut app, &ctx, DVec3::ZERO);
        click(&mut app, &ctx, DVec3::new(10., 10., 0.));
        assert!(!app.modify.active());
        assert!((app.last_tape.unwrap() - 200_f64.sqrt()).abs() < 1e-5);
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.model(), &before);
    }
    #[test]
    fn utility_parity_tape_keyboard_units_and_arrow_lock() {
        for text in ["2m", "150cm"] {
            let (mut app, ctx) = setup(ModifyKind::TapeMeasure);
            app.modify
                .begin(
                    app.history.model(),
                    &Selection::default(),
                    None,
                    DVec3::ZERO,
                    DVec3::Z,
                )
                .unwrap();
            let p = pos(&app, DVec3::new(8., 6., 0.));
            send(
                &mut app,
                &ctx,
                vec![
                    key(Key::ArrowRight, egui::Modifiers::NONE),
                    egui::Event::PointerMoved(p),
                ],
            );
            assert_eq!(app.axis_lock, Some(Axis::X));
            assert!((app.modify.measurement().unwrap() - 8.).abs() < 1e-4);
            send(
                &mut app,
                &ctx,
                vec![
                    egui::Event::Text(text.into()),
                    key(Key::Enter, egui::Modifiers::NONE),
                ],
            );
            assert!(app.tape_resize.is_some());
            app.confirm_tape_resize(true);
            let expected = if text == "2m" { 2. } else { 1.5 };
            assert!((app.last_tape.unwrap() - expected).abs() < 1e-7);
            assert!(!app.modify.active());
        }
    }
    #[test]
    fn eraser_click_is_local_even_with_unrelated_selection() {
        for point in [DVec3::new(5., 5., 0.), DVec3::new(5., 0., 0.), DVec3::ZERO] {
            let (mut app, ctx) = setup(ModifyKind::Eraser);
            // A second face shares an edge with the first.
            app.execute(Command::AddPolyline {
                points: vec![
                    DVec3::ZERO,
                    DVec3::X * 10.,
                    DVec3::new(10., 0., 10.),
                    DVec3::Z * 10.,
                ],
                closed: true,
            })
            .unwrap();
            let before = app.model().clone();
            app.selection.edges = before.edges().keys().copied().collect();
            app.selection.faces = before.faces().keys().copied().collect();
            let depth = app.history.undo_depth();
            click(&mut app, &ctx, point);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.history.undo_depth(), depth + 1);
            if point.y > 0. {
                assert_eq!(app.model().edges(), before.edges());
                assert_eq!(app.model().vertices(), before.vertices());
                assert_eq!(app.model().faces().len(), before.faces().len() - 1);
                for (id, face) in app.model().faces() {
                    assert_eq!(before.faces().get(id), Some(face));
                }
            } else {
                // Including a shared endpoint, one edge is erased, never all incident edges.
                assert_eq!(
                    before
                        .edges()
                        .keys()
                        .filter(|id| !app.model().edges().contains_key(id))
                        .count(),
                    1
                );
            }
            let after = app.model().clone();
            app.model().validate().unwrap();
            app.history.undo().unwrap();
            assert_eq!(app.model(), &before);
            app.history.redo().unwrap();
            assert_eq!(app.model(), &after);
        }
        let (mut app, ctx) = setup(ModifyKind::Eraser);
        app.selection.edges = app.model().edges().keys().copied().collect();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        click(&mut app, &ctx, DVec3::new(15., 15., 0.));
        assert!(app.error.is_none());
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
    }

    #[test]
    fn utility_parity_click_history_and_undo_reset() {
        for kind in [ModifyKind::Eraser, ModifyKind::PaintBucket] {
            let (mut app, ctx) = setup(kind);
            let before = app.model().clone();
            let depth = app.history.undo_depth();
            let point = if kind == ModifyKind::Eraser {
                DVec3::X * 5.
            } else {
                DVec3::new(5., 5., 0.)
            };
            click(&mut app, &ctx, point);
            assert!(!app.modify.active());
            assert_eq!(app.history.undo_depth(), depth + 1);
            app.model().validate().unwrap();
            let after = app.model().clone();
            app.choose_modify(ModifyKind::TapeMeasure);
            app.modify
                .begin(
                    app.history.model(),
                    &Selection::default(),
                    None,
                    DVec3::ZERO,
                    DVec3::Z,
                )
                .unwrap();
            assert!(app.modify.active());
            let command = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            send(&mut app, &ctx, vec![key(Key::Z, command)]);
            assert!(!app.modify.active());
            assert_eq!(app.model(), &before);
            send(&mut app, &ctx, vec![key(Key::Y, command)]);
            assert_eq!(app.model(), &after);
        }
    }
    #[test]
    #[ignore = "E/T/B activation shortcuts are not routed"]
    fn utility_parity_activation_shortcuts() {
        for (key_code, kind) in [
            (Key::E, ModifyKind::Eraser),
            (Key::T, ModifyKind::TapeMeasure),
            (Key::B, ModifyKind::PaintBucket),
        ] {
            let (mut app, ctx) = setup(ModifyKind::Select);
            send(&mut app, &ctx, vec![key(key_code, egui::Modifiers::NONE)]);
            assert_eq!(app.modify.kind(), kind);
        }
    }
    #[test]
    #[ignore = "Eraser currently commits on press instead of waiting for stroke release"]
    fn utility_parity_eraser_stroke_defers_until_release() {
        let (mut app, ctx) = setup(ModifyKind::Eraser);
        let before = app.model().clone();
        let p = pos(&app, DVec3::X * 5.);
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        send(&mut app, &ctx, vec![button(p, true, egui::Modifiers::NONE)]);
        assert_eq!(app.model(), &before);
        send(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        assert_eq!(app.model(), &before);
    }
    #[test]
    #[ignore = "Ctrl Eraser is not routed to soften/smooth; currently deletes edge"]
    fn utility_parity_eraser_ctrl_softens_preserving_geometry() {
        let (mut app, ctx) = setup(ModifyKind::Eraser);
        app.execute(Command::AddEdge {
            start: DVec3::ZERO,
            end: (DVec3::X + DVec3::Y) * 10.,
        })
        .unwrap();
        let count = app.model().edges().len();
        let p = pos(&app, DVec3::new(5., 5., 0.));
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        send(&mut app, &ctx, vec![button(p, true, ctrl)]);
        send(&mut app, &ctx, vec![button(p, false, ctrl)]);
        assert_eq!(app.model().edges().len(), count);
        assert!(app.model().edges().values().any(|e| e.soft && e.smooth));
    }
    #[test]
    #[ignore = "Tape guide creation bypasses command history"]
    fn utility_parity_tape_guide_undo() {
        let (mut app, ctx) = setup(ModifyKind::TapeMeasure);
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.update(DVec3::X * 3.).unwrap();
        app.measurements.text = "2m".into();
        app.apply_measurement();
        assert_eq!(app.modify.guides().len(), 1);
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        send(&mut app, &ctx, vec![key(Key::Z, command)]);
        assert!(app.modify.guides().is_empty());
    }
    #[test]
    fn utility_parity_tape_escape_preserves_model_history_and_guides() {
        let (mut app, ctx) = setup(ModifyKind::TapeMeasure);
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.update(DVec3::X * 3.).unwrap();
        send(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(!app.modify.active());
        assert!(app.modify.guides().is_empty());
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
    }
    #[test]
    #[ignore = "Alt sampler is not routed; click paints instead of sampling"]
    fn utility_parity_paint_alt_samples_without_painting() {
        let (mut app, ctx) = setup(ModifyKind::PaintBucket);
        let before = app.model().clone();
        let p = pos(&app, DVec3::new(5., 5., 0.));
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        send(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        send(&mut app, &ctx, vec![button(p, true, alt)]);
        send(&mut app, &ctx, vec![button(p, false, alt)]);
        assert_eq!(app.model(), &before);
        let face = *app.model().faces().keys().next().unwrap();
        assert_eq!(app.paint_color, app.model().faces()[&face].color);
    }
    #[cfg(feature = "kernel-benchmark")]
    #[test]
    #[ignore = "release-only 20k-face CPU subset; excludes picking/inference/mesh/UI/GPU"]
    #[allow(clippy::assertions_on_constants)]
    fn utility_parity_release_cpu_probe() {
        assert!(!cfg!(debug_assertions), "run --release");
        let mut within_budget = true;
        for kind in [
            ModifyKind::Eraser,
            ModifyKind::PaintBucket,
            ModifyKind::TapeMeasure,
        ] {
            let mut h = cindral_core::benchmark::tiled_squares(100, 100)
                .unwrap()
                .history;
            let face = *h.model().faces().keys().next().unwrap();
            let edge = *h.model().edges().keys().next().unwrap();
            let hit = if kind == ModifyKind::Eraser {
                Hit::Edge(edge)
            } else {
                Hit::Face(face)
            };
            let mut s = ModifySession::new(kind);
            s.begin(
                h.model(),
                &Selection::default(),
                Some(hit),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
            let mut max_ms = 0_f64;
            for sample in 1..=10 {
                let start = std::time::Instant::now();
                if let Some(c) = s.update(DVec3::X * sample as f64).unwrap() {
                    h.preview(c).unwrap();
                }
                max_ms = max_ms.max(start.elapsed().as_secs_f64() * 1000.);
            }
            let start = std::time::Instant::now();
            s.finish();
            if kind != ModifyKind::TapeMeasure {
                assert!(h.commit_preview().unwrap());
            }
            let commit_ms = start.elapsed().as_secs_f64() * 1000.;
            println!("{kind:?}: 20,000 faces, one target, 10 CPU updates; max={max_ms:.6}ms commit={commit_ms:.6}ms");
            within_budget &= max_ms < 16. && commit_ms < 100.;
        }
        assert!(
            within_budget,
            "CPU subset exceeds budget; see printed measurements"
        );
    }
}
