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
    events: Vec<(drawer_core::pointer_input::Event, bool)>,
    pub incoming: Vec<drawer_core::pointer_input::Event>,
    pub settings: drawer_core::pointer_input::Settings,
    pub pressure: drawer_core::pressure::Settings,
    pub last_source: Option<drawer_core::pointer_input::Source>,
    pub last_attributes: drawer_core::pointer_input::Attributes,
    pub error: Option<String>,
}
impl Drawing {
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
        self.events = self
            .incoming
            .drain(..)
            .chain(crate::pointer_input::gui_events(ctx))
            .map(|e| (e, false))
            .collect();
    }
    pub fn finish(&mut self, documents: &mut [Document]) {
        self.ownership.clear();
        if let Some(session) = self.active.take() {
            if let Some(doc) = documents
                .iter_mut()
                .find(|d| d.id == session.stroke.document)
            {
                if let Err(e) = session.stroke.commit(doc) {
                    self.error = Some(e);
                }
            } else {
                self.error = Some("The drawing was closed; pending stroke discarded".into());
            }
        }
    }
    fn finish_document(&mut self, doc: &mut Document) {
        self.ownership.clear();
        if let Some(s) = self.active.take()
            && let Err(e) = s.stroke.commit(doc)
        {
            self.error = Some(e);
        }
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
        let event = self.events[index].0;
        let pressure = if event.source.kind == drawer_core::pointer_input::Kind::Mouse {
            1.0
        } else {
            event.pressure.unwrap_or(1.0)
        };
        self.events[index].1 = true;
        match self.ownership.receive(event, self.settings) {
            Decision::Sample => self.sample(
                doc,
                egui::pos2(event.position[0] as f32, event.position[1] as f32),
                pressure,
                event.attributes,
            ),
            Decision::Finish => {
                self.sample(
                    doc,
                    egui::pos2(event.position[0] as f32, event.position[1] as f32),
                    pressure,
                    event.attributes,
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
            self.cancel();
            self.events.clear();
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
            return;
        }
        for n in 0..self.events.len() {
            let Some(id) = self.active.as_ref().map(|s| s.stroke.document) else {
                break;
            };
            let Some(doc) = documents.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive(n, doc);
        }
        if self
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
        let projection = Projection {
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
            let event = self.events[n].0;
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
            style.tip = self
                .settings
                .features
                .tip(event.source.kind, event.attributes, style.tip);
            if let Some(value) = self
                .settings
                .for_kind(event.source.kind)
                .and_then(|s| s.stabilization)
            {
                style.smoothing = value;
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
                    let pressure = if event.source.kind == drawer_core::pointer_input::Kind::Mouse {
                        1.0
                    } else {
                        event.pressure.unwrap_or(1.0)
                    };
                    self.sample(doc, pos, pressure, event.attributes);
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
        self.drawing.finish(&mut self.documents);
        if let Some(e) = self.drawing.error.take() {
            self.status = Some(e);
        }
    }
    pub(super) fn process_drawing(&mut self, ctx: &egui::Context) {
        self.drawing.settings = self.workspaces.store.current.pointer;
        self.drawing.pressure = self.workspaces.store.current.pressure;
        self.native_input.pump(ctx, &mut self.drawing.incoming);
        self.drawing.prepare(ctx);
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

pub(super) fn smoke_attributes() -> Result<(), String> {
    use drawer_core::{
        brush_cursor::Shape,
        pointer_input::{Attributes, Kind, PenFeatures},
    };
    let a = Attributes {
        tilt: Some([0.0, 0.5]),
        eraser: Some(true),
        button: Some(false),
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
        };
        let mut a = Attributes {
            tilt: Some([0.5, 0.0]),
            eraser: Some(true),
            button: None,
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
