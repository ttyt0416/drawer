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
    planned: Option<Box<drawer_core::sizing::DocumentState>>,
    trace: Vec<Point>,
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
            Tool::Select | Tool::Push | Tool::Redraw => Patch::default(),
            Tool::AddNode | Tool::DeleteNode | Tool::CornerNode | Tool::SmoothNode => {
                Patch::default()
            }
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
            if s.planned.is_some() {
                if e.phase == Phase::Up {
                    self.ownership.clear();
                    if s.start.distance(s.last) > s.settings.radius as f32 {
                        self.vector_selection = s.previous;
                        return;
                    }
                    if let Err(error) = doc.commit_state(*s.planned.take().unwrap()) {
                        self.error = Some(error);
                        self.vector_selection = s.previous;
                    }
                    return;
                }
                self.vector_capture = Some(s);
                return;
            }
            let result = match s.tool {
                Tool::Push => {
                    let a = s.projection.point(s.start);
                    let b = s.projection.point(s.last);
                    drawer_core::vector_refine::push(
                        doc,
                        s.layer,
                        &s.selection,
                        a,
                        [b[0] - a[0], b[1] - a[1]],
                        s.settings.push_radius,
                        s.settings.ends,
                    )
                }
                Tool::Redraw => {
                    let point = s.projection.point(s.last);
                    if s.trace.last() != Some(&point) {
                        if s.trace.len() >= 128 {
                            self.error = Some("Redraw trace limit128; use a shorter stroke".into());
                            self.vector_selection = s.previous;
                            self.ownership.clear();
                            return;
                        }
                        s.trace.push(point);
                    }
                    if s.trace.len() < 2 {
                        self.vector_capture = Some(s);
                        return;
                    }
                    drawer_core::vector_refine::redraw(
                        doc,
                        s.layer,
                        &s.selection,
                        &s.trace,
                        s.settings.ends,
                        s.settings.curve,
                    )
                }
                _ => core::prepare(doc, s.layer, &s.selection, s.patch()),
            };
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
                        if matches!(s.tool, Tool::Push | Tool::Redraw) {
                            self.vector_selection = Selection::current(doc, s.layer);
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
                    if s.tool == Tool::Redraw && error.starts_with("Drag to a different control") {
                        if e.phase != Phase::Up {
                            self.vector_capture = Some(s);
                        } else {
                            self.vector_selection = s.previous;
                            self.ownership.clear();
                        }
                        return;
                    }
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
            let mut planned = None;
            let mut preview = None;
            if let Some(op) = tool.node_operation() {
                match drawer_core::vector_nodes::prepare(doc, layer, *hit, op)
                    .and_then(|state| bucket::preview(doc, state.clone()).map(|p| (state, p)))
                {
                    Ok((state, p)) => {
                        selection = Selection {
                            controls: state.scene.lines.selection.points.clone(),
                            paths: state.scene.lines.selection.paths.clone(),
                        };
                        planned = Some(Box::new(state));
                        preview = Some(p);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.ownership.clear();
                        continue;
                    }
                }
            } else if tool == Tool::Select {
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
            } else if matches!(tool, Tool::Color | Tool::Push) {
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
                preview,
                planned,
                trace: vec![projection.point(pos)],
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
        if effective(ui.ctx(), self.vector_settings) == Tool::Push
            && let Some(h) = self
                .vector_hover
                .as_ref()
                .filter(|h| h.document == original_id && h.view == view)
        {
            painter.circle_stroke(
                h.point,
                (self.vector_settings.push_radius * doc.view.zoom
                    / ui.ctx().pixels_per_point() as f64) as f32,
                egui::Stroke::new(1., egui::Color32::LIGHT_BLUE),
            );
        }
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
            let controls = if effective(ui.ctx(), self.vector_settings)
                .node_operation()
                .is_some()
            {
                drawer_core::vector_nodes::controls(path)
            } else {
                (0..path.control_count())
                    .filter_map(|n| path.control_point(n))
                    .collect()
            };
            for (n, p) in controls.into_iter().enumerate() {
                {
                    let p = project(doc.scene.transform.map(p));
                    let selected = self.vector_selection.controls.contains(&Control {
                        path: path.id,
                        point: n as u16,
                    });
                    let color = if selected {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::LIGHT_BLUE
                    };
                    let nodes = path.definition.as_ref().and_then(|d| d.nodes.as_ref());
                    if (nodes.is_some()
                        || effective(ui.ctx(), self.vector_settings)
                            .node_operation()
                            .is_some())
                        && !nodes.is_some_and(|d| d.smooth.get(n) == Some(&true))
                    {
                        painter.rect_filled(
                            egui::Rect::from_center_size(p, egui::vec2(6., 6.)),
                            0.,
                            color,
                        );
                    } else {
                        painter.circle_filled(p, if selected { 4. } else { 2.5 }, color);
                    }
                    if selected
                        && let Some(def) = &path.definition
                        && let Some(nodes) = &def.nodes
                        && n < def.controls.len()
                    {
                        for handle in [nodes.incoming[n], nodes.outgoing[n]] {
                            let h = project(doc.scene.transform.map(def.creation.inverse().map([
                                def.controls[n][0] + handle[0],
                                def.controls[n][1] + handle[1],
                            ])));
                            painter.line_segment(
                                [p, h],
                                egui::Stroke::new(1., egui::Color32::LIGHT_BLUE),
                            );
                            painter.circle_stroke(
                                h,
                                2.5,
                                egui::Stroke::new(1., egui::Color32::LIGHT_BLUE),
                            );
                        }
                    }
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
                Tool::Push => format!("Local push · {:?}", s.settings.ends),
                Tool::Redraw => format!("Redraw span · {:?}", s.settings.ends),
                tool => tool.node_operation().map_or_else(String::new, |op| {
                    format!("{} · release to apply", op.label())
                }),
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
    reset: Option<bool>,
    refine: RefineOptions,
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
            reset: None,
            refine: Default::default(),
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
struct RefineOptions {
    tip: drawer_core::vector_tip::Preset,
    kind: u8,
    value: f64,
    width: drawer_core::vector_refine::Width,
    minimum: bool,
    alpha: u8,
    curve: drawer_core::shapes::Curve,
}
impl Default for RefineOptions {
    fn default() -> Self {
        Self {
            tip: Default::default(),
            kind: 0,
            value: 2.,
            width: Default::default(),
            minimum: false,
            alpha: 255,
            curve: drawer_core::shapes::Curve::StraightLine,
        }
    }
}
impl RefineOptions {
    fn operation(&self) -> drawer_core::vector_refine::Operation {
        use drawer_core::vector_refine::Operation as Op;
        match self.kind {
            0 => Op::Join(self.value),
            1 => Op::Simplify(self.value),
            2 => Op::Convert(self.curve),
            3 => Op::Width(self.width, self.value, self.minimum),
            4 => Op::Opacity(self.alpha),
            6 => Op::Tip(self.tip),
            _ => Op::Split,
        }
    }
}
impl Drawer {
    pub(crate) fn begin_pressure_reset(
        &mut self,
        ctx: &egui::Context,
        whole_layer: bool,
    ) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
            _ => None,
        };
        let selection =
            if self.drawing.vector_document == Some(d.id) && self.drawing.vector_layer == target {
                self.drawing.vector_selection.clone()
            } else {
                target
                    .map(|id| Selection::current(d, id))
                    .unwrap_or_default()
            };
        self.begin_vector_edit(None)?;
        self.drawing.vector_selection = selection.clone();
        self.vector_edit.draft.as_mut().unwrap().selection = selection;
        self.preview_pressure_reset(ctx, whole_layer)
    }
    fn preview_pressure_reset(
        &mut self,
        ctx: &egui::Context,
        whole_layer: bool,
    ) -> Result<(), String> {
        let e = self
            .vector_edit
            .draft
            .as_mut()
            .ok_or("Vector editor closed")?;
        e.reset = Some(whole_layer);
        e.prepared = None;
        e.images = None;
        self.preview_vector_edit(ctx, false)
    }
    pub(crate) fn begin_vector_tips(&mut self) -> Result<(), String> {
        self.begin_vector_edit(None)?;
        if let Some(e) = &mut self.vector_edit.draft {
            e.refine.kind = 6;
        }
        Ok(())
    }
    pub(crate) fn begin_vector_edit(&mut self, tool: Option<Tool>) -> Result<(), String> {
        self.finish_stroke();
        self.restore_locked_tool();
        let n = self.active.ok_or("Open a page first")?;
        if let Some(tool) = tool {
            self.workspaces.store.current.vector_edit.tool = tool;
            if tool.node_operation().is_some() {
                self.status=Some("Node tools use the stored outline points; preview before release. Undo restores the original curve definition.".into());
            }
        }
        self.drawing.mode = crate::precision::Mode::VectorEdit;
        self.drawing.connect_line = false;
        let d = &self.documents[n];
        let layer = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
            _ => None,
        };
        let selection =
            if self.drawing.vector_document == Some(d.id) && self.drawing.vector_layer == layer {
                self.drawing.vector_selection.clone()
            } else {
                layer.map(|l| Selection::current(d, l)).unwrap_or_default()
            };
        self.drawing.vector_document = Some(d.id);
        self.drawing.vector_layer = layer;
        self.drawing.vector_selection = selection;
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
        if let Some(e) = &mut self.vector_edit.draft {
            e.reset = None;
            e.prepared = None;
            e.images = None;
        }
        self.preview_vector_edit(ctx, false)
    }
    fn preview_vector_edit(&mut self, ctx: &egui::Context, refinement: bool) -> Result<(), String> {
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
        if refinement {
            e.reset = None;
            e.prepared = None;
            e.images = None;
        }
        if !refinement
            && e.reset.is_none()
            && !(e.set_width || e.set_color || e.set_pressure || e.set_movement)
        {
            return Err("Choose at least one property to change".into());
        }
        let state = if let Some(whole_layer) = e.reset {
            core::reset_pressure(
                d,
                e.layer.ok_or("Choose an editable vector layer")?,
                &e.selection,
                whole_layer,
            )?
        } else if refinement {
            drawer_core::vector_refine::prepare(
                d,
                e.layer.ok_or("Choose an editable vector layer")?,
                &e.selection,
                e.refine.operation(),
            )?
        } else {
            core::prepare(
                d,
                e.layer.ok_or("Choose an editable vector layer")?,
                &e.selection,
                e.patch(),
            )?
        };
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
        self.drawing.vector_selection = if e.reset.is_some() {
            e.selection.clone()
        } else {
            Selection::current(&self.documents[n], e.layer.unwrap())
        };
        self.vector_edit.draft = Some(Box::new(Draft::new(
            &self.documents[n],
            e.layer,
            self.drawing.vector_selection.clone(),
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
        let mut refine_preview = false;
        let mut reset_preview = None;
        let fresh = e.fresh(d) && e.layer == self.drawing.vector_layer;
        egui::Window::new("Vector points, width, color and pressure").open(&mut open).collapsible(false).default_width(340.).default_height(600.).vscroll(true).default_pos(egui::pos2(645.,55.)).show(ctx, |ui| {
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
            let r=ui.add_enabled(fresh && e.prepared.is_some(),egui::Button::new(if e.reset.is_some() {"Apply pressure reset"} else {"Apply numeric edit"}));self.vector_edit.apply=Some(r.rect);apply=r.clicked();
            ui.horizontal(|ui| {
                if ui.add_enabled(fresh && values.is_some(),egui::Button::new("Reset selected pressure…")).clicked() { reset_preview=Some(false); }
                if ui.add_enabled(fresh && e.layer.is_some(),egui::Button::new("Reset layer pressure…")).clicked() { reset_preview=Some(true); }
            });
            if let Some(all)=e.reset { ui.label(if all {"Preview: reset every stroke in this layer to 1.00"}else{"Preview: reset selected points / whole selected strokes to 1.00"}); }
            ui.small("Pressure reset keeps device samples, geometry, style and selection. Close/Escape discards the preview; Apply records one Undo.");
            ui.collapsing("Node tools",|ui| {
                ui.horizontal_wrapped(|ui|{for (tool,label) in [(Tool::AddNode,"Add node"),(Tool::DeleteNode,"Delete node"),(Tool::CornerNode,"Corner"),(Tool::SmoothNode,"Smooth")] {ui.radio_value(&mut self.workspaces.store.current.vector_edit.tool,tool,label);}});
                ui.small("Existing shape definitions use their stored outline points as nodes. Add inside a segment, delete a point, or switch its tangents. Preview on press; release applies one Undo. Escape cancels.");
            });
            egui::CollapsingHeader::new("Tip shape and stroke refinements").id_salt(if e.refine.kind==6 {"vector-tip-refinements"}else{"vector-other-refinements"}).default_open(e.refine.kind==6).show(ui,|ui| {
                let settings=&mut self.workspaces.store.current.vector_edit;
                ui.horizontal(|ui|{ui.radio_value(&mut settings.tool,Tool::Push,"Local push");ui.radio_value(&mut settings.tool,Tool::Redraw,"Redraw span");});
                ui.add(egui::DragValue::new(&mut settings.push_radius).range(0.1..=1e6).prefix("Push radius (document pixels): "));
                egui::ComboBox::from_id_salt("refine-ends").selected_text(format!("{:?}",settings.ends)).show_ui(ui,|ui|{for (ends,label) in [(drawer_core::vector_refine::Ends::Both,"Fix both ends"),(drawer_core::vector_refine::Ends::Either,"Fix nearest end"),(drawer_core::vector_refine::Ends::Free,"Free both ends")] {ui.selectable_value(&mut settings.ends,ends,label);}});
                egui::ComboBox::from_id_salt("refine-redraw-curve").selected_text(settings.curve.label()).show_ui(ui,|ui|{for c in drawer_core::shapes::Curve::ALL {ui.selectable_value(&mut settings.curve,c,if c==drawer_core::shapes::Curve::StraightLine {"Polyline"}else{c.label()});}});
                ui.small("Push a stroke near the cursor. Redraw from one control to another, or Shift-select two boundary controls first. Interior boundaries stay attached; the unselected outline stays unchanged. Hold the temporary point modifier to move controls.");
                const LABELS:[&str;7]=["Join endpoints","Simplify outline","Convert curve","Adjust line width","Adjust opacity","Split at selected point","Change tip preset"];
                egui::ComboBox::from_id_salt("refine-op").selected_text(LABELS[e.refine.kind as usize]).show_ui(ui,|ui|{for (i,label) in LABELS.iter().enumerate(){changed|=ui.selectable_value(&mut e.refine.kind,i as u8,*label).changed();}});
                if matches!(e.refine.kind,0|1|3) {changed|=ui.add(egui::DragValue::new(&mut e.refine.value).speed(0.1).range(0. ..=if e.refine.kind==3 {512.}else{1e6}).prefix(if e.refine.kind==0 {"Maximum join distance: "}else if e.refine.kind==1 {"Maximum error: "}else{"Amount / factor: "})).changed();}
                if e.refine.kind==2 {egui::ComboBox::from_id_salt("refine-curve").selected_text(e.refine.curve.label()).show_ui(ui,|ui|{for c in drawer_core::shapes::Curve::ALL {changed|=ui.selectable_value(&mut e.refine.curve,c,if c==drawer_core::shapes::Curve::StraightLine {"Polyline"}else{c.label()}).changed();}});}
                if e.refine.kind==3 {
                    use drawer_core::vector_refine::Width;
                    egui::ComboBox::from_id_salt("refine-width").selected_text(format!("{:?}",e.refine.width)).show_ui(ui,|ui|{for (m,label) in [(Width::Thicken,"Thicken"),(Width::Narrow,"Narrow"),(Width::ScaleUp,"Scale up width"),(Width::ScaleDown,"Scale down width"),(Width::Fix,"Fix width")] {changed|=ui.selectable_value(&mut e.refine.width,m,label).changed();}});
                    changed|=ui.checkbox(&mut e.refine.minimum,"At least 1 pixel (base width)").changed();
                    ui.small("Device pressure and editable multipliers remain independent, including zero pressure.");
                }
                if e.refine.kind==4 {changed|=ui.add(egui::DragValue::new(&mut e.refine.alpha).range(0..=255).prefix("Alpha: ")).changed();}
                if e.refine.kind==6 {
                    egui::ComboBox::from_id_salt("vector-tip-preset").selected_text(e.refine.tip.label()).show_ui(ui,|ui|{for tip in drawer_core::vector_tip::Preset::ALL {changed|=ui.selectable_value(&mut e.refine.tip,tip,tip.label()).changed();}});
                    ui.small("Embedded local tips keep paths, controls and pressure unchanged. No external preset file is needed to reopen the page.");
                }
                if changed {e.prepared=None;e.images=None;}
                refine_preview=ui.add_enabled(fresh && values.is_some(),egui::Button::new("Prepare refinement preview")).clicked();
                ui.small("Join two open strokes with the same tip and device-pressure mode. Different colors/widths keep the original editable segments, selected together with a linear-width connector in the first stroke's color. Equal styles combine into one outline. Distances/errors use document pixels. Apply the shared preview above; Undo restores all sources.");
            });
            ui.collapsing("Vector path display",|ui|{
                let settings=&mut self.workspaces.store.current.vector_overlay;
                ui.checkbox(&mut settings.enabled,"Show vector paths while drawing");
                use drawer_core::vector_tip::{Scope,Points};
                ui.horizontal(|ui|{ui.radio_value(&mut settings.scope,Scope::Selected,"Selected vector layer");ui.radio_value(&mut settings.scope,Scope::All,"All vector layers");});
                ui.horizontal(|ui|{ui.radio_value(&mut settings.points,Points::Ends,"Endpoints");ui.radio_value(&mut settings.points,Points::All,"All controls");ui.radio_value(&mut settings.points,Points::None,"No points");});
                ui.small("Hidden layers are omitted; locked paths may be shown. These workspace preferences do not change artwork, selection, Undo or exports. Editing-tool handles are separate guides.");
            });
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
        if let Some(all) = reset_preview
            && let Err(error) = self.preview_pressure_reset(ctx, all)
        {
            self.status = Some(error);
        }
        if refine_preview && let Err(error) = self.preview_vector_edit(ctx, true) {
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
pub(crate) fn node_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_nodes_verify::verify(root)?;
    for (tool, command, point, count) in [
        (Tool::AddNode, "vector-node-add-tool", [10., 10.], 4),
        (Tool::DeleteNode, "vector-node-delete-tool", [16., 4.], 2),
        (Tool::CornerNode, "vector-node-corner-tool", [16., 4.], 3),
        (Tool::SmoothNode, "vector-node-smooth-tool", [16., 4.], 3),
    ] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].commit_state(drawer_core::vector_nodes_verify::fixture()?.snapshot())?;
        app.documents[0].view.zoom = 3.;
        let layer = app.documents[0].scene.lines.layers[0].id;
        let path = app.documents[0].scene.lines.layers[0].paths[0].id;
        if tool == Tool::CornerNode {
            let hit = core::Hit {
                path,
                control: Some(1),
                parameter: None,
                point: [16., 4.],
                distance: 0.,
            };
            let state = drawer_core::vector_nodes::prepare(
                &app.documents[0],
                layer,
                hit,
                drawer_core::vector_nodes::Operation::Smooth,
            )?;
            app.documents[0].commit_state(state)?;
        }
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Vector(layer),
        );
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            crate::input_replay::frame(&ctx, app, *time, events);
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        app.execute_command(&ctx, command);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let down = crate::precision::pointer(&ctx, &app, point, Phase::Down);
        app.drawing.virtual_events(vec![down], false);
        step(&mut app, &mut time, vec![]);
        let p = app
            .drawing
            .vector_edit_preview(app.documents[0].id)
            .ok_or_else(|| format!("Node preview missing {tool:?}: {:?}", app.status))?;
        if p.scene.lines.layers[0].paths[0].control_count() != count
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
        {
            return Err(format!("Node preview/source isolation {tool:?}"));
        }
        let mut up = down;
        up.phase = Phase::Up;
        app.drawing.virtual_events(vec![up], false);
        step(&mut app, &mut time, vec![]);
        let p = &app.documents[0].scene.lines.layers[0].paths[0];
        if p.control_count() != count
            || app.documents[0].history_counts().0 != history.0 + 1
            || p.width != before.scene.lines.layers[0].paths[0].width
            || p.color != before.scene.lines.layers[0].paths[0].color
            || app.documents[0].scene.lines.layers[0].paths[1]
                != before.scene.lines.layers[0].paths[1]
        {
            return Err(format!(
                "Node mouse commit/style/unselected path {tool:?}: {:?}",
                app.status
            ));
        }
        if matches!(tool, Tool::CornerNode | Tool::SmoothNode)
            && p.definition
                .as_ref()
                .unwrap()
                .nodes
                .as_ref()
                .unwrap()
                .smooth[1]
                != (tool == Tool::SmoothNode)
        {
            return Err("Node mode marker mismatch".into());
        }
        let file = root.join(format!("vector-node-input-{tool:?}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err(format!("Node mouse native/Undo {tool:?}"));
        }
        if tool != Tool::AddNode {
            continue;
        }
        for cancel in 0..5 {
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let selection = app.drawing.vector_selection.clone();
            app.drawing.virtual_events(vec![down], false);
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
                2 => {
                    let mut e = down;
                    e.phase = Phase::Cancel;
                    app.drawing.virtual_events(vec![e], false);
                    vec![]
                }
                3 => {
                    let mut e = up;
                    e.position[0] += 30.;
                    app.drawing.virtual_events(vec![e], false);
                    vec![]
                }
                _ => {
                    app.documents[0].view.angle = 30.;
                    vec![]
                }
            };
            step(&mut app, &mut time, events);
            if app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts() != history
                || app.drawing.vector_selection != selection
            {
                return Err(format!("Node cancel {cancel} changed source/selection"));
            }
            app.documents[0].view.angle = 0.;
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
    }
    Ok(())
}

pub(crate) fn refine_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_refine_verify::verify(root)?;
    for command in ["vector-push-tool", "vector-redraw-tool"] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].commit_state(drawer_core::vector_nodes_verify::fixture()?.snapshot())?;
        app.documents[0].view.zoom = 4.;
        let layer = app.documents[0].scene.lines.layers[0].id;
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Vector(layer),
        );
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            crate::input_replay::frame(&ctx, app, *time, events);
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        app.execute_command(&ctx, command);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let start = if command == "vector-push-tool" {
            [16., 4.]
        } else {
            [4., 16.]
        };
        let end = if command == "vector-push-tool" {
            [18., 7.]
        } else {
            [16., 4.]
        };
        let down = crate::precision::pointer(&ctx, &app, start, Phase::Down);
        app.drawing.virtual_events(vec![down], false);
        step(&mut app, &mut time, vec![]);
        if !app.drawing.pending() {
            return Err(format!(
                "Refine capture missing: {command} {:?}",
                app.status
            ));
        }
        if command == "vector-redraw-tool" {
            let middle = crate::precision::pointer(&ctx, &app, [10., 20.], Phase::Move);
            app.drawing.virtual_events(vec![middle], false);
            step(&mut app, &mut time, vec![]);
        }
        let moved = crate::precision::pointer(&ctx, &app, end, Phase::Move);
        app.drawing.virtual_events(vec![moved], false);
        step(&mut app, &mut time, vec![]);
        if app
            .drawing
            .vector_edit_preview(app.documents[0].id)
            .is_none()
            || app.documents[0].snapshot() != before
        {
            return Err(format!(
                "Refine preview/source isolation: {command} {:?}",
                app.status
            ));
        }
        let mut up = moved;
        up.phase = Phase::Up;
        app.drawing.virtual_events(vec![up], false);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].history_counts().0 != history.0 + 1
            || app.documents[0].scene.lines.layers[0].paths[1]
                != before.scene.lines.layers[0].paths[1]
        {
            return Err("Refine one Undo/unselected path".into());
        }
        if app.documents[0].scene.lines.layers[0].paths[0].points
            == before.scene.lines.layers[0].paths[0].points
        {
            return Err("Refine mouse did not change geometry".into());
        }
        let file = root.join(format!("{command}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Refine native save/Undo".into());
        }
        for cancel in 0..3 {
            let selection = app.drawing.vector_selection.clone();
            let count = app.documents[0].history_counts();
            app.drawing.virtual_events(vec![down], false);
            step(&mut app, &mut time, vec![]);
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
                    let mut event = up;
                    event.phase = Phase::Cancel;
                    app.drawing.virtual_events(vec![event], false);
                    vec![]
                }
            };
            step(&mut app, &mut time, events);
            if app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts() != count
                || app.drawing.vector_selection != selection
            {
                return Err("Refine cancellation altered source/history/selection".into());
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
        let id = app.documents[0].scene.lines.layers[0].paths[0].id;
        app.drawing.vector_selection = Selection {
            paths: vec![id],
            ..Default::default()
        };
        app.execute_command(&ctx, "vector-refine-tools");
        let e = app
            .vector_edit
            .draft
            .as_mut()
            .ok_or("Refine dialog missing")?;
        e.selection = Selection {
            paths: vec![id],
            ..Default::default()
        };
        e.refine.kind = 3;
        e.refine.width = drawer_core::vector_refine::Width::ScaleUp;
        e.refine.value = 2.;
        app.preview_vector_edit(&ctx, true)?;
        if app.documents[0].snapshot() != before {
            return Err("Refine dialog preview mutated original".into());
        }
        app.apply_vector_numbers()?;
        if app.documents[0].scene.lines.layers[0].paths[0].width != 6.
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Refine dialog apply/Undo".into());
        }
    }
    Ok(())
}

