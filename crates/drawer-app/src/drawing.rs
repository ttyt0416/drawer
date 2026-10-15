use super::*;
use drawer_core::{
    painting::{Stroke, Style},
    scene::Point,
    view::View,
};
#[derive(Clone, Copy, PartialEq)]
struct Projection {
    view: View,
    size: Point,
    center: Point,
    ppp: f64,
}
impl Projection {
    fn point(self, pos: egui::Pos2) -> Point {
        // Stabilize f32 screen-coordinate round trips below 1/1024 document pixel.
        self.view
            .screen_to_document(
                [pos.x as f64, pos.y as f64],
                self.size,
                self.center,
                self.ppp,
            )
            .map(|v| (v * 1024.0).round() / 1024.0)
    }
    fn tilt_angle(self, tilt: Option<Point>) -> f64 {
        let Some(tilt) = tilt else {
            return 0.0;
        };
        let vector = tilt.map(f64::sin);
        if vector[0].hypot(vector[1]) < 1e-9 {
            return 0.0;
        }
        let p = self
            .view
            .screen_to_document([0.0; 2], self.size, self.center, self.ppp);
        let q = self
            .view
            .screen_to_document(vector, self.size, self.center, self.ppp);
        (q[1] - p[1]).atan2(q[0] - p[0]).to_degrees()
    }
}
pub(super) struct PaintInput<'a> {
    pub drawing: &'a mut Drawing,
    pub view: u64,
    pub layer: Option<usize>,
    pub color: [u8; 4],
}
struct Session {
    stroke: Stroke,
    view: u64,
    projection: Projection,
    last_screen: egui::Pos2,
    last_pressure: f64,
    last_attributes: drawer_core::pointer_input::Attributes,
    tilt_angle: bool,
}
#[derive(Default)]
pub(super) struct Drawing {
    active: Option<Session>,
    ownership: drawer_core::pointer_input::Ownership,
    events: Vec<(drawer_core::touch::Effect, bool)>,
    pub incoming: Vec<drawer_core::pointer_input::Event>,
    pub gui: Option<Vec<drawer_core::pointer_input::Event>>,
    pub pen_available: bool,
    native_mouse: bool,
    pub settings: drawer_core::pointer_input::Settings,
    pub mode: super::precision::Mode,
    pub connect_line: bool,
    pub opacity: Option<[f64; 4]>,
    pub picked: Option<[u8; 4]>,
    endpoints: std::collections::HashMap<(u64, usize, BrushTip), Point>,
    pub pressure: drawer_core::pressure::Settings,
    pub last_source: Option<drawer_core::pointer_input::Source>,
    pub last_attributes: drawer_core::pointer_input::Attributes,
    pub error: Option<String>,
    controls: drawer_core::pen_controls::Adapter,
    anchor: Option<[f64; 2]>,
    last_input: Option<drawer_core::pointer_input::Kind>,
    pub mapping_context: drawer_core::tablet_mapping::Context,
    pub mapping_report: Option<String>,
    mapping_settings: Option<drawer_core::tablet_mapping::Settings>,
    touch: drawer_core::touch::Controller,
    touch_view: u64,
    touch_layer: Option<egui::LayerId>,
    touch_allowed: bool,
}
impl Drawing {
    pub(super) fn mouse_view_input(&self) -> bool {
        self.last_input != Some(drawer_core::pointer_input::Kind::Touch)
            && !self.touch.gui_drag_blocked()
    }
    pub(super) fn pen_cursor(&self, view: u64) -> Option<egui::Pos2> {
        if self.settings.controls.mode != drawer_core::pen_controls::Mode::Relative
            || self.last_input != Some(drawer_core::pointer_input::Kind::Pen)
            || self.active_view().is_some_and(|id| id != view)
        {
            return None;
        }
        self.controls
            .cursor()
            .map(|p| egui::pos2(p[0] as f32, p[1] as f32))
    }
    pub(super) fn virtual_events(
        &mut self,
        events: Vec<drawer_core::pointer_input::Event>,
        pen_available: bool,
    ) {
        self.incoming = events;
        self.gui = Some(vec![]);
        self.pen_available = pen_available;
        self.native_mouse = true;
    }
    pub fn active_view(&self) -> Option<u64> {
        self.active.as_ref().map(|s| s.view)
    }
    pub fn pending(&self) -> bool {
        self.active.is_some()
    }
    pub fn changed(&self, document: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|s| s.stroke.document == document && s.stroke.changed_pixels() > 0)
    }
    pub fn preview(&self, document: u64) -> Option<&Stroke> {
        self.active
            .as_ref()
            .filter(|s| s.stroke.document == document)
            .map(|s| &s.stroke)
    }
    fn cancel(&mut self) {
        self.active = None;
        self.ownership.clear();
    }
    pub fn prepare(&mut self, ctx: &egui::Context) {
        let rect = ctx.content_rect();
        let bounds = [
            [rect.min.x as f64, rect.min.y as f64],
            [rect.max.x as f64, rect.max.y as f64],
        ];
        let anchor = self
            .anchor
            .unwrap_or([rect.center().x as f64, rect.center().y as f64]);
        use drawer_core::touch::Effect;
        let mut effects = self.touch.configure(self.settings.touch);
        if !self.touch_allowed {
            effects.extend(self.touch.clear_touches());
        }
        let mut events: Vec<_> = self
            .controls
            .configure(self.settings.controls)
            .into_iter()
            .map(Effect::Pointer)
            .collect();
        let incoming: Vec<_> = self
            .incoming
            .drain(..)
            .chain(
                self.gui
                    .take()
                    .unwrap_or_else(|| crate::pointer_input::gui_events(ctx)),
            )
            .filter(|e| drawer_core::pointer_input::drawing_event_allowed(*e, self.pen_available))
            .collect();
        let mut mapped_events = Vec::with_capacity(incoming.len());
        for event in incoming {
            self.last_input = Some(event.source.kind);
            let mapped = match self.mapping_context.map(self.settings.mapping, event) {
                Ok(e) => {
                    if e.source.kind == drawer_core::pointer_input::Kind::Pen {
                        self.mapping_report = None;
                    }
                    e
                }
                Err(error) => {
                    self.mapping_report = Some(error.message().into());
                    let mut e = event;
                    e.phase = drawer_core::pointer_input::Phase::Cancel;
                    e
                }
            };
            mapped_events.push(mapped);
        }
        let target = self.mapping_context.canvas;
        let layer = self.touch_layer;
        let allowed = self.touch_allowed;
        effects.extend(self.touch.process(&mapped_events, self.touch_view, |e| {
            let p = egui::pos2(e.position[0] as f32, e.position[1] as f32);
            allowed && target.is_some_and(|r| r.contains(e.position)) && ctx.layer_id_at(p) == layer
        }));
        for effect in effects {
            match effect {
                Effect::Pointer(e) => events.extend(
                    self.controls
                        .event(e, bounds, anchor)
                        .into_iter()
                        .map(Effect::Pointer),
                ),
                Effect::Motion(m) => events.push(Effect::Motion(m)),
            }
        }
        self.events = events.into_iter().map(|e| (e, false)).collect();
    }
    pub fn finish(&mut self, documents: &mut [Document]) {
        self.controls.reset();
        self.touch.clear_touches();
        self.ownership.clear();
        if let Some(session) = self.active.take() {
            if let Some(doc) = documents
                .iter_mut()
                .find(|d| d.id == session.stroke.document)
            {
                self.commit_session(session, doc);
            } else {
                self.error = Some("The drawing was closed; pending stroke discarded".into());
            }
        }
    }
    fn finish_document(&mut self, doc: &mut Document) {
        self.ownership.clear();
        if let Some(s) = self.active.take() {
            self.commit_session(s, doc);
        }
    }
    fn commit_session(&mut self, session: Session, doc: &mut Document) {
        let anchor = session.stroke.anchor();
        match session.stroke.commit(doc) {
            Ok(true) => {
                if let Some((layer, tip, point)) = anchor {
                    self.remember(doc.id, layer, tip, point);
                }
            }
            Ok(false) => {}
            Err(e) => self.error = Some(e),
        }
    }
    fn remember(&mut self, id: u64, layer: usize, tip: BrushTip, point: Point) {
        if self.endpoints.len() >= 4096 && !self.endpoints.contains_key(&(id, layer, tip)) {
            self.endpoints.clear();
        }
        self.endpoints.insert((id, layer, tip), point);
    }
    fn sample(
        &mut self,
        doc: &Document,
        pos: egui::Pos2,
        pressure: f64,
        attributes: drawer_core::pointer_input::Attributes,
    ) {
        self.last_attributes = attributes.normalized();
        if let Some(s) = &mut self.active {
            s.last_screen = pos;
            s.last_pressure = pressure;
            s.last_attributes = attributes.normalized();
            let angle = if s.tilt_angle {
                s.projection.tilt_angle(s.last_attributes.tilt)
            } else {
                0.0
            };
            if let Err(e) = s.stroke.set_angle_offset(angle).and_then(|_| {
                s.stroke
                    .sample_pressure(doc, s.projection.point(pos), pressure)
            }) {
                self.error = Some(e);
                self.cancel();
            }
        }
    }
    fn receive(&mut self, index: usize, doc: &mut Document) {
        use drawer_core::pointer_input::Decision;
        let drawer_core::touch::Effect::Pointer(event) = self.events[index].0 else {
            return;
        };
        let pressure = event.pressure_value();
        self.events[index].1 = true;
        match self.ownership.receive(event, self.settings) {
            Decision::Sample => self.sample(
                doc,
                egui::pos2(event.position[0] as f32, event.position[1] as f32),
                pressure,
                event.pen_attributes(),
            ),
            Decision::Finish => {
                self.sample(
                    doc,
                    egui::pos2(event.position[0] as f32, event.position[1] as f32),
                    pressure,
                    event.pen_attributes(),
                );
                self.finish_document(doc);
            }
            Decision::Cancel => self.cancel(),
            Decision::Busy => {
                self.error =
                    Some("Another device owns the stroke; release and press again to draw".into())
            }
            _ => {}
        }
    }
    pub fn advance(&mut self, ctx: &egui::Context, documents: &mut [Document]) {
        if ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        }) {
            self.controls.reset();
            self.touch.reset();
            self.cancel();
            self.events.clear();
            return;
        }
        if self
            .active
            .as_ref()
            .is_some_and(|s| (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8)
        {
            self.finish(documents);
            self.events.clear();
            return;
        }
        if self.pending() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.controls.reset();
            self.touch.clear_touches();
            self.cancel();
            self.events.clear();
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
            return;
        }
        for n in 0..self.events.len() {
            if matches!(self.events[n].0, drawer_core::touch::Effect::Motion(_)) {
                break;
            }
            let Some(id) = self.active.as_ref().map(|s| s.stroke.document) else {
                break;
            };
            let Some(doc) = documents.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive(n, doc);
        }
        if !self.native_mouse
            && self
                .ownership
                .owner()
                .is_some_and(|s| s.kind == drawer_core::pointer_input::Kind::Mouse)
            && ctx.input(|i| !i.pointer.primary_down())
        {
            self.cancel();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Document,
        view: u64,
        layer: Option<usize>,
        style: Style,
        enabled: bool,
    ) {
        use drawer_core::pointer_input::{Decision, Phase};
        let rect = response.rect;
        let mut projection = Projection {
            view: doc.view,
            size: [doc.width as f64, doc.height as f64],
            center: [rect.center().x as f64, rect.center().y as f64],
            ppp: ui.ctx().pixels_per_point() as f64,
        };
        if let Some(s) = &mut self.active
            && s.stroke.document == doc.id
            && s.view == view
            && s.projection != projection
        {
            s.projection = projection;
            let pos = s.last_screen;
            let pressure = s.last_pressure;
            let attributes = s.last_attributes;
            self.sample(doc, pos, pressure, attributes);
        }
        for n in 0..self.events.len() {
            if self.events[n].1 {
                continue;
            }
            let event = match self.events[n].0 {
                drawer_core::touch::Effect::Pointer(e) => e,
                drawer_core::touch::Effect::Motion(m) => {
                    if m.view != view {
                        continue;
                    }
                    self.events[n].1 = true;
                    if ui.is_enabled()
                        && self.touch_allowed
                        && self.active_view().is_none_or(|id| id == view)
                    {
                        self.finish_document(doc);
                        if let Err(e) = m.apply(
                            &mut doc.view,
                            projection.size,
                            projection.center,
                            projection.ppp,
                        ) {
                            self.error = Some(e);
                        }
                        projection.view = doc.view;
                        ui.ctx().request_repaint();
                    }
                    continue;
                }
            };
            if self.pending() {
                if self.active_view() == Some(view) {
                    self.receive(n, doc);
                }
                continue;
            }
            let pos = egui::pos2(event.position[0] as f32, event.position[1] as f32);
            if !enabled
                || event.phase != Phase::Down
                || !rect.contains(pos)
                || !ui.clip_rect().contains(pos)
                || ui.ctx().layer_id_at(pos) != Some(response.layer_id)
            {
                continue;
            }
            self.events[n].1 = true;
            if self.ownership.receive(event, self.settings) != Decision::Begin {
                continue;
            }
            let mut style = style;
            if self.mode != super::precision::Mode::Paint {
                if self.mode == super::precision::Mode::Eyedropper {
                    let point = projection.point(pos);
                    if point[0] >= 0.0
                        && point[1] >= 0.0
                        && point[0] < doc.width as f64
                        && point[1] < doc.height as f64
                    {
                        self.picked = Some(
                            doc.composite_pixel(point[0].floor() as u32, point[1].floor() as u32),
                        );
                    }
                }
                self.ownership.clear();
                continue;
            }
            let current_tip = style.tip;
            style.tip =
                self.settings
                    .features
                    .tip(event.source.kind, event.pen_attributes(), style.tip);
            if event.source.kind == drawer_core::pointer_input::Kind::Pen {
                style.tip =
                    self.settings
                        .controls
                        .tip(event.pen_attributes(), current_tip, style.tip);
            }
            if let Some(value) = self
                .settings
                .for_kind(event.source.kind)
                .and_then(|s| s.stabilization)
            {
                style.smoothing = value;
            }
            let index = BrushTip::ALL
                .iter()
                .position(|tip| *tip == style.tip)
                .unwrap();
            style.color[3] =
                (style.color[3] as f64 * self.opacity.unwrap_or([1.0; 4])[index]).round() as u8;
            if self.connect_line {
                self.ownership.clear();
                self.connect_line = false;
                let result = (|| -> Result<(), String> {
                    let layer = layer.ok_or("Select a raster layer to connect a line")?;
                    let from = *self
                        .endpoints
                        .get(&(doc.id, layer, style.tip))
                        .ok_or("Draw a starting stroke with this tool on this layer first")?;
                    let to = projection.point(pos);
                    style.smoothing = 0.0;
                    let mut stroke = Stroke::begin(doc, layer, style)?;
                    stroke.sample(doc, from)?;
                    stroke.sample(doc, to)?;
                    if stroke.commit(doc)? {
                        self.remember(doc.id, layer, style.tip, to);
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    self.error = Some(e);
                }
                continue;
            }
            match layer
                .ok_or_else(|| "Select a raster layer to paint".to_string())
                .and_then(|layer| Stroke::begin(doc, layer, style))
            {
                Ok(stroke) => {
                    self.last_source = Some(event.source);
                    self.active = Some(Session {
                        stroke,
                        view,
                        projection,
                        last_screen: pos,
                        last_pressure: 1.0,
                        last_attributes: Default::default(),
                        tilt_angle: event.source.kind == drawer_core::pointer_input::Kind::Pen
                            && self.settings.features.tilt_angle,
                    });
                    let pressure = event.pressure_value();
                    self.sample(doc, pos, pressure, event.pen_attributes());
                }
                Err(e) => {
                    self.error = Some(e);
                    self.cancel();
                }
            }
        }
    }
}
impl Drawer {
    pub(super) fn finish_stroke(&mut self) {
        self.precision.finish();
        self.drawing.finish(&mut self.documents);
        if let Some(e) = self.drawing.error.take() {
            self.status = Some(e);
        }
    }
    pub(super) fn process_drawing(&mut self, ctx: &egui::Context) {
        self.drawing.opacity = Some(self.panels.settings.brush_opacity);
        self.drawing
            .endpoints
            .retain(|(id, _, _), _| self.documents.iter().any(|d| d.id == *id));
        if let Some(color) = self.drawing.picked.take() {
            self.references.session.workspace.color = color;
        }
        self.drawing.settings = self.workspaces.store.current.pointer;
        self.drawing.pressure = self.workspaces.store.current.pressure;
        let target_view = self
            .drawing
            .active_view()
            .or_else(|| self.active_tab.map(|n| self.tabs[n].id));
        let canvas_rect = if !self.simple_view
            && self.workspaces.store.current.layout.arrangement
                != drawer_core::layout::Arrangement::Tabs
        {
            target_view.and_then(|id| self.layout.viewport(id))
        } else {
            self.canvas_view.viewport_rect()
        };
        let mut mapping_context = drawer_core::tablet_mapping::Context {
            origin: [0.0; 2],
            scale: ctx.pixels_per_point() as f64,
            canvas: canvas_rect.map(|r| drawer_core::scene::Rect {
                min: [r.min.x as f64, r.min.y as f64],
                max: [r.max.x as f64, r.max.y as f64],
            }),
            monitors: self.display.monitors.clone(),
        };
        #[cfg(windows)]
        if let Some(native) = &self.native_mouse {
            mapping_context.origin = native.client_origin().unwrap_or([f64::NAN; 2]);
        }
        let mapping = self.drawing.settings.mapping;
        let touch_view = target_view.unwrap_or(0);
        let touch_changed = self.drawing.touch_view != touch_view
            || self.drawing.mapping_context.origin != mapping_context.origin
            || self.drawing.mapping_context.scale != mapping_context.scale
            || self.drawing.mapping_context.canvas != mapping_context.canvas;
        if touch_changed {
            self.drawing.touch.clear_touches();
            if self
                .drawing
                .ownership
                .owner()
                .is_some_and(|s| s.kind == drawer_core::pointer_input::Kind::Touch)
            {
                self.drawing.cancel();
            }
        }
        self.drawing.touch_view = touch_view;
        self.drawing.touch_layer = if !self.simple_view
            && self.workspaces.store.current.layout.arrangement
                != drawer_core::layout::Arrangement::Tabs
        {
            target_view.and_then(|id| self.layout.viewport_layer(id))
        } else {
            self.canvas_view.viewport_layer()
        };
        self.drawing.touch_allowed = !self.tool_shortcuts_blocked(ctx);
        let mapping_changed = self.drawing.mapping_settings != Some(mapping);
        let environment_changed =
            mapping.enabled && self.drawing.mapping_context != mapping_context;
        if mapping_changed || environment_changed {
            self.finish_stroke();
            self.drawing.incoming.clear();
            self.drawing.mapping_settings = Some(mapping);
        }
        self.drawing.mapping_context = mapping_context;
        self.drawing.anchor = canvas_rect.map(|r| [r.center().x as f64, r.center().y as f64]);
        let preferred = self.drawing.settings.backend;
        let changed = mapping_changed
            || self.native_input.applied != Some(preferred)
            || std::mem::take(&mut self.native_input.retry);
        if changed {
            self.finish_stroke();
            self.drawing.incoming.clear();
            self.native_input.applied = Some(preferred);
        }
        #[cfg(windows)]
        if let Some(native) = &self.native_mouse {
            if changed {
                native.configure(preferred);
            }
            native.set_focus(ctx.input(|i| i.focused));
            self.native_input.report = format!("{} · {}", native.report(), native.touch_report());
            self.drawing.native_mouse = true;
            self.drawing.pen_available =
                native.effective() != drawer_core::pointer_input::Backend::Mouse;
            match native.drain(ctx.pixels_per_point() as f64) {
                Ok(events) => {
                    self.drawing.gui = Some(if environment_changed { vec![] } else { events })
                }
                Err(error) => {
                    self.drawing.controls.reset();
                    self.drawing.touch.reset();
                    self.drawing.cancel();
                    self.drawing.incoming.clear();
                    self.drawing.gui = Some(vec![]);
                    self.status = Some(error);
                }
            }
        }
        self.drawing.prepare(ctx);
        if ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        }) {
            self.drawing.controls.reset();
            self.drawing.touch.reset();
            self.drawing.cancel();
            self.drawing.events.clear();
            return;
        }
        if self.tool_shortcuts_blocked(ctx) {
            self.finish_stroke();
        } else {
            self.drawing.advance(ctx, &mut self.documents);
        }
        if let Some(e) = self.drawing.error.take() {
            self.status = Some(e);
            ctx.request_repaint();
        }
    }
}

