use super::*;
use drawer_core::{
    pointer_input::{Decision, Phase, Source},
    vector_edit::{self as core, Modifier, Patch, Selection, Settings, Tool},
    vector_selection::Control,
};

pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    revision: u64,
    frame: u32,
    source: Source,
    projection: Projection,
    root: egui::Rect,
    layer: u64,
    settings: Settings,
    tool: Tool,
    color: [u8; 4],
    start: egui::Pos2,
    last: egui::Pos2,
    previous: Selection,
    selection: Selection,
    preview: Option<Document>,
}
pub(super) struct Hover {
    canvas_layer: egui::LayerId,
    document: u64,
    view: u64,
    point: egui::Pos2,
    candidates: Vec<core::Hit>,
    index: usize,
}
fn effective(ctx: &egui::Context, settings: Settings) -> Tool {
    if ctx.input(|i| match settings.temporary {
        Modifier::Control => i.modifiers.ctrl,
        Modifier::Alt => i.modifiers.alt,
        Modifier::Shift => i.modifiers.shift,
        Modifier::None => false,
    }) {
        Tool::Point
    } else {
        settings.tool
    }
}
fn same_candidates(a: &[core::Hit], b: &[core::Hit]) -> bool {
    a.iter()
        .map(|h| (h.path, h.control))
        .eq(b.iter().map(|h| (h.path, h.control)))
}
impl Gesture {
    fn fresh(&self, drawing: &Drawing, ctx: &egui::Context, doc: &Document) -> bool {
        self.document == doc.id
            && self.revision == doc.revision
            && self.frame == doc.current_frame
            && self.projection.view == doc.view
            && self.root == ctx.content_rect()
            && (self.projection.ppp - ctx.pixels_per_point() as f64).abs() < 1e-8
            && drawing.mode == crate::precision::Mode::VectorEdit
            && drawing.vector_layer == Some(self.layer)
            && drawing.vector_settings == self.settings
            && effective(ctx, drawing.vector_settings) == self.tool
    }
    fn patch(&self) -> Patch {
        match self.tool {
            Tool::Point => {
                let a = self.projection.point(self.start);
                let b = self.projection.point(self.last);
                Patch {
                    movement: Some([b[0] - a[0], b[1] - a[1]]),
                    ..Default::default()
                }
            }
            Tool::Pressure => Patch {
                pressure_delta: Some(
                    (self.last.x - self.start.x) as f64 * self.settings.sensitivity,
                ),
                ..Default::default()
            },
            Tool::Color => Patch {
                color: Some(self.color),
                ..Default::default()
            },
            Tool::Select => Patch::default(),
        }
    }
}
impl Drawing {
    pub(super) fn cancel_vector_edit(&mut self) {
        if let Some(s) = self.vector_capture.take() {
            self.vector_selection = s.previous;
            self.ownership.clear();
            self.vector_hover = None;
        }
    }
    pub(crate) fn vector_edit_preview(&self, id: u64) -> Option<&Document> {
        self.vector_capture
            .as_ref()
            .filter(|s| s.document == id)
            .and_then(|s| s.preview.as_ref())
    }
    pub(super) fn advance_vector_edit(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        // Consume canvas Tab before egui assigns focus to widgets. This method
        // only runs when text entry and modal UI have not captured shortcuts.
        if self.mode == crate::precision::Mode::VectorEdit
            && self.vector_capture.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Tab))
            && let Some(h) = &mut self.vector_hover
            && self.vector_document == Some(h.document)
            && !h.candidates.is_empty()
        {
            let pos = self
                .events
                .iter()
                .rev()
                .find_map(|(e, _)| match e {
                    drawer_core::touch::Effect::Pointer(e) => {
                        Some(egui::pos2(e.position[0] as f32, e.position[1] as f32))
                    }
                    _ => None,
                })
                .or_else(|| ctx.input(|i| i.pointer.hover_pos()))
                .unwrap_or(h.point);
            if ctx.layer_id_at(pos) == Some(h.canvas_layer)
                && pos.distance(h.point) <= self.vector_settings.radius as f32
            {
                h.index = (h.index + 1) % h.candidates.len();
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
                });
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            }
        }
        if self.vector_capture.as_ref().is_some_and(|s| {
            docs.iter()
                .find(|d| d.id == s.document)
                .is_none_or(|d| !s.fresh(self, ctx, d))
        }) {
            self.cancel_vector_edit();
        }
        for index in 0..self.events.len() {
            let Some(id) = self.vector_capture.as_ref().map(|s| s.document) else {
                break;
            };
            if let Some(d) = docs.iter_mut().find(|d| d.id == id) {
                self.receive_vector_edit(index, ctx, d);
            }
        }
    }
    fn receive_vector_edit(&mut self, index: usize, ctx: &egui::Context, doc: &mut Document) {
        if self.events[index].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(e) = self.events[index].0 else {
            return;
        };
        let Some(mut s) = self.vector_capture.take() else {
            return;
        };
        if s.document != doc.id || s.source != e.source {
            self.vector_capture = Some(s);
            return;
        }
        self.events[index].1 = true;
        if !s.fresh(self, ctx, doc) || e.phase == Phase::Cancel {
            self.vector_selection = s.previous;
            self.ownership.clear();
            return;
        }
        if matches!(e.phase, Phase::Move | Phase::Up) {
            s.last = egui::pos2(e.position[0] as f32, e.position[1] as f32);
            let result = core::prepare(doc, s.layer, &s.selection, s.patch());
            match result {
                Ok(state) => {
                    if e.phase == Phase::Up {
                        self.ownership.clear();
                        // A click can select locally without dirtying the page.
                        // Color clicks deliberately recolor the selected stroke(s).
                        let changed = matches!(s.tool, Tool::Color | Tool::Select)
                            || s.start.distance(s.last) > 0.01;
                        if changed && let Err(error) = doc.commit_state(state) {
                            self.error = Some(error);
                        }
                        return;
                    }
                    match bucket::preview(doc, state) {
                        Ok(preview) => s.preview = Some(preview),
                        Err(error) => {
                            self.error = Some(error);
                            self.vector_selection = s.previous;
                            self.ownership.clear();
                            return;
                        }
                    }
                }
                Err(error) => {
                    self.error = Some(error);
                    self.vector_selection = s.previous;
                    self.ownership.clear();
                    return;
                }
            }
        }
        self.vector_capture = Some(s);
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn vector_edit_canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: u64,
        projection: Projection,
        color: [u8; 4],
        enabled: bool,
    ) {
        if self
            .vector_capture
            .as_ref()
            .is_some_and(|s| s.document == doc.id && (s.view != view || s.projection != projection))
        {
            self.cancel_vector_edit();
        }
        let Some(layer) = self.vector_layer else {
            self.vector_hover = None;
            return;
        };
        let tool = effective(ui.ctx(), self.vector_settings);
        let hover = self
            .events
            .iter()
            .rev()
            .find_map(|(e, _)| match e {
                drawer_core::touch::Effect::Pointer(e) => {
                    Some(egui::pos2(e.position[0] as f32, e.position[1] as f32))
                }
                _ => None,
            })
            .or_else(|| response.hover_pos())
            .or_else(|| {
                self.vector_hover
                    .as_ref()
                    .filter(|h| h.document == doc.id && h.view == view)
                    .map(|h| h.point)
            });
        if self.vector_capture.is_none() {
            if let Some(pos) = hover.filter(|p| {
                enabled
                    && response.rect.contains(*p)
                    && ui.ctx().layer_id_at(*p) == Some(response.layer_id)
            }) {
                let candidates = core::hits(
                    doc,
                    layer,
                    projection.point(pos),
                    self.vector_settings.radius * projection.ppp / projection.view.zoom,
                    tool,
                );
                let index = self
                    .vector_hover
                    .as_ref()
                    .filter(|h| {
                        h.document == doc.id
                            && h.view == view
                            && h.point.distance(pos) <= self.vector_settings.radius as f32
                            && same_candidates(&h.candidates, &candidates)
                    })
                    .map_or(0, |h| h.index);
                self.vector_hover = Some(Box::new(Hover {
                    canvas_layer: response.layer_id,
                    document: doc.id,
                    view,
                    point: pos,
                    candidates,
                    index,
                }));
            } else {
                self.vector_hover = None;
            }
        }
        for index in 0..self.events.len() {
            if self.vector_capture.is_some() {
                self.receive_vector_edit(index, ui.ctx(), doc);
                continue;
            }
            if self.events[index].1 {
                continue;
            }
            let drawer_core::touch::Effect::Pointer(e) = self.events[index].0 else {
                continue;
            };
            let pos = egui::pos2(e.position[0] as f32, e.position[1] as f32);
            if !enabled
                || e.phase != Phase::Down
                || !response.rect.contains(pos)
                || !ui.clip_rect().contains(pos)
                || ui.ctx().layer_id_at(pos) != Some(response.layer_id)
            {
                continue;
            }
            let candidates = core::hits(
                doc,
                layer,
                projection.point(pos),
                self.vector_settings.radius * projection.ppp / projection.view.zoom,
                tool,
            );
            let choice = self
                .vector_hover
                .as_ref()
                .filter(|h| {
                    h.document == doc.id
                        && h.view == view
                        && h.point.distance(pos) <= self.vector_settings.radius as f32
                        && same_candidates(&h.candidates, &candidates)
                })
                .map_or(0, |h| h.index);
            let Some(hit) = candidates.get(choice) else {
                continue;
            };
            if self.ownership.receive(e, self.settings) != Decision::Begin {
                continue;
            }
            self.events[index].1 = true;
            let previous = self.vector_selection.clone();
            let mut selection = previous.clone();
            if tool == Tool::Select {
                let mut paths = if ui.input(|i| i.modifiers.shift) {
                    previous.path_ids().into_iter().collect::<Vec<_>>()
                } else {
                    vec![]
                };
                if !paths.contains(&hit.path) {
                    paths.push(hit.path);
                }
                selection = Selection {
                    paths,
                    controls: vec![],
                };
            } else if tool == Tool::Color {
                if !selection.path_ids().contains(&hit.path) {
                    selection = Selection {
                        controls: vec![],
                        paths: vec![hit.path],
                    };
                }
            } else if let Some(point) = hit.control {
                let control = Control {
                    path: hit.path,
                    point,
                };
                if !selection.controls.contains(&control) {
                    if !ui.input(|i| i.modifiers.shift) {
                        selection = Selection::default();
                    }
                    selection.controls.push(control);
                }
                selection.paths.clear();
            }
            let s = Gesture {
                document: doc.id,
                view,
                revision: doc.revision,
                frame: doc.current_frame,
                source: e.source,
                projection,
                root: ui.ctx().content_rect(),
                layer,
                settings: self.vector_settings,
                tool,
                color,
                start: pos,
                last: pos,
                previous,
                selection: selection.clone(),
                preview: None,
            };
            self.vector_selection = selection;
            self.vector_capture = Some(Box::new(s));
        }
    }
    pub(crate) fn vector_edit_overlay(
        &self,
        ui: &egui::Ui,
        doc: &Document,
        original_id: u64,
        view: u64,
        rect: egui::Rect,
    ) {
        if self.mode != crate::precision::Mode::VectorEdit
            || self.vector_document != Some(original_id)
        {
            return;
        }
        let Some(layer) = doc
            .scene
            .lines
            .layers
            .iter()
            .find(|l| Some(l.id) == self.vector_layer)
        else {
            return;
        };
        let painter = ui.painter_at(rect);
        let project = |point| {
            let p = doc.view.document_to_screen(
                point,
                [doc.width as f64, doc.height as f64],
                [rect.center().x as f64, rect.center().y as f64],
                ui.ctx().pixels_per_point() as f64,
            );
            egui::pos2(p[0] as f32, p[1] as f32)
        };
        for path in &layer.paths {
            if self.vector_selection.paths.contains(&path.id) {
                for (a, b) in path.segments() {
                    painter.line_segment(
                        [
                            project(doc.scene.transform.map(a)),
                            project(doc.scene.transform.map(b)),
                        ],
                        egui::Stroke::new(1.5, egui::Color32::YELLOW),
                    );
                }
            }
            for n in 0..path.control_count() {
                if let Some(p) = path.control_point(n) {
                    let p = project(doc.scene.transform.map(p));
                    let selected = self.vector_selection.controls.contains(&Control {
                        path: path.id,
                        point: n as u16,
                    });
                    painter.circle_filled(
                        p,
                        if selected { 4. } else { 2.5 },
                        if selected {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::LIGHT_BLUE
                        },
                    );
                }
            }
        }
        if let Some(h) = self
            .vector_hover
            .as_ref()
            .filter(|h| h.document == original_id && h.view == view)
            && let Some(hit) = h.candidates.get(h.index)
        {
            painter.circle_stroke(
                project(hit.point),
                7.,
                egui::Stroke::new(2., egui::Color32::WHITE),
            );
            painter.text(
                h.point + egui::vec2(12., 12.),
                egui::Align2::LEFT_TOP,
                format!("{} / {} · Tab", h.index + 1, h.candidates.len()),
                egui::FontId::proportional(12.),
                egui::Color32::WHITE,
            );
        }
        if let Some(s) = self
            .vector_capture
            .as_ref()
            .filter(|s| s.document == original_id && s.view == view)
        {
            let value = core::values(doc, s.layer, &s.selection).ok();
            let label = match s.tool {
                Tool::Pressure => value
                    .and_then(|v| v.pressure)
                    .map_or_else(|| "Pressure: mixed".into(), |p| format!("Pressure: {p:.2}")),
                Tool::Point => {
                    let delta = s.patch().movement.unwrap_or_default();
                    format!("Move: {:.2}, {:.2}", delta[0], delta[1])
                }
                Tool::Color => format!("RGBA: {:?}", s.color),
                Tool::Select => format!("{} whole strokes", s.selection.path_ids().len()),
            };
            painter.text(
                s.last + egui::vec2(12., -18.),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(14.),
                egui::Color32::YELLOW,
            );
        }
    }
}