pub(crate) fn tip_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_tip_verify::verify(root)?;
    super::vector_paths::verify()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::vector_tip_verify::fixture()?.snapshot())?;
    let d = &app.documents[0];
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    app.panels
        .targets
        .insert(d.id, drawer_core::thumbnail::Target::Vector(layer));
    let before = d.snapshot();
    let count = d.history_counts();
    for _ in 0..2 {
        app.execute_command(&ctx, "vector-path-overlay");
    }
    if app.workspaces.store.current.vector_overlay.enabled
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
    {
        return Err("Overlay toggle changed page or Undo".into());
    }
    for tip in drawer_core::vector_tip::Preset::ALL {
        app.execute_command(&ctx, "vector-tip-tools");
        let e = app
            .vector_edit
            .draft
            .as_mut()
            .ok_or("Vector tip dialog missing")?;
        if e.refine.kind != 6 {
            return Err("Vector tip command did not choose style edit".into());
        }
        e.selection = Selection {
            paths: vec![id],
            ..Default::default()
        };
        e.refine.tip = tip;
        app.preview_vector_edit(&ctx, true)?;
        if app.documents[0].snapshot() != before
            || app
                .vector_edit
                .draft
                .as_ref()
                .is_none_or(|e| e.images.is_none())
        {
            return Err("Tip before/after preview/source isolation".into());
        }
        app.apply_vector_numbers()?;
        if app.documents[0].scene.lines.layers[0].paths[0].tip != tip
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Tip dialog apply/Undo".into());
        }
    }
    app.vector_edit.draft = None;
    for time in [0.1, 0.2] {
        crate::input_replay::frame(&ctx, &mut app, time, vec![]);
    }
    let down = crate::precision::pointer(&ctx, &app, [8., 16.], Phase::Down);
    app.drawing.virtual_events(vec![down], false);
    crate::input_replay::frame(&ctx, &mut app, 0.3, vec![]);
    if !app.drawing.pending() {
        return Err("Overlay active gesture fixture did not capture".into());
    }
    let history = app.documents[0].history_counts();
    app.execute_command(&ctx, "vector-path-overlay");
    if !app.drawing.pending()
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != history
    {
        return Err("Overlay toggle finished or changed the active gesture".into());
    }
    app.finish_stroke();
    app.workspaces.store.current.vector_overlay = drawer_core::vector_tip::Overlay {
        enabled: true,
        scope: drawer_core::vector_tip::Scope::All,
        points: drawer_core::vector_tip::Points::All,
    };
    let file = root.join("vector-overlay-workspace.json");
    app.workspaces
        .store
        .current
        .export(&file)
        .map_err(|e| e.to_string())?;
    let read = drawer_core::workspaces::Snapshot::read(&file).map_err(|e| e.to_string())?;
    if read.vector_overlay != app.workspaces.store.current.vector_overlay {
        return Err("Vector overlay workspace roundtrip".into());
    }
    Ok(())
}

