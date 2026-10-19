use super::super::precision::Mode;
use super::*;
use drawer_core::{
    magnetic_lasso::{Input, Reference, Scope, Segment, Settings, Trace},
    pointer_input::{Decision, Phase, Source},
    selection_edit::Combine,
};
pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    pub projection: Projection,
    revision: u64,
    frame: u32,
    root: egui::Rect,
    source: Source,
    mode: Mode,
    settings: Settings,
    combine: Combine,
    trace: Trace,
    segments: Vec<Vec<Point>>,
    active: Vec<Point>,
    draft: Vec<Vec<Point>>,
    points: Vec<Point>,
    outline: Vec<Point>,
    hover: Option<Point>,
}
impl Gesture {
    fn multiple(&self) -> bool {
        self.settings.input == Input::Multiple || self.mode == Mode::SelectPolygon
    }
    fn rebuild(&mut self) -> Result<(), String> {
        self.points.clear();
        for p in self.segments.iter().chain(&self.draft).flatten() {
            if self.points.last() != Some(p) {
                self.points.push(*p);
            }
            if self.points.len() > 4096 {
                return Err("Lasso exceeds4096 traced points; prior selection kept".into());
            }
        }
        self.outline = self.points.clone();
        if self.points.len() >= 3 {
            let kind = if self.multiple() {
                Segment::Polyline
            } else {
                Segment::Freehand
            };
            let route = self
                .trace
                .route(self.points.last().copied(), self.points[0], kind)?;
            for p in route {
                if self.outline.last() != Some(&p) {
                    self.outline.push(p);
                }
            }
            if self.outline.len() > 4096 {
                return Err("Lasso closing route exceeds 4096 points".into());
            }
        }
        Ok(())
    }
    fn refresh(&mut self) -> Result<(), String> {
        self.draft.clear();
        let mut from = self.segments.iter().rev().find_map(|s| s.last()).copied();
        let kind = if self.mode == Mode::SelectPolygon || self.multiple() && self.active.len() == 1
        {
            Segment::Polyline
        } else {
            Segment::Freehand
        };
        for p in &self.active {
            let route = self.trace.route(from, *p, kind)?;
            from = route.last().copied();
            self.draft.push(route);
        }
        self.rebuild()
    }
    fn commit(&self, doc: &mut Document) -> Result<bool, String> {
        let scene = drawer_core::selection_geometry::prepare(
            doc,
            drawer_core::selection_geometry::Kind::Polygon,
            &self.outline,
            self.combine,
        )?;
        doc.edit_scene(scene)
    }
}
impl Drawing {
    pub(super) fn uses_lasso(&self) -> bool {
        matches!(self.mode, Mode::SelectLasso | Mode::SelectPolygon)
            && (self.lasso_settings.enabled || self.lasso_settings.input == Input::Multiple)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_lasso(
        &mut self,
        doc: &Document,
        view: u64,
        projection: Projection,
        root: egui::Rect,
        layer: Option<usize>,
        source: Source,
        p: Point,
    ) -> Result<(), String> {
        if self.lasso_settings.enabled
            && self.lasso_settings.reference == Reference::Current
            && layer.is_none()
        {
            return Err("Choose a raster edge reference or use composite/vector".into());
        }
        let trace = Trace::new(doc, layer.unwrap_or(0), self.lasso_settings)?;
        if doc
            .memory_bytes()
            .saturating_mul(3)
            .saturating_add(trace.bytes())
            .saturating_add(16 * 1024 * 1024)
            > self.fill_budget
        {
            return Err("Lasso exceeds workspace memory".into());
        }
        let mut g = Box::new(Gesture {
            document: doc.id,
            view,
            projection,
            revision: doc.revision,
            frame: doc.current_frame,
            root,
            source,
            mode: self.mode,
            settings: self.lasso_settings,
            combine: self.selection_settings.combine,
            trace,
            segments: vec![],
            active: vec![p],
            draft: vec![],
            points: vec![],
            outline: vec![],
            hover: None,
        });
        g.refresh()?;
        self.lasso = Some(g);
        Ok(())
    }
    pub(super) fn lasso_preview(&self, document: u64) -> Option<Vec<Point>> {
        let g = self.lasso.as_ref().filter(|g| g.document == document)?;
        let mut points = if g.hover.is_some() {
            g.points.clone()
        } else {
            g.outline.clone()
        };
        if g.active.is_empty()
            && let Some(p) = g.hover
        {
            points.push(p);
        }
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        Some(points)
    }
    pub(super) fn receive_lasso(&mut self, index: usize, doc: &mut Document, canvas: bool) {
        if self.events[index].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(e) = self.events[index].0 else {
            return;
        };
        let Some(mut g) = self.lasso.take() else {
            return;
        };
        if e.source.kind != g.source.kind
            || e.source.device != g.source.device
            || e.phase == Phase::Down && !canvas
            || e.phase != Phase::Down && !g.active.is_empty() && e.source != g.source
        {
            self.lasso = Some(g);
            return;
        }
        self.events[index].1 = true;
        if e.phase == Phase::Cancel {
            self.ownership.clear();
            return;
        }
        let p = g
            .projection
            .point(egui::pos2(e.position[0] as f32, e.position[1] as f32));
        let result = (|| -> Result<(), String> {
            if e.phase == Phase::Down {
                if self.ownership.receive(e, self.settings) != Decision::Begin {
                    return Ok(());
                }
                g.source = e.source;
                g.active = vec![p];
                g.hover = None;
                g.refresh()?;
            } else if !g.active.is_empty() {
                if g.mode != Mode::SelectPolygon
                    && g.active
                        .last()
                        .is_none_or(|q| (q[0] - p[0]).hypot(q[1] - p[1]) >= 0.25)
                {
                    if g.active.len() == 4096 {
                        return Err("Lasso input point limit".into());
                    }
                    g.active.push(p);
                    g.refresh()?;
                }
                if e.phase == Phase::Up {
                    self.ownership.clear();
                    g.segments.append(&mut g.draft);
                    g.active.clear();
                    g.rebuild()?;
                }
            } else {
                g.hover = Some(p);
            }
            Ok(())
        })();
        if let Err(err) = result {
            self.error = Some(err);
            self.ownership.clear();
            return;
        }
        if e.phase == Phase::Up && !g.multiple() {
            if let Err(err) = g.commit(doc) {
                self.error = Some(err);
            }
            return;
        }
        self.lasso = Some(g);
    }
    pub(super) fn advance_lasso(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        let Some(g) = self.lasso.as_ref() else {
            return;
        };
        if g.mode != self.mode
            || g.settings != self.lasso_settings
            || g.combine != self.selection_settings.combine
            || g.root != ctx.content_rect()
            || (g.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
            || docs.iter().find(|d| d.id == g.document).is_none_or(|d| {
                d.revision != g.revision
                    || d.current_frame != g.frame
                    || d.view != g.projection.view
            })
        {
            self.cancel();
            return;
        }
        if g.multiple() {
            if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
                let g = self.lasso.as_mut().unwrap();
                if !g.active.is_empty() {
                    g.active.pop();
                    if let Err(err) = g.refresh() {
                        self.error = Some(err);
                        self.cancel();
                        return;
                    }
                } else {
                    g.segments.pop();
                    if let Err(err) = g.rebuild() {
                        self.error = Some(err);
                        self.cancel();
                        return;
                    }
                }
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                });
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                let g = self.lasso.take().unwrap();
                if let Some(d) = docs.iter_mut().find(|d| d.id == g.document)
                    && let Err(err) = g.commit(d)
                {
                    self.error = Some(err);
                    self.lasso = Some(g);
                }
                self.ownership.clear();
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                });
            }
        }
        for i in 0..self.events.len() {
            let Some(id) = self.lasso.as_ref().map(|g| g.document) else {
                break;
            };
            let Some(d) = docs.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive_lasso(i, d, false);
        }
    }
}
impl Drawer {
    pub(crate) fn magnetic_lasso_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Magnetic lasso and multiple input",|ui|{
            let s=&mut self.workspaces.store.current.magnetic_lasso;let old=*s;
            ui.checkbox(&mut s.enabled,"Follow raster boundary / vector centerline");
            egui::ComboBox::from_id_salt("lasso-reference").selected_text(format!("{:?}",s.reference)).show_ui(ui,|ui|{ui.selectable_value(&mut s.reference,Reference::Current,"Current raster");ui.selectable_value(&mut s.reference,Reference::Composite,"Visible composite");if let Some(n)=self.active{for l in &self.documents[n].scene.lines.layers{for p in &l.paths{ui.selectable_value(&mut s.reference,Reference::Vector(p.id),format!("{} / path {}",l.name,p.id));}}}});
            egui::ComboBox::from_id_salt("lasso-input").selected_text(format!("{:?}",s.input)).show_ui(ui,|ui|{ui.selectable_value(&mut s.input,Input::SingleStroke,"Single stroke");ui.selectable_value(&mut s.input,Input::Multiple,"Multiple input (freehand and polyline)");});
            egui::ComboBox::from_id_salt("lasso-scope").selected_text(format!("{:?}",s.scope)).show_ui(ui,|ui|{for scope in [Scope::All,Scope::FreehandOnly,Scope::PolylineOnly]{ui.selectable_value(&mut s.scope,scope,format!("{scope:?}"));}});
            ui.add(egui::Slider::new(&mut s.distance,0.0..=64.0).text("Magnetic radius (document pixels)"));ui.add(egui::Slider::new(&mut s.contrast,1..=255).text("Raster edge contrast"));
            if *s!=old{self.finish_stroke();}
            ui.label("Lasso: drag a freehand section or click a polyline vertex. Multiple input keeps the draft between contacts; Enter closes, Backspace removes the latest input point/section, Escape cancels. Polygon uses polyline scope. Changes to reference/settings cancel the draft.");
        });
    }
}

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    }
}
pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::magnetic_lasso_verify::verify(root)?;
    for pen in [false, true] {
        for vector in [false, true] {
            for multiple in [false, true] {
                for scope in [Scope::All, Scope::FreehandOnly, Scope::PolylineOnly] {
                    if !multiple && scope != Scope::All {
                        continue;
                    }
                    let ctx = egui::Context::default();
                    let mut app = Drawer::new(&ctx, None);
                    app.simple_view = true;
                    app.create_canvas(CanvasSpec {
                        width: 16,
                        height: 16,
                        dpi: 300.0,
                        background: Background::Transparent,
                    })
                    .map_err(|e| e.to_string())?;
                    for y in 2..10 {
                        for x in 2..10 {
                            app.documents[0].layers[0].pixels[y * 16 + x] = [80, 90, 100, 200];
                        }
                    }
                    let path = if vector {
                        let layer = app.documents[0]
                            .scene
                            .lines
                            .add_layer("Magnetic reference")?;
                        Some(app.documents[0].scene.lines.add_path(
                            layer,
                            vec![[2.0, 2.0], [10.0, 2.0], [10.0, 10.0], [2.0, 10.0]],
                            true,
                            6.0,
                            [0, 0, 0, 255],
                        )?)
                    } else {
                        None
                    };
                    app.documents[0].view.zoom = 4.0;
                    app.documents[0].view.angle = 90.0;
                    app.workspaces.store.current.magnetic_lasso = Settings {
                        enabled: true,
                        scope,
                        input: if multiple {
                            Input::Multiple
                        } else {
                            Input::SingleStroke
                        },
                        reference: path.map_or(Reference::Current, Reference::Vector),
                        distance: 1.0,
                        ..Default::default()
                    };
                    app.execute_command(&ctx, "select-lasso");
                    let mut time = 0.0;
                    let step = |app: &mut Drawer, time: &mut f64, events| {
                        *time += 0.1;
                        super::super::input_replay::frame(&ctx, app, *time, events);
                    };
                    let pointer = |app: &mut Drawer, time: &mut f64, p, phase, contact| {
                        let mut e = super::super::precision::pointer(&ctx, app, p, phase);
                        if pen {
                            e.source = Source {
                                kind: drawer_core::pointer_input::Kind::Pen,
                                device: 111,
                                contact,
                            };
                            e.pressure = Some(0.5);
                        }
                        app.drawing.virtual_events(vec![e], pen);
                        step(app, time, vec![]);
                    };
                    step(&mut app, &mut time, vec![]);
                    step(&mut app, &mut time, vec![]);
                    let before = app.documents[0].snapshot();
                    let history = app.documents[0].history_counts();
                    if multiple {
                        for (contact, points) in [
                            (1, vec![[2.2, 2.2]]),
                            (2, vec![[9.8, 2.2], [9.8, 9.8]]),
                            (3, vec![[2.2, 9.8]]),
                            (4, vec![[14.0, 14.0]]),
                        ] {
                            pointer(&mut app, &mut time, points[0], Phase::Down, contact);
                            for p in points.iter().skip(1) {
                                pointer(&mut app, &mut time, *p, Phase::Move, contact);
                            }
                            pointer(
                                &mut app,
                                &mut time,
                                *points.last().unwrap(),
                                Phase::Up,
                                contact,
                            );
                            if app.documents[0].snapshot() != before || !app.drawing.pending() {
                                return Err(format!(
                                    "Multi-lasso contact committed/lost draft pen={pen} vector={vector} scope={scope:?}: {:?}",
                                    app.status
                                ));
                            }
                        }
                        step(&mut app, &mut time, vec![key(egui::Key::Backspace, true)]);
                        step(&mut app, &mut time, vec![key(egui::Key::Backspace, false)]);
                        step(&mut app, &mut time, vec![key(egui::Key::Enter, true)]);
                        step(&mut app, &mut time, vec![key(egui::Key::Enter, false)]);
                    } else {
                        pointer(&mut app, &mut time, [2.2, 2.2], Phase::Down, 1);
                        for p in [[9.8, 2.2], [9.8, 9.8], [2.2, 9.8]] {
                            pointer(&mut app, &mut time, p, Phase::Move, 1);
                        }
                        if app.documents[0].snapshot() != before
                            || app
                                .drawing
                                .selection_preview(app.documents[0].id)
                                .is_none_or(|p| p.len() <= 4)
                        {
                            return Err("Magnetic freehand preview/source".into());
                        }
                        pointer(&mut app, &mut time, [2.2, 9.8], Phase::Up, 1);
                    }
                    let d = &app.documents[0];
                    let edge_left = if scope == Scope::FreehandOnly {
                        191
                    } else {
                        255
                    };
                    let edge_right = if scope == Scope::PolylineOnly {
                        191
                    } else {
                        255
                    };
                    if app.drawing.pending()
                        || d.layers != before.layers
                        || d.scene.lines != before.scene.lines
                        || d.history_counts().0 != history.0 + 1
                        || d.scene.selection_coverage([5.5, 5.5]) != 255
                        || d.scene.selection_coverage([2.5, 5.5]) != edge_left
                        || d.scene.selection_coverage([9.5, 5.5]) != edge_right
                        || d.scene.selection_coverage([0.5, 0.5]) != 0
                    {
                        return Err(format!(
                            "Magnetic result pen={pen} vector={vector} multi={multiple} scope={scope:?} edges={}/{} history={:?} status={:?}",
                            d.scene.selection_coverage([2.5, 5.5]),
                            d.scene.selection_coverage([9.5, 5.5]),
                            d.history_counts(),
                            app.status
                        ));
                    }
                    let after = d.snapshot();
                    if !app.documents[0].undo()
                        || app.documents[0].snapshot() != before
                        || !app.documents[0].redo()
                        || app.documents[0].snapshot() != after
                    {
                        return Err("Magnetic selection one Undo/Redo".into());
                    }
                    if !pen && !vector && !multiple {
                        app.documents[0].layers[0].pixels.fill([80, 90, 100, 200]);
                        app.workspaces.store.current.selection_shortcuts.commands =
                            vec![drawer_core::selection_shortcuts::Command::DeleteOutside];
                        step(&mut app, &mut time, vec![]);
                        step(&mut app, &mut time, vec![]);
                        let before = app.documents[0].snapshot();
                        let history = app.documents[0].history_counts();
                        let r = app
                            .selection_shortcuts
                            .buttons
                            .iter()
                            .find(|(c, _)| {
                                *c == drawer_core::selection_shortcuts::Command::DeleteOutside
                            })
                            .ok_or("Delete outside toolbar missing")?
                            .1;
                        for pressed in [true, false] {
                            step(
                                &mut app,
                                &mut time,
                                vec![
                                    egui::Event::PointerMoved(r.center()),
                                    egui::Event::PointerButton {
                                        pos: r.center(),
                                        button: egui::PointerButton::Primary,
                                        pressed,
                                        modifiers: Default::default(),
                                    },
                                ],
                            );
                        }
                        step(&mut app, &mut time, vec![]);
                        if app.documents[0].layers[0].pixels[0] != [0; 4]
                            || app.documents[0].layers[0].pixels[5 * 16 + 5] != [80, 90, 100, 200]
                            || app.documents[0].scene != before.scene
                            || app.documents[0].history_counts().0 != history.0 + 1
                        {
                            return Err(format!("Delete outside toolbar result: {:?}", app.status));
                        }
                        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                            return Err("Outside erase one Undo".into());
                        }
                        app.execute_command(&ctx, "selection-clear");
                        if app.command_reason("delete-outside-selection").is_none() {
                            return Err("Outside erase enabled without selection".into());
                        }
                        app.documents[0].scene.selection_mask =
                            Some(drawer_core::selection::Mask::default());
                        if app.command_reason("delete-outside-selection").is_none() {
                            return Err("Outside erase enabled for empty mask".into());
                        }
                    }
                    let before = app.documents[0].snapshot();
                    pointer(&mut app, &mut time, [2.2, 2.2], Phase::Down, 8);
                    app.workspaces.store.current.magnetic_lasso.distance = 2.0;
                    step(&mut app, &mut time, vec![]);
                    if app.drawing.pending() || app.documents[0].snapshot() != before {
                        return Err(
                            "Magnetic settings change did not cancel unchanged draft".into()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn magnetic_single_multiple_scopes_mouse_pen_delete_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