#[derive(Default)]
pub(crate) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: Option<u64>,
    selection: Selection,
    width: f64,
    color: [u8; 4],
    pressure: f64,
    movement: Point,
    set_width: bool,
    set_color: bool,
    set_pressure: bool,
    set_movement: bool,
    prepared: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Draft {
    fn new(d: &Document, layer: Option<u64>, selection: Selection) -> Self {
        let value = layer.and_then(|l| core::values(d, l, &selection).ok());
        Self {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            selection,
            width: value.as_ref().and_then(|v| v.width).unwrap_or(1.),
            color: value
                .as_ref()
                .and_then(|v| v.color)
                .unwrap_or([0, 0, 0, 255]),
            pressure: value.and_then(|v| v.pressure).unwrap_or(1.),
            movement: [0.; 2],
            set_width: false,
            set_color: false,
            set_pressure: false,
            set_movement: false,
            prepared: None,
            images: None,
        }
    }
    fn patch(&self) -> Patch {
        Patch {
            movement: self.set_movement.then_some(self.movement),
            pressure: self.set_pressure.then_some(self.pressure),
            width: self.set_width.then_some(self.width),
            color: self.set_color.then_some(self.color),
            ..Default::default()
        }
    }
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
}
impl Drawer {
    pub(crate) fn begin_vector_edit(&mut self, tool: Option<Tool>) -> Result<(), String> {
        self.finish_stroke();
        self.restore_locked_tool();
        let n = self.active.ok_or("Open a page first")?;
        if let Some(tool) = tool {
            self.workspaces.store.current.vector_edit.tool = tool;
        }
        self.drawing.mode = crate::precision::Mode::VectorEdit;
        self.drawing.connect_line = false;
        let d = &self.documents[n];
        let layer = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
            _ => None,
        };
        self.drawing.vector_document = Some(d.id);
        self.drawing.vector_layer = layer;
        self.drawing.vector_selection = layer.map(|l| Selection::current(d, l)).unwrap_or_default();
        self.drawing.vector_settings = self.workspaces.store.current.vector_edit;
        if tool.is_none() {
            self.vector_edit.draft = Some(Box::new(Draft::new(
                d,
                layer,
                self.drawing.vector_selection.clone(),
            )));
        }
        Ok(())
    }
    fn preview_vector_numbers(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Vector preview exceeds workspace memory".into());
        }
        let e = self
            .vector_edit
            .draft
            .as_mut()
            .ok_or("Vector editor closed")?;
        if !e.fresh(d)
            || e.layer
                != match self.panels.targets.get(&d.id) {
                    Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
                    _ => None,
                }
        {
            return Err("Page/frame changed; reload the current selection".into());
        }
        if !(e.set_width || e.set_color || e.set_pressure || e.set_movement) {
            return Err("Choose at least one property to change".into());
        }
        let state = core::prepare(
            d,
            e.layer.ok_or("Choose an editable vector layer")?,
            &e.selection,
            e.patch(),
        )?;
        let after = bucket::preview(d, state.clone())?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "vector-edit-before"),
            crate::layer_merge::image(ctx, &after, "vector-edit-after"),
        ]);
        e.prepared = Some(state);
        Ok(())
    }
    fn apply_vector_numbers(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .vector_edit
            .draft
            .as_mut()
            .ok_or("Vector editor closed")?;
        if !e.fresh(&self.documents[n])
            || e.layer
                != match self.panels.targets.get(&self.documents[n].id) {
                    Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
                    _ => None,
                }
        {
            return Err("Page/frame changed; reload the current selection".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare the vector preview first")?;
        self.documents[n].commit_state(state)?;
        self.drawing.vector_selection = e.selection.clone();
        self.vector_edit.draft = Some(Box::new(Draft::new(
            &self.documents[n],
            e.layer,
            e.selection.clone(),
        )));
        Ok(())
    }
    pub(crate) fn vector_edit_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.vector_edit.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut reload, mut changed) = (false, false, false, false);
        let fresh = e.fresh(d) && e.layer == self.drawing.vector_layer;
        egui::Window::new("Vector points, width, color and pressure").open(&mut open).collapsible(false).default_width(340.).default_pos(egui::pos2(645.,55.)).show(ctx, |ui| {
            let settings = &mut self.workspaces.store.current.vector_edit;
            ui.horizontal(|ui| { for (tool,label) in [(Tool::Point,"Point"),(Tool::Pressure,"Pressure"),(Tool::Color,"Color"),(Tool::Select,"Stroke")] { ui.radio_value(&mut settings.tool,tool,label); } });
            ui.add(egui::DragValue::new(&mut settings.sensitivity).speed(0.001).range(0.001..=1.).prefix("Pressure / screen point: "));
            ui.add(egui::DragValue::new(&mut settings.radius).range(2. ..=32.).prefix("Pick radius: "));
            egui::ComboBox::from_id_salt("vector-temporary").selected_text(format!("Temporary point tool: {:?}",settings.temporary)).show_ui(ui,|ui|{ for m in [Modifier::Control,Modifier::Alt,Modifier::Shift,Modifier::None] { ui.selectable_value(&mut settings.temporary,m,format!("{m:?}")); } });
            ui.small("Drag points to move. Pressure uses horizontal screen distance; color clicks use the current drawing color. Tab cycles overlapping candidates. Shift adds points. Escape cancels a drag.");
            ui.separator();
            ui.horizontal(|ui| { reload = ui.button("Load current selection").clicked(); if ui.button("All points in this layer").clicked() && let Some(l) = d.scene.lines.layers.iter().find(|l| Some(l.id)==self.drawing.vector_layer) {
                self.drawing.vector_selection = Selection { controls: l.paths.iter().flat_map(|p| (0..p.control_count()).map(move |n| Control { path:p.id,point:n as u16 })).collect(), paths:vec![] }; reload=true;
            } });
            if !fresh { ui.colored_label(egui::Color32::YELLOW,"Page, frame or layer changed. Load the current selection before editing."); }
            let values = e.layer.and_then(|l| core::values(d,l,&e.selection).ok());
            ui.label(values.as_ref().map_or_else(|| "Select points on an editable vector layer.".into(), |v| format!("{} paths · {} selected points",v.paths,v.controls)));
            ui.add_enabled_ui(fresh && values.is_some(), |ui| {
                ui.horizontal(|ui| { changed |= ui.checkbox(&mut e.set_width,"Set width").changed(); changed |= ui.add(egui::DragValue::new(&mut e.width).speed(0.1).range(0. ..=512.)).changed(); if values.as_ref().is_some_and(|v| v.width.is_none()) { ui.small("Mixed"); } });
                ui.horizontal(|ui| { changed |= ui.checkbox(&mut e.set_color,"Set RGBA").changed(); let old=e.color; crate::edit::rgba(ui,&mut e.color); changed |= old!=e.color; if ui.button("Current color").clicked() { e.color=self.references.session.workspace.color; e.set_color=true;changed=true; } });
                if values.as_ref().is_some_and(|v| v.color.is_none()) { ui.small("Stroke colors: mixed"); }
                ui.horizontal(|ui| { changed |= ui.checkbox(&mut e.set_pressure,"Set pressure multiplier").changed(); changed |= ui.add(egui::DragValue::new(&mut e.pressure).speed(0.01).range(0. ..=2.)).changed(); if ui.button("Reset 1.00").clicked() { e.pressure=1.;e.set_pressure=true;changed=true; } });
                if values.as_ref().is_some_and(|v| v.pressure.is_none()) { ui.small("Point multipliers: mixed or no selected points"); }
                ui.horizontal(|ui| { changed |= ui.checkbox(&mut e.set_movement,"Move selected points").changed(); for (v,label) in e.movement.iter_mut().zip(["X: ","Y: "]) { changed |= ui.add(egui::DragValue::new(v).range(-1e9..=1e9).prefix(label)).changed(); } });
                ui.small("Device samples remain separate from the editable multiplier. Zero width/pressure removes stroke coverage; filled interiors remain.");
            });
            if changed { e.prepared=None;e.images=None; }
            let r=ui.add_enabled(fresh && values.is_some(),egui::Button::new("Prepare numeric preview"));self.vector_edit.preview=Some(r.rect);preview=r.clicked();
            if let Some(images)=&e.images { ui.horizontal(|ui| {for (label,image) in ["Before","After"].into_iter().zip(images) {ui.vertical(|ui| {ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});} }); }
            let r=ui.add_enabled(fresh && e.prepared.is_some(),egui::Button::new("Apply numeric edit"));self.vector_edit.apply=Some(r.rect);apply=r.clicked();
        });
        if reload {
            e = Box::new(Draft::new(
                d,
                self.drawing.vector_layer,
                self.drawing.vector_selection.clone(),
            ));
        }
        if open {
            self.vector_edit.draft = Some(e);
        }
        if preview && let Err(error) = self.preview_vector_numbers(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_vector_numbers() {
            self.status = Some(error);
        }
    }
}

pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_edit_verify::verify(root)?;
    for zoom in [1., 3.] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let d = &mut app.documents[0];
        d.view.zoom = zoom;
        let layer = d.scene.lines.add_layer("Edit target")?;
        let path = d.scene.lines.add_path(
            layer,
            vec![[12., 12.], [40., 12.], [40., 40.]],
            false,
            4.,
            [200, 20, 40, 255],
        )?;
        d.scene.lines.layers[0].paths[0].pressure.device = vec![Some(0.5), Some(0.8), None];
        app.panels
            .targets
            .insert(d.id, drawer_core::thumbnail::Target::Vector(layer));
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            crate::input_replay::frame(&ctx, app, *time, events);
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        app.execute_command(&ctx, "vector-pressure-tool");
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let down = crate::precision::pointer(&ctx, &app, [40., 12.], Phase::Down);
        app.drawing.virtual_events(vec![down], false);
        step(&mut app, &mut time, vec![]);
        if !app.drawing.pending() {
            return Err(format!("Pressure tool did not capture: {:?}", app.status));
        }
        let mut moved = down;
        moved.phase = Phase::Move;
        moved.position[0] += 25.;
        app.drawing.virtual_events(vec![moved], false);
        step(&mut app, &mut time, vec![]);
        let p = app
            .drawing
            .vector_edit_preview(app.documents[0].id)
            .ok_or("Pressure preview missing")?;
        if p.scene.lines.layers[0].paths[0].pressure.controls != [1., 1.25, 1.]
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
        {
            return Err("Screen-distance pressure preview/source isolation".into());
        }
        moved.phase = Phase::Up;
        app.drawing.virtual_events(vec![moved], false);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].history_counts().0 != history.0 + 1
            || app.documents[0].scene.lines.layers[0].paths[0]
                .pressure
                .device
                != [Some(0.5), Some(0.8), None]
        {
            return Err("Pressure commit/device samples/one Undo".into());
        }
        let file = root.join(format!("vector-edited-{zoom}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Mouse pressure save/reopen/Undo".into());
        }
        // Esc, focus loss and input cancellation restore local selection as well
        // as leaving all document data and history untouched.
        for cancel in 0..3 {
            let selection = app.drawing.vector_selection.clone();
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            app.drawing.virtual_events(vec![down], false);
            step(&mut app, &mut time, vec![]);
            moved.phase = Phase::Move;
            app.drawing.virtual_events(vec![moved], false);
            step(&mut app, &mut time, vec![]);
            let events = match cancel {
                0 => vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                1 => vec![egui::Event::WindowFocused(false)],
                _ => {
                    let mut e = moved;
                    e.phase = Phase::Cancel;
                    app.drawing.virtual_events(vec![e], false);
                    vec![]
                }
            };
            step(&mut app, &mut time, events);
            if app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts() != history
                || app.drawing.vector_selection != selection
            {
                return Err(format!("Vector drag cancellation {cancel}"));
            }
            step(
                &mut app,
                &mut time,
                vec![
                    egui::Event::WindowFocused(true),
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        app.execute_command(&ctx, "vector-point-tool");
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        let mut e = crate::precision::pointer(&ctx, &app, [12., 12.], Phase::Down);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
        e = crate::precision::pointer(&ctx, &app, [15., 10.], Phase::Up);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        if p.points != [[15., 10.], [40., 12.], [40., 40.]]
            || app.documents[0].history_counts().0 != count.0 + 1
        {
            return Err("Partial mouse point move".into());
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Point move Undo".into());
        }
        app.choose_color([30, 190, 80, 128]);
        app.execute_command(&ctx, "vector-color-tool");
        // Click the curve away from every control point.
        for phase in [Phase::Down, Phase::Up] {
            let e = crate::precision::pointer(&ctx, &app, [25., 12.], phase);
            app.drawing.virtual_events(vec![e], false);
            step(&mut app, &mut time, vec![]);
        }
        if app.documents[0].scene.lines.layers[0].paths[0].color != [30, 190, 80, 128]
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Curve click whole-stroke color/Undo".into());
        }
        // Modifier-only transitions never commit an edit. Test the exact input
        // state read by the production tool, including returning to Pressure.
        app.execute_command(&ctx, "vector-pressure-tool");
        let count = app.documents[0].history_counts();
        for ctrl in [true, false, true, false] {
            time += 0.1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 700.),
                    )),
                    time: Some(time),
                    events: vec![egui::Event::ModifiersChanged(egui::Modifiers {
                        ctrl,
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                |ui| {
                    app.show(ui);
                    if effective(ui.ctx(), app.workspaces.store.current.vector_edit)
                        != (if ctrl { Tool::Point } else { Tool::Pressure })
                    {
                        app.status = Some("Incorrect temporary vector tool".into());
                    }
                },
            );
            output.textures_delta.clear();
        }
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != count
            || app.status.as_deref() == Some("Incorrect temporary vector tool")
        {
            return Err("Temporary tool switching changed document/history".into());
        }
        // Both an empty hit and a locked/raster target must remain inert.
        for target in 0..3 {
            if target == 1 {
                app.documents[0].scene.lines.layers[0].locked = true;
            }
            if target == 2 {
                app.panels.targets.insert(
                    app.documents[0].id,
                    drawer_core::thumbnail::Target::Raster(0),
                );
            }
            let before = app.documents[0].snapshot();
            let count = app.documents[0].history_counts();
            for phase in [Phase::Down, Phase::Up] {
                let e = crate::precision::pointer(
                    &ctx,
                    &app,
                    if target == 0 { [60., 60.] } else { [40., 12.] },
                    phase,
                );
                app.drawing.virtual_events(vec![e], false);
                step(&mut app, &mut time, vec![]);
            }
            if app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts() != count
            {
                return Err(format!("Vector inert target {target}"));
            }
        }
        app.documents[0].scene.lines.layers[0].locked = false;
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Vector(layer),
        );
        app.execute_command(&ctx, "vector-edit-tools");
        app.drawing.vector_selection = Selection {
            controls: vec![Control { path, point: 0 }, Control { path, point: 2 }],
            paths: vec![],
        };
        app.vector_edit.draft = Some(Box::new(Draft::new(
            &app.documents[0],
            Some(layer),
            app.drawing.vector_selection.clone(),
        )));
        {
            let e = app.vector_edit.draft.as_mut().unwrap();
            e.set_pressure = true;
            e.pressure = 0.;
            e.set_width = true;
            e.width = 6.;
        }
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                step(
                    app,
                    time,
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
        };
        let r = app
            .vector_edit
            .preview
            .ok_or("Numeric preview button missing")?;
        click(&mut app, &mut time, r);
        if app
            .vector_edit
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
            || app.documents[0].snapshot() != before
        {
            return Err(format!("Numeric preview failed: {:?}", app.status));
        }
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let r = app
            .vector_edit
            .apply
            .ok_or("Numeric apply button missing")?;
        click(&mut app, &mut time, r);
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        if p.pressure.controls != [0., 1., 0.]
            || p.width != 6.
            || app.documents[0].history_counts().0 != count.0 + 1
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err(format!("Numeric partial edit and Undo: {:?}", app.status));
        }
        app.vector_edit.draft = None;
    }
    smoke_edges(root)
}
fn smoke_edges(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 64,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let d = &mut app.documents[0];
    let layer = d.scene.lines.add_layer("Crossing targets")?;
    let a = d.scene.lines.add_path(
        layer,
        vec![[12., 12.], [40., 12.]],
        false,
        2.,
        [0, 0, 0, 255],
    )?;
    let b = d.scene.lines.add_path(
        layer,
        vec![[12., 12.], [12., 40.]],
        false,
        2.,
        [0, 0, 0, 255],
    )?;
    app.panels
        .targets
        .insert(d.id, drawer_core::thumbnail::Target::Vector(layer));
    let mut time = 0.;
    let step = |app: &mut Drawer, t: &mut f64, events| {
        *t += 0.1;
        crate::input_replay::frame(&ctx, app, *t, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "vector-pressure-tool");
    let e = crate::precision::pointer(&ctx, &app, [12., 12.], Phase::Move);
    app.drawing.virtual_events(vec![e], false);
    step(&mut app, &mut time, vec![]);
    if app
        .drawing
        .vector_hover
        .as_ref()
        .is_none_or(|h| h.candidates.len() != 2 || h.candidates[h.index].path != b)
    {
        return Err("Crossing candidate order/preview".into());
    }
    step(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app
        .drawing
        .vector_hover
        .as_ref()
        .is_none_or(|h| h.candidates[h.index].path != a)
    {
        return Err("Tab did not cycle crossing candidates".into());
    }
    step(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    let mut down = e;
    down.phase = Phase::Down;
    app.drawing.virtual_events(vec![down], false);
    step(&mut app, &mut time, vec![]);
    if app.drawing.vector_selection.controls != [Control { path: a, point: 0 }] {
        return Err(format!(
            "Cycled crossing picked another stroke: selection={:?}, capture={}, hover={:?}, status={:?}",
            app.drawing.vector_selection,
            app.drawing.vector_capture.is_some(),
            app.drawing
                .vector_hover
                .as_ref()
                .map(|h| (h.index, &h.candidates)),
            app.status
        ));
    }
    let mut moved = down;
    moved.phase = Phase::Move;
    moved.position[0] += 12.;
    app.drawing.virtual_events(vec![moved], false);
    step(&mut app, &mut time, vec![]);
    step(
        &mut app,
        &mut time,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL)],
    );
    if app.drawing.pending()
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
        || !app.drawing.vector_selection.controls.is_empty()
    {
        return Err("Modifier change did not cancel old pressure drag/selection".into());
    }
    step(
        &mut app,
        &mut time,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    );
    app.set_tool(BrushTip::PixelPen);
    app.panels.settings.brush_size = 4.;
    app.workspaces.store.current.pressure.tools[6].size = true;
    app.workspaces.store.current.auto_shape.enabled = false;
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    for (phase, point, pressure) in [
        (Phase::Down, [20., 24.], 0.25),
        (Phase::Move, [48., 24.], 0.75),
        (Phase::Up, [48., 24.], 0.75),
    ] {
        let mut e = crate::precision::pointer(&ctx, &app, point, phase);
        e.source = Source {
            kind: drawer_core::pointer_input::Kind::Pen,
            device: 41,
            contact: 1,
        };
        e.pressure = Some(pressure);
        app.drawing.virtual_events(vec![e], true);
        step(&mut app, &mut time, vec![]);
        if phase == Phase::Down && !app.drawing.pending() {
            return Err(format!(
                "Vector pen down not captured: events={:?}, owner={:?}, layer={:?}, point={:?}, viewport={:?}, top={:?}, canvas_layer={:?}, pen={:?}, blocked={}",
                app.drawing.events,
                app.drawing.ownership.owner(),
                app.drawing.auto_target,
                e.position,
                app.canvas_view.viewport_rect(),
                ctx.layer_id_at(egui::pos2(e.position[0] as f32, e.position[1] as f32)),
                app.canvas_view.viewport_layer(),
                app.drawing.settings.pen,
                app.tool_shortcuts_blocked(&ctx)
            ));
        }
        if phase == Phase::Move && !app.drawing.pending() {
            return Err(format!(
                "Vector pen lost during Move: events={:?}, mode={:?}, target={:?}, auto={:?}, view={:?}, status={:?}",
                app.drawing.events,
                app.drawing.mode,
                app.drawing.auto_target,
                app.drawing.auto_settings,
                app.documents[0].view,
                app.status
            ));
        }
    }
    let paths = &app.documents[0].scene.lines.layers[0].paths;
    if paths.len() != 3
        || paths[2].pressure.device != [Some(0.25), Some(0.75)]
        || !paths[2].pressure.device_enabled
        || !paths[2].pressure.controls.is_empty()
        || app.documents[0].history_counts().0 != count.0 + 1
    {
        return Err(format!(
            "Vector device pressure without automatic shape correction: paths={}, last={:?}, history={:?}/{count:?}, mode={:?}, pending={}, status={:?}",
            paths.len(),
            paths.last().map(|p| &p.pressure),
            app.documents[0].history_counts(),
            app.drawing.mode,
            app.drawing.pending(),
            app.status
        ));
    }
    let path = root.join("vector-device-input.drawer");
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err("Vector device save/reopen/Undo".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_mouse_pressure_points_color_numbers_cancellation_and_temporary_tool() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