pub(crate) fn reset_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_reset_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::vector_reset_verify::fixture()?.snapshot())?;
    let layer = app.documents[0].scene.lines.layers[0].id;
    let path = app.documents[0].scene.lines.layers[0].paths[0].id;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Vector(layer),
    );
    let mut time = 0.;
    for all in [false, true] {
        app.begin_vector_edit(Some(Tool::Pressure))?;
        let selection = Selection {
            controls: vec![Control { path, point: 1 }],
            paths: vec![],
        };
        app.drawing.vector_selection = selection.clone();
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let command = if all {
            "vector-pressure-reset-layer"
        } else {
            "vector-pressure-reset-selected"
        };
        app.execute_command(&ctx, command);
        if app.vector_edit.draft.as_ref().is_none_or(|e| {
            e.prepared.is_none() || e.reset != Some(all) || e.selection != selection
        }) || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
        {
            return Err(format!(
                "Reset command preview/scope/source: {:?}",
                app.status
            ));
        }
        let expected = app
            .vector_edit
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .clone()
            .unwrap();
        time += 0.1;
        crate::input_replay::frame(&ctx, &mut app, time, vec![]);
        app.apply_vector_numbers()?;
        if app.documents[0].snapshot() != expected
            || app.documents[0].history_counts().0 != history.0 + 1
            || app.drawing.vector_selection != selection
        {
            return Err("Reset apply/selection/Undo count".into());
        }
        let file = root.join("app-pressure-reset.drawer");
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != expected
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != before
        {
            return Err("Reset save/reopen/Undo".into());
        }
        app.execute_command(&ctx, command);
        time += 0.1;
        crate::input_replay::frame(
            &ctx,
            &mut app,
            time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        if app.vector_edit.draft.is_some() || app.documents[0].snapshot() != before {
            return Err("Reset Escape changed the page".into());
        }
        time += 0.1;
        crate::input_replay::frame(
            &ctx,
            &mut app,
            time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        app.execute_command(&ctx, command);
        if app.preview_vector_numbers(&ctx).is_ok()
            || app
                .vector_edit
                .draft
                .as_ref()
                .is_some_and(|e| e.prepared.is_some())
        {
            return Err("Failed numeric preview retained an old reset plan".into());
        }
        app.execute_command(&ctx, command);
        app.documents[0].revision += 1;
        let stale = app.documents[0].snapshot();
        if app.apply_vector_numbers().is_ok() || app.documents[0].snapshot() != stale {
            return Err("Stale pressure reset accepted".into());
        }
        app.vector_edit.draft = None;
    }
    app.documents[0].scene.lines.layers[0].locked = true;
    let before = app.documents[0].snapshot();
    app.execute_command(&ctx, "vector-pressure-reset-layer");
    if app
        .vector_edit
        .draft
        .as_ref()
        .is_some_and(|e| e.prepared.is_some())
        || app.documents[0].snapshot() != before
    {
        return Err("Locked pressure reset prepared".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn vector_pressure_reset_preview_scopes_cancel_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::reset_smoke(&root.0).unwrap();
    }
    #[test]
    fn vector_tips_preview_apply_undo_and_overlay_workspace() {
        let root = crate::tests::TestDirectory::new();
        super::tip_smoke(&root.0).unwrap();
    }
    #[test]
    fn vector_refinement_mouse_preview_cancel_save_numeric_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::refine_smoke(&root.0).unwrap();
    }
    #[test]
    fn vector_mouse_pressure_points_color_numbers_cancellation_and_temporary_tool() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
    #[test]
    fn vector_nodes_mouse_add_delete_corner_smooth_preview_cancel_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::node_smoke(&root.0).unwrap();
    }
}
