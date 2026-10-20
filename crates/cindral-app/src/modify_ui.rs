//! Modify input routing and screen overlays; kernel previews use ordinary dirty deltas.
use super::*;
use cindral_tools::{Hit, ModifyKind};
impl DrafterApp {
    pub(crate) fn choose_modify(&mut self, kind: ModifyKind) {
        self.cancel_modify();
        self.modifying = true;
        self.modify.set_kind(kind);
        self.modify.set_color(self.paint_color);
        self.tool.cancel();
        self.measurement.clear();
    }
    fn cancel_modify(&mut self) {
        match self.history.cancel_preview() {
            Ok(true) => self.refresh_mesh(),
            Err(e) => self.error = Some(e.to_string()),
            _ => {}
        }
        self.modify.cancel();
        self.modify_press = None;
        self.selection_start = None;
    }
    fn modify_preview(&mut self, command: Command) {
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
            Ok(_) => self.error = None,
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
        match self.modify.measure(&self.measurement) {
            Ok(Some(command)) => {
                self.modify_preview(command);
                if self.error.is_none() {
                    self.finish_modify();
                    self.measurement.clear();
                }
            }
            Ok(None) => {
                if self.modify.kind() == ModifyKind::TapeMeasure {
                    self.last_tape = self.modify.measurement();
                    if let Err(e) = self.modify.add_guide() {
                        self.error = Some(e.to_string());
                    }
                    self.modify.finish();
                    self.measurement.clear();
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    pub(crate) fn modify_viewport_input(&mut self, response: &egui::Response) {
        let rect = response.rect;
        let view = View {
            matrix: self
                .camera
                .view_projection(rect.width() / rect.height().max(1.))
                .as_dmat4(),
            size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
        };
        let keyboard_free = !response.ctx.wants_keyboard_input();
        let escape = response.ctx.input(|i| i.key_pressed(Key::Escape));
        if escape {
            self.cancel_modify();
            self.measurement.clear();
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
                for event in &i.events {
                    if let egui::Event::Text(text) = event {
                        if self.modify.active()
                            && (!self.measurement.is_empty()
                                || text
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_digit() || "+-.".contains(c)))
                        {
                            self.measurement.push_str(text);
                        }
                    }
                }
                if i.key_pressed(Key::Backspace) {
                    self.measurement.pop();
                }
            });
            if response.ctx.input(|i| i.key_pressed(Key::Enter)) && !self.measurement.is_empty() {
                self.modify_measurement();
                return;
            }
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
            origin: origin.as_dvec3(),
            direction: direction.as_dvec3(),
        };
        let hit = cindral_tools::pick(self.history.model(), ray, view, cursor, 10.);
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
        if pressed && !self.modify.active() {
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
                        .modify
                        .measurement()
                        .is_some_and(|d| d > cindral_core::TOLERANCE)
                {
                    self.last_tape = self.modify.measurement();
                    if let Err(e) = self.modify.add_guide() {
                        self.error = Some(e.to_string());
                    }
                    self.modify.finish();
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
        let stroke = egui::Stroke::new(3_f32, Color32::from_rgb(255, 184, 68));
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
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::LIGHT_GREEN
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
                    egui::Stroke::new(1_f32, Color32::from_rgb(85, 185, 210)),
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
                Color32::WHITE,
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
            app.measurement = text.into();
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
    fn triple_click_routes_connected_selection_and_tape_does_not_edit_history() {
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
        app.measurement = "5m".into();
        app.apply_measurement();
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.modify.guides().len(), 1);
        assert_eq!(app.last_tape, Some(5.));
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
}
