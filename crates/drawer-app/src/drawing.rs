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
}
#[derive(Default)]
pub(super) struct Drawing {
    active: Option<Session>,
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
    pub fn finish(&mut self, documents: &mut [Document]) {
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
        if let Some(s) = self.active.take()
            && let Err(e) = s.stroke.commit(doc)
        {
            self.error = Some(e);
        }
    }
    fn sample(&mut self, doc: &Document, pos: egui::Pos2) {
        if let Some(s) = &mut self.active
            && let Err(e) = s.stroke.sample(doc, s.projection.point(pos))
        {
            self.error = Some(e);
            self.active = None;
        }
    }
    pub fn advance(&mut self, ctx: &egui::Context, documents: &mut [Document]) {
        if self
            .active
            .as_ref()
            .is_some_and(|s| (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8)
        {
            self.finish(documents);
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            let Some(id) = self.active.as_ref().map(|s| s.stroke.document) else {
                break;
            };
            let Some(doc) = documents.iter_mut().find(|d| d.id == id) else {
                self.active = None;
                break;
            };
            match event {
                egui::Event::PointerMoved(pos) => self.sample(doc, pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    self.sample(doc, pos);
                    self.finish_document(doc);
                }
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                }
                | egui::Event::PointerGone
                | egui::Event::WindowFocused(false) => self.active = None,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.active = None;
                    ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                    });
                }
                _ => {}
            }
        }
        if ctx.input(|i| !i.focused || !i.pointer.primary_down()) {
            self.active = None;
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
        let rect = response.rect;
        let projection = Projection {
            view: doc.view,
            size: [doc.width as f64, doc.height as f64],
            center: [rect.center().x as f64, rect.center().y as f64],
            ppp: ui.ctx().pixels_per_point() as f64,
        };
        // Event positions are authoritative even when a complete fast drag arrives
        // in one frame and egui's final pointer position is outside the canvas.
        let started = self.active.is_none() && enabled;
        if started {
            for event in ui.input(|i| i.events.clone()) {
                match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } if rect.contains(pos)
                        && ui.clip_rect().contains(pos)
                        && ui.ctx().layer_id_at(pos) == Some(response.layer_id) =>
                    {
                        self.finish_document(doc);
                        match layer
                            .ok_or_else(|| "Select a raster layer to paint".to_string())
                            .and_then(|layer| Stroke::begin(doc, layer, style))
                        {
                            Ok(stroke) => {
                                self.active = Some(Session {
                                    stroke,
                                    view,
                                    projection,
                                });
                                self.sample(doc, pos);
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    egui::Event::PointerMoved(pos) => self.sample(doc, pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    } => {
                        self.sample(doc, pos);
                        self.finish_document(doc);
                    }
                    egui::Event::PointerGone | egui::Event::WindowFocused(false) => {
                        self.active = None;
                    }
                    _ => {}
                }
            }
        } else if let Some(s) = &mut self.active
            && s.stroke.document == doc.id
            && s.view == view
            && s.projection != projection
        {
            s.projection = projection;
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                self.sample(doc, pos);
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
