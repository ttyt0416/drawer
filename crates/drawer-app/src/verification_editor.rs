use crate::*;
pub(super) struct Editor {
    pub ctx: egui::Context,
    pub app: Drawer,
    pub ppp: f32,
    time: f64,
}
impl Editor {
    pub fn new(ppp: f32) -> Result<Self, String> {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.simple_view = true;
        app.set_tool(BrushTip::Pen);
        app.panels.settings.brush_size = 1.;
        app.panels.settings.brush_spacing = [0.1; 7];
        app.workspaces.store.current.input.smoothing = [0.; 7];
        for t in &mut app.workspaces.store.current.pressure.tools {
            t.size = false;
            t.opacity = false;
        }
        for t in &mut app.workspaces.store.current.input.mouse_pressure {
            t.enabled = false;
        }
        app.references.session.workspace.color = [180, 20, 40, 255];
        app.documents[0].mark_saved("acceptance-baseline.drawer".into());
        let mut e = Self {
            ctx,
            app,
            ppp,
            time: 0.,
        };
        e.frame(vec![]);
        e.frame(vec![]);
        Ok(e)
    }
    pub fn frame(&mut self, events: Vec<egui::Event>) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 760.),
            )),
            events,
            time: Some(self.time),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(self.ppp);
        let mut out = self.ctx.run_ui(input, |ui| self.app.show(ui));
        out.textures_delta.clear();
        self.time += 0.01;
    }
    /// Independent forward projection. Never calls the production forward/inverse pair.
    pub fn point(&self, document: [f64; 2]) -> Result<egui::Pos2, String> {
        let rect = self
            .app
            .canvas_view
            .viewport_rect()
            .ok_or("Acceptance viewport missing")?;
        let v = self.app.documents[0].view;
        let mut x = document[0] - 32.;
        let y = document[1] - 24.;
        if v.flipped {
            x = -x;
        }
        let angle = v.angle * std::f64::consts::PI / 180.;
        let scale = v.zoom / f64::from(self.ctx.pixels_per_point());
        Ok(egui::pos2(
            (f64::from(rect.center().x) + v.pan[0] + (x * angle.cos() - y * angle.sin()) * scale)
                as f32,
            (f64::from(rect.center().y) + v.pan[1] + (x * angle.sin() + y * angle.cos()) * scale)
                as f32,
        ))
    }
}
pub(super) fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
pub(super) fn check(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}
