use drawer_core::document::Document;
use eframe::egui;

pub struct CanvasView {
    image: Option<((u64, u64), egui::TextureHandle)>,
    checker: egui::TextureHandle,
    #[cfg(test)]
    pub viewport: Option<egui::Rect>,
}
impl CanvasView {
    pub fn new(ctx: &egui::Context) -> Self {
        let gray = |v| egui::Color32::from_gray(v);
        Self {
            image: None,
            checker: ctx.load_texture(
                "transparency-grid",
                egui::ColorImage::new([2, 2], vec![gray(196), gray(224), gray(224), gray(196)]),
                egui::TextureOptions {
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    ..egui::TextureOptions::NEAREST
                },
            ),
            #[cfg(test)]
            viewport: None,
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, _index: usize, document: &mut Document) -> bool {
        let before = document.view;
        ui.horizontal(|ui| {
            ui.label("Zoom");
            let mut percent = document.view.zoom * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .range(1.0..=6400.0)
                        .suffix("%"),
                )
                .changed()
                && percent.is_finite()
            {
                document.view.zoom = (percent / 100.0).clamp(0.01, 64.0);
            }
            ui.label("Angle");
            let mut angle = document.view.angle;
            if ui
                .add(egui::DragValue::new(&mut angle).speed(0.5).suffix("°"))
                .changed()
            {
                document.view.rotate(angle);
            }
            ui.checkbox(&mut document.view.flipped, "Mirror");
            if ui.button("Reset view").clicked() {
                document.view = Default::default();
            }
        });
        ui.label("Wheel to zoom · Middle drag or Space + drag to pan");
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(egui::vec2(1.0, 1.0)),
            egui::Sense::drag(),
        );
        #[cfg(test)]
        {
            self.viewport = Some(rect);
        }
        let center = [rect.center().x as f64, rect.center().y as f64];
        let size = [document.width as f64, document.height as f64];
        let ppp = ui.ctx().pixels_per_point() as f64;
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && ui.input(|input| input.key_down(egui::Key::Space)))
        {
            let delta = response.drag_delta();
            document.view.pan_by([delta.x as f64, delta.y as f64]);
        }
        if response.hovered() {
            let wheel = ui.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta).y);
            if wheel != 0.0
                && let Some(pos) = response.hover_pos()
            {
                document.view.zoom_about(
                    (wheel as f64 * 0.002).exp(),
                    [pos.x as f64, pos.y as f64],
                    size,
                    center,
                    ppp,
                );
            }
        }
        if self.image.as_ref().map(|(active, _)| *active) != Some((document.id, document.revision))
        {
            let reduction = document.width.max(document.height).div_ceil(2048).max(1);
            let width = document.width.div_ceil(reduction);
            let height = document.height.div_ceil(reduction);
            let mut pixels = Vec::with_capacity((width * height) as usize);
            for y in 0..height {
                for x in 0..width {
                    let pixel = document.composite_pixel(x * reduction, y * reduction);
                    pixels.push(egui::Color32::from_rgba_unmultiplied(
                        pixel[0], pixel[1], pixel[2], pixel[3],
                    ));
                }
            }
            let texture = ui.ctx().load_texture(
                "canvas-preview",
                egui::ColorImage::new([width as usize, height as usize], pixels),
                egui::TextureOptions::NEAREST,
            );
            self.image = Some(((document.id, document.revision), texture));
        }
        let corners = [
            [0.0, 0.0],
            [size[0], 0.0],
            [size[0], size[1]],
            [0.0, size[1]],
        ]
        .map(|point| {
            let pos = document.view.document_to_screen(point, size, center, ppp);
            egui::pos2(pos[0] as f32, pos[1] as f32)
        });
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(32));
        for (texture, uv_max) in [
            (
                self.checker.id(),
                egui::pos2(document.width as f32 / 32.0, document.height as f32 / 32.0),
            ),
            (self.image.as_ref().unwrap().1.id(), egui::pos2(1.0, 1.0)),
        ] {
            let mut mesh = egui::Mesh::with_texture(texture);
            let uv = [
                egui::pos2(0.0, 0.0),
                egui::pos2(uv_max.x, 0.0),
                uv_max,
                egui::pos2(0.0, uv_max.y),
            ];
            for (pos, uv) in corners.into_iter().zip(uv) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv,
                    color: egui::Color32::WHITE,
                });
            }
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            painter.add(egui::Shape::mesh(mesh));
        }
        document.view != before
    }
}