pub(super) fn smoke_touch() -> Result<(), String> {
    use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.simple_view = true;
    app.panels.settings.brush_size = 4.0;
    app.documents[0].mark_saved("touch-virtual.drawer".into());
    crate::input_replay::frame(&ctx, &mut app, 0.0, vec![]);
    crate::input_replay::frame(&ctx, &mut app, 0.01, vec![]);
    let rect = app
        .canvas_view
        .viewport_rect()
        .ok_or("Touch canvas unavailable")?;
    let p = [rect.center().x as f64, rect.center().y as f64];
    let event = |id, phase, delta: [f64; 2], frame| Event {
        source: Source {
            kind: Kind::Touch,
            device: 99,
            contact: id,
        },
        phase,
        position: [p[0] + delta[0], p[1] + delta[1]],
        pressure: None,
        attributes: Attributes {
            touch_frame: Some(frame),
            ..Default::default()
        },
        origin: Default::default(),
    };
    let mut time = 0.02;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        app.drawing.virtual_events(events, true);
        crate::input_replay::frame(&ctx, app, *time, vec![]);
        *time += 0.01;
    };
    let original = app.documents[0].snapshot();
    step(
        &mut app,
        &mut time,
        vec![
            event(1, Phase::Down, [-10.0, 0.0], 1),
            event(2, Phase::Down, [10.0, 0.0], 1),
        ],
    );
    step(
        &mut app,
        &mut time,
        vec![
            event(1, Phase::Move, [0.0, -20.0], 2),
            event(2, Phase::Move, [0.0, 20.0], 2),
        ],
    );
    let view = app.documents[0].view;
    if (view.zoom - 2.0).abs() > 1e-6
        || (view.angle - 90.0).abs() > 1e-6
        || app.documents[0].snapshot() != original
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err(format!(
            "Touch view-only manipulation failed: zoom {}, angle {}",
            view.zoom, view.angle
        ));
    }
    step(
        &mut app,
        &mut time,
        vec![
            event(1, Phase::Up, [0.0, -20.0], 3),
            event(2, Phase::Up, [0.0, 20.0], 3),
        ],
    );
    app.workspaces.store.current.pointer.touch.finger_drawing = true;
    step(&mut app, &mut time, vec![]);
    step(
        &mut app,
        &mut time,
        vec![event(1, Phase::Down, [0.0; 2], 4)],
    );
    if !app.drawing.pending() {
        return Err("Enabled finger drawing did not start".into());
    }
    let mut pen = event(3, Phase::Down, [8.0, 0.0], 5);
    pen.source.kind = Kind::Pen;
    pen.attributes = Default::default();
    step(&mut app, &mut time, vec![pen]);
    if app.drawing.ownership.owner() != Some(pen.source)
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Pen did not discard pending finger ink".into());
    }
    let mut palm = event(2, Phase::Down, [0.0; 2], 6);
    palm.attributes.palm = Some(true);
    step(&mut app, &mut time, vec![palm]);
    pen.phase = Phase::Up;
    step(&mut app, &mut time, vec![pen]);
    if app.documents[0].history_counts() != (1, 0) {
        return Err("Pen/finger transition did not make one pen edit".into());
    }
    let painted = app.documents[0].snapshot();
    step(
        &mut app,
        &mut time,
        vec![
            event(1, Phase::Move, [0.0; 2], 7),
            event(1, Phase::Up, [0.0; 2], 8),
        ],
    );
    step(
        &mut app,
        &mut time,
        vec![event(4, Phase::Down, [0.0; 2], 9)],
    );
    app.drawing.virtual_events(vec![], true);
    crate::input_replay::frame(
        &ctx,
        &mut app,
        time,
        vec![egui::Event::WindowFocused(false)],
    );
    time += 0.01;
    if app.drawing.pending() || app.documents[0].snapshot() != painted {
        return Err("Focus loss committed finger preview".into());
    }
    crate::input_replay::frame(&ctx, &mut app, time, vec![egui::Event::WindowFocused(true)]);
    time += 0.01;
    step(
        &mut app,
        &mut time,
        vec![
            event(4, Phase::Move, [-8.0, 0.0], 10),
            event(4, Phase::Up, [-8.0, 0.0], 11),
        ],
    );
    if app.documents[0].snapshot() != painted {
        return Err("Held finger restarted after focus loss".into());
    }
    step(
        &mut app,
        &mut time,
        vec![
            event(4, Phase::Down, [-8.0, 0.0], 12),
            event(4, Phase::Up, [-8.0, 0.0], 13),
        ],
    );
    if app.documents[0].history_counts() != (2, 0) {
        return Err(format!(
            "Fresh finger stroke was not one edit: {:?}, focused {}",
            app.documents[0].history_counts(),
            ctx.input(|i| i.focused)
        ));
    }
    let after = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != painted
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Finger stroke undo/redo failed".into());
    }
    Ok(())
}
pub(super) fn smoke_defaults() -> Result<(), String> {
    use drawer_core::{
        brush_cursor::Shape,
        pointer_input::{Event, Kind, Phase, Source},
        pressure::Mapping,
    };
    for kind in [Kind::Mouse, Kind::Pen] {
        for tip in BrushTip::ALL {
            let e = Event {
                source: Source {
                    kind,
                    device: 0,
                    contact: 0,
                },
                phase: Phase::Down,
                position: [8.5, 8.5],
                pressure: None,
                attributes: Default::default(),
                origin: Default::default(),
            };
            if e.pressure_value() != 1.0 || e.pen_attributes().tilt.is_some() {
                return Err("Missing input defaults mismatch".into());
            }
            let mut d = Document::create(
                CanvasSpec {
                    width: 16,
                    height: 16,
                    dpi: 300.0,
                    background: if tip == BrushTip::Eraser {
                        Background::Color([80, 90, 100])
                    } else {
                        Background::Transparent
                    },
                },
                4096,
            )
            .map_err(|e| e.to_string())?;
            let style = Style {
                shape: Shape {
                    size: 4.0,
                    aspect: 1.0,
                    angle: 0.0,
                },
                tip,
                color: [20, 40, 80, 255],
                smoothing: 0.0,
                pressure: Mapping {
                    size: true,
                    opacity: true,
                    ..Default::default()
                },
            };
            let mut s = Stroke::begin(&d, 0, style)?;
            s.sample_pressure(&d, e.position, e.pressure_value())?;
            s.commit(&mut d)?;
            let alpha = d.composite_pixel(8, 8)[3];
            if (tip == BrushTip::Eraser && alpha == 255) || (tip != BrushTip::Eraser && alpha == 0)
            {
                return Err("Missing input properties produced an empty stroke".into());
            }
        }
    }
    Ok(())
}
pub(super) fn smoke_dedup() -> Result<(), String> {
    use drawer_core::pointer_input::{Event, Origin, Phase, Source, drawing_event_allowed};
    let make = |phase, origin| Event {
        source: Source::MOUSE,
        phase,
        position: [5.5, 5.5],
        pressure: None,
        attributes: Default::default(),
        origin,
    };
    for phase in [Phase::Down, Phase::Move, Phase::Up, Phase::Cancel] {
        if drawing_event_allowed(make(phase, Origin::PenCompatibility), true)
            || !drawing_event_allowed(make(phase, Origin::Unmarked), true)
            || !drawing_event_allowed(make(phase, Origin::PenCompatibility), false)
        {
            return Err("Synthetic promotion filter failed".into());
        }
    }
    Ok(())
}
pub(super) fn smoke_attributes() -> Result<(), String> {
    use drawer_core::{
        brush_cursor::Shape,
        pointer_input::{Attributes, Kind, PenFeatures},
    };
    let a = Attributes {
        tilt: Some([0.0, 0.5]),
        eraser: Some(true),
        button: Some(false),
        buttons: None,
        tablet: None,
        palm: None,
        touch_frame: None,
    }
    .normalized();
    let features = PenFeatures {
        tilt_angle: true,
        eraser: true,
        button_eraser: true,
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Color([80, 90, 100]),
        },
        4096,
    )
    .map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let style = Style {
        shape: Shape {
            size: 4.0,
            aspect: 0.25,
            angle: 0.0,
        },
        tip: features.tip(Kind::Pen, a, BrushTip::Pen),
        color: [20, 40, 80, 255],
        smoothing: 0.0,
        pressure: Default::default(),
    };
    let mut stroke = Stroke::begin(&d, 0, style)?;
    stroke.set_angle_offset(90.0)?;
    stroke.sample_pressure(&d, [8.5, 8.5], 1.0)?;
    if stroke.shape().angle != 90.0
        || !stroke.commit(&mut d)?
        || d.composite_pixel(8, 8)[3] != 0
        || !d.undo()
        || d.snapshot() != before
    {
        return Err("Synthetic pen attribute rendering failed".into());
    }
    Ok(())
}
pub(super) fn smoke_painting() -> Result<(), String> {
    use drawer_core::{
        brush_cursor::Shape,
        document::{Background, CanvasSpec},
        render::Renderer,
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        4096,
    )
    .map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let mut style = Style {
        shape: Shape {
            size: 1.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [80, 20, 40, 255],
        smoothing: 0.0,
        pressure: drawer_core::pressure::Mapping::default(),
    };
    for tip in [BrushTip::Pen, BrushTip::Eraser] {
        style.tip = tip;
        let mut s = Stroke::begin(&d, 0, style)?;
        s.sample(&d, [2.5, 5.5])?;
        s.sample(&d, [29.5, 5.5])?;
        let preview = Renderer::preview(&d, Some(&s)).pixel([12.5, 5.5]);
        if !s.commit(&mut d)? || d.composite_pixel(12, 5) != preview {
            return Err("Native stroke preview/commit mismatch".into());
        }
    }
    if d.history_counts() != (2, 0) || !d.undo() || !d.undo() || d.snapshot() != before {
        return Err("Native stroke undo mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_gestures_finger_ink_pen_priority_and_focus_use_the_actual_pipeline() {
        smoke_touch().unwrap();
    }
    use drawer_core::{
        brush_cursor::Mode,
        document::{Background, CanvasSpec},
        render::Renderer,
    };
    fn frame(ctx: &egui::Context, app: &mut Drawer, ppp: f32, events: Vec<egui::Event>) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 760.0),
            )),
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(ppp);
        let mut out = ctx.run_ui(input, |ui| app.show(ui));
        out.textures_delta.clear();
    }
    fn setup(ppp: f32) -> (egui::Context, Drawer) {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .unwrap();
        app.simple_view = true;
        app.panels.settings.brush_size = 1.0;
        app.references.session.workspace.color = [180, 20, 40, 255];
        app.documents[0].mark_saved("mouse.drawer".into());
        frame(&ctx, &mut app, ppp, vec![]);
        frame(&ctx, &mut app, ppp, vec![]);
        (ctx, app)
    }
    fn point(ctx: &egui::Context, app: &Drawer, p: Point) -> egui::Pos2 {
        let rect = app.canvas_view.viewport.unwrap();
        let p = app.documents[app.active.unwrap()].view.document_to_screen(
            p,
            [64.0, 48.0],
            [rect.center().x as f64, rect.center().y as f64],
            ctx.pixels_per_point() as f64,
        );
        egui::pos2(p[0] as f32, p[1] as f32)
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn completed_finger_stroke_precedes_gesture_in_the_same_input_batch() {
        use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
        let (ctx, mut app) = setup(1.0);
        app.workspaces.store.current.pointer.touch.finger_drawing = true;
        frame(&ctx, &mut app, 1.0, vec![]);
        let initial = app.documents[0].snapshot();
        let event = |id, phase, p: Point, batch| {
            let pos = point(&ctx, &app, p);
            Event {
                source: Source {
                    kind: Kind::Touch,
                    device: 9,
                    contact: id,
                },
                phase,
                position: [pos.x as f64, pos.y as f64],
                pressure: None,
                attributes: Attributes {
                    touch_frame: Some(batch),
                    ..Default::default()
                },
                origin: Default::default(),
            }
        };
        let events = vec![
            event(1, Phase::Down, [20.5, 20.5], 1),
            event(1, Phase::Up, [20.5, 20.5], 1),
            event(2, Phase::Down, [10.5, 30.5], 2),
            event(3, Phase::Down, [30.5, 30.5], 2),
            event(2, Phase::Move, [20.5, 20.5], 3),
            event(3, Phase::Move, [20.5, 40.5], 3),
        ];
        app.drawing.virtual_events(events, true);
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(
            app.documents[0].layers[0].pixels[20 * 64 + 20],
            [180, 20, 40, 255]
        );
        assert_eq!(app.documents[0].layers[0].pixels[30 * 64 + 10], [0; 4]);
        let view = app.documents[0].view;
        assert!((view.angle - 90.0).abs() < 1e-6);
        assert!(app.documents[0].undo());
        assert!(app.documents[0].snapshot() == initial);
        assert_eq!(app.documents[0].view, view);
    }
    #[test]
    fn mapped_input_scale_change_finishes_once_and_held_pen_does_not_restart() {
        use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
        let (ctx, mut app) = setup(1.0);
        app.workspaces.store.current.pointer.mapping.enabled = true;
        frame(&ctx, &mut app, 1.0, vec![]);
        frame(&ctx, &mut app, 1.0, vec![]);
        let e = |phase| Event {
            source: Source {
                kind: Kind::Pen,
                device: 1,
                contact: 1,
            },
            phase,
            position: [0.0; 2],
            pressure: Some(1.0),
            attributes: Attributes {
                tablet: Some([0.5; 2]),
                ..Default::default()
            },
            origin: Default::default(),
        };
        app.drawing.virtual_events(vec![e(Phase::Down)], true);
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(app.drawing.pending());
        app.drawing.virtual_events(vec![e(Phase::Move)], true);
        frame(&ctx, &mut app, 2.0, vec![]);
        assert!(!app.drawing.pending());
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        let saved = app.documents[0].snapshot();
        app.drawing
            .virtual_events(vec![e(Phase::Move), e(Phase::Up)], true);
        frame(&ctx, &mut app, 2.0, vec![]);
        assert!(app.documents[0].snapshot() == saved);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
    }
    #[test]
    fn backend_change_and_retry_finish_pending_stroke_once_and_discard_old_input() {
        use drawer_core::pointer_input::{Backend, Event, Phase, Source};
        let (ctx, mut app) = setup(1.0);
        let p = point(&ctx, &app, [20.5, 20.5]);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![egui::Event::PointerMoved(p), button(p, true)],
        );
        assert!(app.drawing.pending());
        app.workspaces.store.current.pointer.backend = Backend::Wintab;
        app.drawing.incoming = vec![Event {
            source: Source::MOUSE,
            phase: Phase::Down,
            position: [p.x as f64, p.y as f64],
            pressure: None,
            attributes: Default::default(),
            origin: Default::default(),
        }];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(!app.drawing.pending());
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        let before = app.documents[0].snapshot();
        frame(&ctx, &mut app, 1.0, vec![button(p, false)]);
        assert!(app.documents[0].snapshot() == before);
        let q = point(&ctx, &app, [25.5, 25.5]);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![egui::Event::PointerMoved(q), button(q, true)],
        );
        assert!(app.drawing.pending());
        app.native_input.retry = true;
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(!app.drawing.pending());
        assert_eq!(app.documents[0].history_counts(), (2, 0));
        frame(&ctx, &mut app, 1.0, vec![button(q, false)]);
        assert_eq!(app.documents[0].history_counts(), (2, 0));
    }
    #[test]
    fn pen_mouse_interleaving_keeps_device_owner_and_one_event_one_stroke() {
        use drawer_core::pointer_input::{Event, Kind, Phase, Source};
        let (ctx, mut app) = setup(1.0);
        let pen = Source {
            kind: Kind::Pen,
            device: 73,
            contact: 81,
        };
        let p = point(&ctx, &app, [10.5, 20.5]);
        let q = point(&ctx, &app, [40.5, 20.5]);
        let event = |source, phase, pos: egui::Pos2| Event {
            source,
            phase,
            position: [pos.x as f64, pos.y as f64],
            pressure: Some(0.5),
            attributes: Default::default(),
            origin: Default::default(),
        };
        app.workspaces.store.current.pointer.pen.stabilization = Some(0.4);
        app.drawing.incoming = vec![event(pen, Phase::Down, p)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(app.drawing.last_source, Some(pen));
        assert_eq!(
            app.drawing
                .active
                .as_ref()
                .unwrap()
                .stroke
                .style()
                .smoothing,
            0.4
        );
        app.workspaces.store.current.pointer.pen.stabilization = None;
        app.drawing.incoming = vec![
            event(Source::MOUSE, Phase::Down, q),
            event(Source::MOUSE, Phase::Move, q),
            event(Source::MOUSE, Phase::Up, q),
            event(pen, Phase::Move, q),
        ];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(app.drawing.pending());
        assert_eq!(app.drawing.ownership.owner(), Some(pen));
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        app.drawing.incoming = vec![event(pen, Phase::Up, q)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.status.as_ref().unwrap().contains("Another device"));
        // A new actual mouse press works immediately after the pen is lifted.
        let m = point(&ctx, &app, [20.5, 30.5]);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![
                egui::Event::PointerMoved(m),
                button(m, true),
                button(m, false),
            ],
        );
        assert_eq!(app.drawing.last_source, Some(Source::MOUSE));
        assert_eq!(app.documents[0].history_counts(), (2, 0));
        let painted = app.documents[0].snapshot();
        app.drawing.incoming = vec![event(pen, Phase::Move, q), event(pen, Phase::Up, q)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(app.documents[0].snapshot() == painted);
        app.drawing.incoming = vec![event(pen, Phase::Down, p), event(pen, Phase::Cancel, p)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(app.documents[0].snapshot() == painted);
        app.workspaces.store.current.pointer.mouse.enabled = false;
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![
                egui::Event::PointerMoved(m),
                button(m, true),
                button(m, false),
            ],
        );
        assert!(app.documents[0].snapshot() == painted);
    }
    #[test]
    fn pen_pressure_reaches_pixels_mouse_uses_one_and_stroke_keeps_mapping() {
        use drawer_core::{
            pointer_input::{Event, Kind, Phase, Source},
            pressure::Mapping,
        };
        for pressure in [0.0, 0.5, 1.0] {
            let (ctx, mut app) = setup(1.0);
            app.panels.settings.brush_size = 12.0;
            app.workspaces.store.current.pressure.tools[0] = Mapping {
                size: false,
                opacity: true,
                ..Default::default()
            };
            let p = point(&ctx, &app, [20.5, 20.5]);
            let source = Source {
                kind: Kind::Pen,
                device: 401,
                contact: 88,
            };
            let event = |phase| Event {
                source,
                phase,
                position: [p.x as f64, p.y as f64],
                pressure: Some(pressure),
                attributes: Default::default(),
                origin: Default::default(),
            };
            app.drawing.incoming = vec![event(Phase::Down)];
            frame(&ctx, &mut app, 1.0, vec![]);
            assert_eq!(
                app.drawing.active.as_ref().unwrap().stroke.style().pressure,
                Mapping {
                    size: false,
                    opacity: true,
                    ..Default::default()
                }
            );
            app.workspaces.store.current.pressure.tools[0] = Mapping {
                size: true,
                opacity: false,
                ..Default::default()
            };
            app.drawing.incoming = vec![event(Phase::Up)];
            frame(&ctx, &mut app, 1.0, vec![]);
            assert_eq!(
                app.documents[0].composite_pixel(20, 20)[3],
                (255.0 * pressure).round() as u8
            );
            assert_eq!(
                app.documents[0].history_counts(),
                (usize::from(pressure > 0.0), 0)
            );
            app.workspaces.store.current.pressure.tools[0] = Mapping {
                size: false,
                opacity: true,
                ..Default::default()
            };
            let mouse = point(&ctx, &app, [40.5, 30.5]);
            frame(
                &ctx,
                &mut app,
                1.0,
                vec![
                    egui::Event::PointerMoved(mouse),
                    button(mouse, true),
                    button(mouse, false),
                ],
            );
            assert_eq!(app.documents[0].composite_pixel(40, 30)[3], 255);
        }
    }
    #[test]
    fn pen_tilt_eraser_button_release_missing_attributes_and_view_transform() {
        use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
        let (ctx, mut app) = setup(1.0);
        app.workspaces.store.current.pointer.features.tilt_angle = true;
        app.panels.settings.brush_size = 12.0;
        app.panels.settings.brush_aspect = 0.25;
        app.documents[0].layers[0].pixels.fill([90, 100, 110, 255]);
        let before = app.documents[0].snapshot();
        let p = point(&ctx, &app, [20.5, 20.5]);
        let source = Source {
            kind: Kind::Pen,
            device: 905,
            contact: 23,
        };
        let event = |phase, attributes| Event {
            source,
            phase,
            position: [p.x as f64, p.y as f64],
            pressure: Some(1.0),
            attributes,
            origin: Default::default(),
        };
        let mut a = Attributes {
            tilt: Some([0.5, 0.0]),
            eraser: Some(true),
            button: None,
            buttons: None,
            tablet: None,
            palm: None,
            touch_frame: None,
        };
        app.drawing.incoming = vec![event(Phase::Down, a)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(
            app.drawing.active.as_ref().unwrap().stroke.style().tip,
            BrushTip::Eraser
        );
        assert!(
            app.drawing
                .active
                .as_ref()
                .unwrap()
                .stroke
                .shape()
                .angle
                .abs()
                < 1e-6
        );
        a.tilt = Some([0.0, 0.5]);
        a.eraser = Some(false);
        app.drawing.incoming = vec![event(Phase::Move, a)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!((app.drawing.active.as_ref().unwrap().stroke.shape().angle - 90.0).abs() < 1e-6);
        app.drawing.incoming = vec![event(Phase::Up, a)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(app.documents[0].tool, BrushTip::Pen);
        assert!(app.documents[0].composite_pixel(20, 20)[3] < 255);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].undo());
        assert!(app.documents[0].snapshot() == before);
        app.workspaces.store.current.pointer.features.button_eraser = true;
        a.button = Some(true);
        a.tilt = None;
        app.drawing.incoming = vec![event(Phase::Down, a)];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(
            app.drawing.active.as_ref().unwrap().stroke.style().tip,
            BrushTip::Eraser
        );
        a.button = Some(false);
        app.drawing.incoming = vec![event(Phase::Up, a)];
        frame(&ctx, &mut app, 1.0, vec![]);
        app.drawing.incoming = vec![event(Phase::Down, Attributes::default())];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert_eq!(
            app.drawing.active.as_ref().unwrap().stroke.style().tip,
            BrushTip::Pen
        );
        assert_eq!(
            app.drawing.active.as_ref().unwrap().stroke.shape().angle,
            0.0
        );
        app.drawing.incoming = vec![event(Phase::Cancel, Attributes::default())];
        frame(&ctx, &mut app, 1.0, vec![]);
        assert!(!app.drawing.pending());
        for ppp in [1.0, 1.5, 2.0] {
            for flipped in [false, true] {
                let projection = Projection {
                    view: View {
                        angle: 37.0,
                        flipped,
                        ..Default::default()
                    },
                    size: [64.0, 48.0],
                    center: [300.0, 200.0],
                    ppp,
                };
                let angle = projection.tilt_angle(Some([0.4, 0.0]));
                let v = [angle.to_radians().cos(), angle.to_radians().sin()];
                let screen =
                    projection
                        .view
                        .document_to_screen(v, projection.size, projection.center, ppp);
                let origin = projection.view.document_to_screen(
                    [0.0; 2],
                    projection.size,
                    projection.center,
                    ppp,
                );
                assert!(screen[0] > origin[0]);
                assert!((screen[1] - origin[1]).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn promoted_mouse_before_during_after_pen_matches_pen_only_and_real_mouse_is_kept() {
        use drawer_core::pointer_input::{Event, Kind, Origin, Phase, Source};
        let mut expected = None;
        for mixed in [false, true] {
            let (ctx, mut app) = setup(1.0);
            app.drawing.pen_available = true;
            app.workspaces.store.current.pressure.tools[0].size = false;
            app.workspaces.store.current.pressure.tools[0].opacity = true;
            let p = point(&ctx, &app, [10.5, 20.5]);
            let q = point(&ctx, &app, [40.5, 20.5]);
            let pen = Source {
                kind: Kind::Pen,
                device: 301,
                contact: 78,
            };
            let event = |source, phase, pos: egui::Pos2, origin| Event {
                source,
                phase,
                position: [pos.x as f64, pos.y as f64],
                pressure: Some(0.5),
                attributes: Default::default(),
                origin,
            };
            let mut events = vec![
                event(pen, Phase::Down, p, Origin::Unmarked),
                event(pen, Phase::Move, q, Origin::Unmarked),
                event(pen, Phase::Up, q, Origin::Unmarked),
            ];
            if mixed {
                events.insert(
                    0,
                    event(Source::MOUSE, Phase::Down, p, Origin::PenCompatibility),
                );
                events.insert(
                    2,
                    event(Source::MOUSE, Phase::Move, q, Origin::PenCompatibility),
                );
                events.push(event(Source::MOUSE, Phase::Up, q, Origin::PenCompatibility));
                events.push(event(
                    Source::MOUSE,
                    Phase::Down,
                    q,
                    Origin::PenCompatibility,
                ));
                events.push(event(Source::MOUSE, Phase::Up, q, Origin::PenCompatibility));
            }
            app.drawing.incoming = events;
            frame(&ctx, &mut app, 1.0, vec![]);
            assert_eq!(app.documents[0].history_counts(), (1, 0));
            if let Some(expected) = &expected {
                assert!(&app.documents[0].layers[0].pixels == expected);
            } else {
                expected = Some(app.documents[0].layers[0].pixels.clone());
            }
            assert_eq!(app.documents[0].composite_pixel(40, 20)[3], 128);
            // Same position, immediately following the pen: its OS origin is a real mouse.
            app.drawing.gui = Some(vec![
                event(Source::MOUSE, Phase::Down, q, Origin::Unmarked),
                event(Source::MOUSE, Phase::Up, q, Origin::Unmarked),
            ]);
            frame(&ctx, &mut app, 1.0, vec![]);
            assert_eq!(app.documents[0].history_counts(), (2, 0));
            assert_eq!(app.documents[0].composite_pixel(40, 20)[3], 255);
            // A failed/unavailable native pen provider retains compatible mouse fallback.
            app.drawing.pen_available = false;
            let r = point(&ctx, &app, [20.5, 30.5]);
            app.drawing.gui = Some(vec![
                event(Source::MOUSE, Phase::Down, r, Origin::PenCompatibility),
                event(Source::MOUSE, Phase::Up, r, Origin::PenCompatibility),
            ]);
            frame(&ctx, &mut app, 1.0, vec![]);
            assert_eq!(app.documents[0].history_counts(), (3, 0));
        }
    }
    #[test]
    fn mouse_and_pen_without_pressure_tilt_match_full_pressure_for_every_tip() {
        use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
        for tip in BrushTip::ALL {
            let mut expected = None;
            for (kind, pressure, tilt) in [
                (Kind::Pen, Some(1.0), Some([0.0; 2])),
                (Kind::Mouse, None, None),
                (Kind::Pen, None, None),
            ] {
                let (ctx, mut app) = setup(1.0);
                app.set_tool(tip);
                app.panels.settings.brush_size = 8.0;
                let n = BrushTip::ALL.iter().position(|t| *t == tip).unwrap();
                app.workspaces.store.current.pressure.tools[n].size = true;
                app.workspaces.store.current.pressure.tools[n].opacity = true;
                app.workspaces.store.current.pointer.features.tilt_angle = true;
                if tip == BrushTip::Eraser {
                    app.documents[0].layers[0].pixels.fill([80, 90, 100, 255]);
                }
                frame(&ctx, &mut app, 1.0, vec![]);
                let before = app.documents[0].snapshot();
                let p = point(&ctx, &app, [10.5, 20.5]);
                let q = point(&ctx, &app, [45.5, 20.5]);
                let source = if kind == Kind::Mouse {
                    Source::MOUSE
                } else {
                    Source {
                        kind,
                        device: 711,
                        contact: 32,
                    }
                };
                let event = |phase, pos: egui::Pos2| Event {
                    source,
                    phase,
                    position: [pos.x as f64, pos.y as f64],
                    pressure,
                    attributes: Attributes {
                        tilt,
                        ..Default::default()
                    },
                    origin: Default::default(),
                };
                app.drawing.incoming = vec![
                    event(Phase::Down, p),
                    event(Phase::Move, q),
                    event(Phase::Up, q),
                ];
                frame(&ctx, &mut app, 1.0, vec![]);
                assert_eq!(app.documents[0].history_counts(), (1, 0));
                assert!((10..=45).all(|x| if tip == BrushTip::Eraser {
                    app.documents[0].composite_pixel(x, 20)[3] < 255
                } else {
                    app.documents[0].composite_pixel(x, 20)[3] > 0
                }));
                if let Some(expected) = &expected {
                    assert!(&app.documents[0].layers[0].pixels == expected);
                } else {
                    expected = Some(app.documents[0].layers[0].pixels.clone());
                }
                let after = app.documents[0].snapshot();
                assert!(app.documents[0].undo());
                assert!(app.documents[0].snapshot() == before);
                assert!(app.documents[0].redo());
                assert!(app.documents[0].snapshot() == after);
            }
        }
    }
    #[test]
    fn mouse_fast_drag_preview_one_undo_eraser_and_view_coordinates() {
        let mut expected = None;
        for (ppp, angle, mirror) in [(1.0, 0.0, false), (1.5, 37.0, true), (2.0, -31.0, false)] {
            let (ctx, mut app) = setup(ppp);
            app.documents[0].view.angle = angle;
            app.documents[0].view.flipped = mirror;
            frame(&ctx, &mut app, ppp, vec![]);
            let before = app.documents[0].snapshot();
            let start = point(&ctx, &app, [10.5, 20.5]);
            let end = point(&ctx, &app, [40.5, 20.5]);
            frame(
                &ctx,
                &mut app,
                ppp,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            assert!(app.drawing.pending());
            assert_eq!(app.documents[0].history_counts(), (0, 0));
            assert!(!app.documents[0].dirty);
            assert!(app.documents[0].snapshot() == before);
            assert!(
                Renderer::preview(&app.documents[0], app.drawing.preview(app.documents[0].id))
                    .pixel([10.5, 20.5])[3]
                    > 0
            );
            assert_ne!(app.canvas_view.rendered_stroke(), (0, 0));
            frame(
                &ctx,
                &mut app,
                ppp,
                vec![egui::Event::PointerMoved(end), button(end, false)],
            );
            assert!(!app.drawing.pending());
            assert_eq!(app.documents[0].history_counts(), (1, 0));
            assert!((10..=40).all(|x| app.documents[0].layers[0].pixels[20 * 64 + x][3] > 0));
            if let Some(expected) = &expected {
                assert!(&app.documents[0].layers[0].pixels == expected);
            } else {
                expected = Some(app.documents[0].layers[0].pixels.clone());
            }
            let painted = app.documents[0].snapshot();
            app.set_tool(BrushTip::Eraser);
            app.panels.settings.brush_size = 1.0;
            frame(&ctx, &mut app, ppp, vec![]);
            frame(
                &ctx,
                &mut app,
                ppp,
                vec![
                    egui::Event::PointerMoved(start),
                    button(start, true),
                    egui::Event::PointerMoved(end),
                    button(end, false),
                ],
            );
            assert_eq!(app.documents[0].history_counts(), (2, 0));
            assert!(
                app.documents[0].layers[0].pixels[20 * 64 + 20][3]
                    < painted.layers[0].pixels[20 * 64 + 20][3]
            );
            assert!(app.documents[0].undo());
            assert!(app.documents[0].snapshot() == painted);
            assert!(app.documents[0].undo());
            assert!(app.documents[0].snapshot() == before);
            assert!(!app.documents[0].dirty);
        }
    }
    #[test]
    fn missing_release_escape_same_frame_click_frozen_tool_and_quit_finish() {
        let (ctx, mut app) = setup(1.0);
        let start = point(&ctx, &app, [10.5, 20.5]);
        let end = point(&ctx, &app, [30.5, 20.5]);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![
                egui::Event::PointerMoved(start),
                button(start, true),
                button(start, false),
            ],
        );
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        let after_click = app.documents[0].snapshot();
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![button(start, true), egui::Event::PointerMoved(end)],
        );
        assert!(app.drawing.pending());
        frame(&ctx, &mut app, 1.0, vec![egui::Event::WindowFocused(false)]);
        assert!(!app.drawing.pending());
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].snapshot() == after_click);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![
                egui::Event::WindowFocused(true),
                egui::Event::PointerMoved(start),
            ],
        );
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        let before = app.documents[0].snapshot();
        frame(&ctx, &mut app, 1.0, vec![button(end, true)]);
        frame(
            &ctx,
            &mut app,
            1.0,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(!app.drawing.pending());
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].snapshot() == before);
        frame(&ctx, &mut app, 1.0, vec![button(end, false)]);
        app.set_tool(BrushTip::Pen);
        frame(&ctx, &mut app, 1.0, vec![button(start, true)]);
        assert!(app.drawing.pending());
        app.set_tool(BrushTip::Eraser);
        assert_eq!(
            app.drawing.active.as_ref().unwrap().stroke.style().tip,
            BrushTip::Pen
        );
        let next = point(&ctx, &app, [10.5, 30.5]);
        frame(&ctx, &mut app, 1.0, vec![egui::Event::PointerMoved(next)]);
        app.begin_quit();
        assert!(!app.drawing.pending());
        assert!(app.pending_close.is_some());
        assert_eq!(app.documents[0].history_counts(), (2, 0));
    }
    #[test]
    fn cursor_modes_use_identical_strokes_and_empty_selection_cannot_paint() {
        let mut expected = None;
        for mode in [Mode::Raw, Mode::Corrected, Mode::Both] {
            let (ctx, mut app) = setup(1.0);
            app.workspaces.store.current.cursors.tools[0].mode = mode;
            app.workspaces.store.current.input.smoothing[0] = 0.6;
            frame(&ctx, &mut app, 1.0, vec![]);
            let start = point(&ctx, &app, [5.5, 10.5]);
            let corner = point(&ctx, &app, [40.5, 10.5]);
            let end = point(&ctx, &app, [40.5, 40.5]);
            frame(
                &ctx,
                &mut app,
                1.0,
                vec![
                    egui::Event::PointerMoved(start),
                    button(start, true),
                    egui::Event::PointerMoved(corner),
                    egui::Event::PointerMoved(end),
                ],
            );
            let outside = app.canvas_view.viewport.unwrap().max + egui::vec2(40.0, 40.0);
            frame(&ctx, &mut app, 1.0, vec![button(outside, false)]);
            assert_eq!(app.documents[0].history_counts(), (1, 0));
            if let Some(expected) = &expected {
                assert!(&app.documents[0].layers[0].pixels == expected);
            } else {
                expected = Some(app.documents[0].layers[0].pixels.clone());
            }
            app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask::default());
            let before = app.documents[0].snapshot();
            frame(
                &ctx,
                &mut app,
                1.0,
                vec![
                    egui::Event::PointerMoved(start),
                    button(start, true),
                    button(start, false),
                ],
            );
            assert!(app.documents[0].snapshot() == before);
            assert_eq!(app.documents[0].history_counts(), (1, 0));
        }
    }
}
