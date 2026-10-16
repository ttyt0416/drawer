use drawer_core::document::Document;
use eframe::egui;

type ImageKey = (u64, u64, u32, (u64, u64));
pub struct CanvasView {
    proof_settings: Option<drawer_core::soft_proof::Settings>,
    proof: Option<std::sync::Arc<drawer_core::soft_proof::Proof>>,
    pub(super) proof_error: Option<String>,
    pub(super) preview_sample: Option<([u8; 4], [u8; 4])>,
    image: Option<(ImageKey, egui::TextureHandle)>,
    checker: egui::TextureHandle,
    pub move_delta: Option<[f64; 2]>,
    pub filter: drawer_core::interpolation::Filter,
    screen_image: Option<(ScreenKey, egui::TextureHandle)>,
    pub area_zoom: bool,
    area_start: Option<egui::Pos2>,
    pub view_notice: Option<String>,
    pub navigator: drawer_core::navigator_preferences::Preferences,
    pub navigator_generation: u64,
    pub simple: bool,
    pub compact_controls: bool,
    pub dock_navigator: bool,
    last_viewport: Option<egui::Rect>,
    last_layer: Option<egui::LayerId>,
    pub brush_shape: drawer_core::brush_cursor::Shape,
    pub first_point: Option<(u64, [f64; 2])>,
    pub drag_pixels: f64,
    pub input_active: bool,
    pub behavior: drawer_core::ui_behavior::Settings,
    pub working: bool,
    pub cursors: drawer_core::brush_cursor::Settings,
    pub smoothing: [f64; 7],
    pub profiles: [drawer_core::tip_profile::Settings; 7],
    pub filters: [drawer_core::stroke_filter::Settings; 7],
    pub cursor_trace: drawer_core::brush_cursor::Trace,
    pub measurement_mode: bool,
    pub measurement: Option<([f64; 2], [f64; 2])>,
    pan_drag: Option<drawer_core::tool_input::Drag>,
    input_document: Option<u64>,
    input_blocked: bool,
    tool_icons: Vec<egui::TextureHandle>,
    #[cfg(test)]
    pub brush_outline: Vec<egui::Pos2>,
    #[cfg(test)]
    pub cursor_rect: Option<egui::Rect>,
    #[cfg(test)]
    pub navigator_outline: bool,
    #[cfg(test)]
    pub viewport: Option<egui::Rect>,
    #[cfg(test)]
    pub navigator_rect: Option<egui::Rect>,
}
#[derive(Clone, Copy, PartialEq)]
struct ScreenKey {
    id: u64,
    revision: u64,
    frame: u32,
    view: drawer_core::view::View,
    size: [usize; 2],
    ppp: f64,
    filter: drawer_core::interpolation::Filter,
    stroke: (u64, u64),
}
impl CanvasView {
    pub(super) fn viewport_rect(&self) -> Option<egui::Rect> {
        self.last_viewport
    }
    pub(super) fn viewport_layer(&self) -> Option<egui::LayerId> {
        self.last_layer
    }
    pub fn icon(&self, n: usize) -> egui::TextureId {
        self.tool_icons[n].id()
    }
    pub fn cursor_tool_count(&self) -> usize {
        self.tool_icons.len()
    }
    pub fn new(ctx: &egui::Context) -> Self {
        let gray = |v| egui::Color32::from_gray(v);
        Self {
            image: None,
            proof_settings: None,
            proof: None,
            proof_error: None,
            preview_sample: None,
            move_delta: None,
            filter: drawer_core::interpolation::Filter::Nearest,
            screen_image: None,
            area_zoom: false,
            area_start: None,
            view_notice: None,
            navigator: Default::default(),
            navigator_generation: 0,
            simple: false,
            compact_controls: false,
            dock_navigator: false,
            last_viewport: None,
            last_layer: None,
            brush_shape: Default::default(),
            first_point: None,
            drag_pixels: 4.0,
            input_active: true,
            behavior: Default::default(),
            working: false,
            cursors: Default::default(),
            smoothing: [0.0; 7],
            profiles: [Default::default(); 7],
            filters: [Default::default(); 7],
            cursor_trace: Default::default(),
            measurement_mode: false,
            measurement: None,
            pan_drag: None,
            input_document: None,
            input_blocked: false,
            tool_icons: drawer_core::assets::BrushTip::ALL
                .into_iter()
                .map(|tip| {
                    ctx.load_texture(
                        format!("{} cursor", tip.name()),
                        egui::ColorImage::new(
                            [16, 16],
                            drawer_core::assets::tool_icon(tip, 16)
                                .into_iter()
                                .map(|p| {
                                    egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
                                })
                                .collect(),
                        ),
                        egui::TextureOptions::NEAREST,
                    )
                })
                .collect(),
            #[cfg(test)]
            brush_outline: Vec::new(),
            #[cfg(test)]
            cursor_rect: None,
            #[cfg(test)]
            navigator_outline: false,
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
            #[cfg(test)]
            navigator_rect: None,
        }
    }
    pub(super) fn set_proof(&mut self, settings: &drawer_core::soft_proof::Settings) {
        if self.proof_settings.as_ref() == Some(settings) {
            return;
        }
        self.proof_settings = Some(settings.clone());
        self.image = None;
        self.screen_image = None;
        self.preview_sample = None;
        match drawer_core::soft_proof::Proof::read(settings) {
            Ok(p) => {
                self.proof = settings.enabled.then(|| std::sync::Arc::new(p));
                self.proof_error = None;
            }
            Err(e) => {
                self.proof = None;
                self.proof_error = Some(e);
            }
        }
    }
    pub(super) fn reload_proof(&mut self) {
        self.proof_settings = None;
    }
    fn proof_notice(&self, ui: &mut egui::Ui) {
        if let Some(e) = &self.proof_error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("View proof unavailable; showing document color: {e}"),
            );
        } else if self.proof.is_some() {
            ui.label("View-only ICC / tone preview · document pixels unchanged");
        }
    }
    fn displayed_color(&self, color: [u8; 4]) -> [u8; 4] {
        self.proof.as_ref().map_or(color, |p| p.map(color))
    }
    pub fn cancel_input(&mut self) {
        self.pan_drag = None;
        self.area_start = None;
        self.first_point = None;
        self.cursor_trace.clear();
        self.input_blocked = true;
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        overlay: Option<drawer_core::scene::Rect>,
        navigator_ui: &mut egui::Ui,
        aids: drawer_core::aids::AidPreferences,
        paint: crate::drawing::PaintInput<'_>,
    ) -> bool {
        self.proof_notice(ui);
        if self.input_document != Some(document.id) {
            self.cancel_input();
            self.input_document = Some(document.id);
        }
        if ui.input(|i| !i.pointer.any_down() || i.pointer.any_pressed()) {
            self.input_blocked = false;
        }
        let input_active = ui.is_enabled()
            && self.input_active
            && !self.input_blocked
            && ui.input(|i| {
                i.focused
                    && !i
                        .events
                        .iter()
                        .any(|e| matches!(e, egui::Event::WindowFocused(false)))
            });
        self.working = false;
        let before = document.view;
        let size = [document.width as f64, document.height as f64];
        let ppp = ui.ctx().pixels_per_point() as f64;
        if !self.compact_controls {
            ui.horizontal_wrapped(|ui| {
                ui.label("Zoom");
                let mut percent = document.view.zoom * 100.0;
                if super::edit::number(ui, &mut percent, 1.0_f64..=6400.0_f64, 1.0, "%", "view")
                    .changed()
                    && percent.is_finite()
                {
                    document.view.zoom = (percent / 100.0).clamp(0.01, 64.0);
                }
                ui.label("Angle");
                let mut angle = document.view.angle;
                if super::edit::number(ui, &mut angle, -1e9_f64..=1e9_f64, 0.5, "°", "view")
                    .changed()
                {
                    document.view.rotate(angle);
                }
                crate::help::checkbox(ui, &mut document.view.flipped, "Mirror", "view");
                if crate::help::button(ui, "100%", "view").clicked() {
                    document.view.reset_zoom(size, ppp);
                }
                if crate::help::button(ui, "0°", "view").clicked() {
                    document.view.reset_angle(size, ppp);
                }
                if crate::help::button(ui, "Center", "view").clicked() {
                    document.view.pan = [0.0; 2];
                }
                if crate::help::button(ui, "Reset view", "view").clicked() {
                    document.view = Default::default();
                }
                if crate::help::checkbox(ui, &mut self.area_zoom, "Zoom region", "view").changed() {
                    self.area_start = None;
                    self.view_notice = None;
                }
            });
            ui.label("Wheel to zoom · Middle drag or Space + drag to pan");
        }
        if let Some(notice) = &self.view_notice {
            ui.label(notice);
        }
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(egui::vec2(1.0, 1.0)),
            egui::Sense::click_and_drag(),
        );
        self.last_viewport = Some(rect);
        self.last_layer = Some(response.layer_id);
        crate::help::tag(ui.ctx(), &response, "tools");
        #[cfg(test)]
        {
            self.viewport = Some(rect);
        }
        let center = [rect.center().x as f64, rect.center().y as f64];
        let mouse_view_input = input_active && paint.drawing.mouse_view_input();
        if !mouse_view_input {
            self.area_start = None;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.area_start = None;
            self.area_zoom = false;
        }
        if mouse_view_input && self.area_zoom && !ui.input(|i| i.key_down(egui::Key::Space)) {
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.area_start = ui.input(|i| i.pointer.press_origin());
            }
            if response.drag_stopped_by(egui::PointerButton::Primary)
                && let Some(start) = self.area_start.take()
                && let Some(end) = response.interact_pointer_pos()
            {
                let selection = egui::Rect::from_two_pos(start, end).intersect(rect);
                if start.distance(end) as f64 * ppp + 1e-9 < self.drag_pixels {
                    self.view_notice = Some("Move beyond the configured drag threshold.".into());
                } else {
                    self.view_notice = document
                        .view
                        .fit_screen_region(
                            drawer_core::scene::Rect {
                                min: [selection.min.x as f64, selection.min.y as f64],
                                max: [selection.max.x as f64, selection.max.y as f64],
                            },
                            drawer_core::scene::Rect {
                                min: [rect.min.x as f64, rect.min.y as f64],
                                max: [rect.max.x as f64, rect.max.y as f64],
                            },
                            size,
                            ppp,
                        )
                        .err();
                }
            }
            if response.clicked() {
                self.view_notice =
                    Some("Drag a region larger than one document pixel in each direction.".into());
            }
        }
        if mouse_view_input
            && (response.dragged_by(egui::PointerButton::Middle)
                || (response.dragged_by(egui::PointerButton::Primary)
                    && ui.input(|i| i.key_down(egui::Key::Space))))
        {
            if let Some(pos) = response.interact_pointer_pos() {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                let drag = self.pan_drag.get_or_insert_with(|| {
                    drawer_core::tool_input::Drag::new([origin.x as f64, origin.y as f64])
                });
                if let Some(delta) =
                    drag.update([pos.x as f64, pos.y as f64], self.drag_pixels, ppp)
                {
                    document.view.pan_by(delta);
                }
            }
        } else {
            self.pan_drag = None;
        }
        self.working = (paint.drawing.active_view() == Some(paint.view)
            || ui.input(|i| i.pointer.primary_pressed()))
            && !self.measurement_mode
            && input_active
            && !self.area_zoom
            && !ui.input(|i| i.key_down(egui::Key::Space))
            && response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.primary_down() && i.pointer.latest_pos().is_some());
        if self.working
            && self.behavior.edge_pan
            && !ui.input(|i| i.pointer.primary_pressed())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let delta = drawer_core::ui_behavior::edge_delta(
                [pos.x as f64, pos.y as f64],
                drawer_core::scene::Rect {
                    min: [rect.min.x as f64, rect.min.y as f64],
                    max: [rect.max.x as f64, rect.max.y as f64],
                },
                ppp,
                ui.input(|i| i.unstable_dt as f64),
                self.behavior,
            );
            document.view.pan_by(delta);
            if delta != [0.0; 2] {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if response.hovered() && self.area_start.is_none() {
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
        let tip_index = drawer_core::assets::BrushTip::ALL
            .iter()
            .position(|t| *t == document.tool)
            .unwrap();
        let style = drawer_core::painting::Style {
            clone_stamp: Default::default(),
            variation: Default::default(),
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Default::default(),
            texture: Default::default(),
            material: Default::default(),
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.15,
            shape: self.brush_shape,
            tip: document.tool,
            color: paint.color,
            smoothing: self.smoothing[tip_index],
            pressure: paint.drawing.pressure.tools[tip_index],
        };
        paint.drawing.canvas(
            ui,
            &response,
            document,
            paint.view,
            paint.layer,
            style,
            input_active
                && !self.measurement_mode
                && !self.area_zoom
                && !ui.input(|i| i.key_down(egui::Key::Space)),
        );
        self.working &= paint.drawing.active_view() == Some(paint.view);
        let stroke = paint.drawing.preview(document.id);
        self.update_image(ui, document, stroke);
        #[cfg(test)]
        {
            if !self.dock_navigator {
                self.navigator_rect = None;
            }
        }
        if self.navigator.visible
            && !self.simple
            && (self.navigator.floating || !self.dock_navigator)
        {
            if self.navigator.floating {
                let mut open = true;
                let response = egui::Window::new("Navigator")
                    .id(egui::Id::new((
                        "floating-navigator",
                        self.navigator_generation,
                    )))
                    .open(&mut open)
                    .collapsible(false)
                    .default_pos(self.navigator.position)
                    .default_size([self.navigator.size[0], self.navigator.size[1] + 24.0])
                    .min_width(self.navigator.min_width)
                    .max_width(self.navigator.max_width)
                    .min_height(80.0)
                    .max_height(640.0)
                    .show(ui.ctx(), |ui| self.show_navigator(ui, document, rect, ppp));
                if let Some(window) = response {
                    self.navigator.position =
                        [window.response.rect.min.x, window.response.rect.min.y];
                }
                self.navigator.visible = open;
            } else {
                self.show_navigator(navigator_ui, document, rect, ppp);
            }
        }
        self.paint_paper(ui, document, document.view, rect, stroke);
        let painter = ui.painter_at(rect);
        if let Some(start) = self.area_start
            && let Some(end) = response.interact_pointer_pos()
        {
            painter.rect_stroke(
                egui::Rect::from_two_pos(start, end).intersect(rect),
                0.0,
                egui::Stroke::new(1.5, egui::Color32::YELLOW),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(boundary) = overlay {
            let outline = [
                boundary.min,
                [boundary.max[0], boundary.min[1]],
                boundary.max,
                [boundary.min[0], boundary.max[1]],
            ]
            .map(|point| {
                let point = document.view.document_to_screen(point, size, center, ppp);
                egui::pos2(point[0] as f32, point[1] as f32)
            });
            for i in 0..4 {
                painter.line_segment(
                    [outline[i], outline[(i + 1) % 4]],
                    egui::Stroke::new(2.0, egui::Color32::YELLOW),
                );
            }
        }
        if let Some((start, end)) = self.measurement {
            let project = |p: [f64; 2]| {
                let q = document.view.document_to_screen(p, size, center, ppp);
                egui::pos2(q[0] as f32, q[1] as f32)
            };
            let a = project(start);
            let b = project(end);
            painter.line_segment([a, b], egui::Stroke::new(1.5, egui::Color32::LIGHT_BLUE));
            for p in [a, b] {
                painter.circle_stroke(p, 3.0, egui::Stroke::new(1.5, egui::Color32::LIGHT_BLUE));
            }
        }
        self.paint_aids(ui, document, rect, aids, ppp);
        self.first_point = None;
        if input_active
            && !self.area_zoom
            && !ui.input(|i| i.key_down(egui::Key::Space))
            && (response.is_pointer_button_down_on()
                || response.clicked_by(egui::PointerButton::Primary)
                || response.drag_stopped_by(egui::PointerButton::Primary))
            && let Some(pos) = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            })
        {
            self.first_point = Some((
                document.id,
                document
                    .view
                    .screen_to_document([pos.x as f64, pos.y as f64], size, center, ppp),
            ));
        }
        #[cfg(test)]
        {
            self.brush_outline.clear();
        }
        #[cfg(test)]
        {
            self.cursor_rect = None;
        }
        let relative_cursor = paint
            .drawing
            .pen_cursor(paint.view)
            .filter(|p| rect.contains(*p));
        if input_active
            && (relative_cursor.is_some()
                || (ui.input(|i| i.pointer.latest_pos().is_some())
                    && (response.hovered() || response.dragged())))
            && let Some(pos) = relative_cursor.or_else(|| {
                response
                    .hover_pos()
                    .or_else(|| response.interact_pointer_pos())
            })
        {
            let active_style = stroke.map(|s| s.style());
            let cursor_tool = active_style.map_or(document.tool, |s| s.tip);
            let mut cursor_shape = stroke.map_or(self.brush_shape, |s| s.shape());
            if stroke.is_none() {
                let n = drawer_core::assets::BrushTip::ALL
                    .iter()
                    .position(|t| *t == cursor_tool)
                    .unwrap();
                cursor_shape.size = paint.drawing.patterns[n]
                    .behavior
                    .document_size(cursor_shape.size, document.view.zoom)
                    .unwrap_or(cursor_shape.size);
            }
            let n = drawer_core::assets::BrushTip::ALL
                .iter()
                .position(|t| *t == cursor_tool)
                .unwrap();
            let appearance = self.cursors.tools[n];
            let strength = if self.working {
                self.filters[n].position(self.smoothing[n])
            } else {
                0.0
            };
            if let Some(stroke) = stroke {
                self.cursor_trace = stroke.trace();
            } else {
                if ui.input(|i| i.pointer.primary_pressed() || i.pointer.primary_released()) {
                    self.cursor_trace.clear();
                }
                let points = if relative_cursor.is_some() {
                    vec![]
                } else {
                    ui.input(|i| {
                        i.events
                            .iter()
                            .filter_map(|e| match e {
                                egui::Event::PointerMoved(p) => Some(*p),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    })
                };
                let moved = !points.is_empty();
                for p in points {
                    let point = document.view.screen_to_document(
                        [p.x as f64, p.y as f64],
                        size,
                        center,
                        ppp,
                    );
                    self.cursor_trace.receive(point, strength);
                }
                let point = document.view.screen_to_document(
                    [pos.x as f64, pos.y as f64],
                    size,
                    center,
                    ppp,
                );
                if self.cursor_trace.raw != Some(point) {
                    self.cursor_trace.receive(point, strength);
                } else if !moved {
                    self.cursor_trace.prediction = None;
                }
            }
            if !self.working {
                self.cursor_trace.prediction = None;
            }
            if self.behavior.brush_cursor {
                if appearance.style != drawer_core::brush_cursor::Style::System {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                }
                let outline = crate::cursor_settings::paint(
                    &painter,
                    &self.cursor_trace,
                    appearance,
                    cursor_shape,
                    crate::cursor_settings::Projection {
                        square: stroke
                            .map_or(self.profiles[n].square(cursor_tool), |s| s.mask_is_square()),
                        view: document.view,
                        size,
                        center,
                        ppp,
                    },
                    self.tool_icons[n].id(),
                );
                #[cfg(test)]
                {
                    self.brush_outline = outline;
                    self.cursor_rect = (appearance.style
                        != drawer_core::brush_cursor::Style::System)
                        .then_some(egui::Rect::from_center_size(pos, egui::vec2(14.0, 14.0)));
                }
                #[cfg(not(test))]
                let _ = outline;
            }
        } else {
            self.cursor_trace.clear();
        }
        document.view != before
    }
    fn update_image(
        &mut self,
        ui: &egui::Ui,
        document: &Document,
        stroke: Option<&drawer_core::painting::Stroke>,
    ) {
        if self.image.as_ref().map(|(active, _)| *active)
            != Some((
                document.id,
                document.revision,
                document.current_frame,
                stroke.map_or((0, 0), |s| s.preview_key()),
            ))
        {
            let reduction = document.width.max(document.height).div_ceil(2048).max(1);
            let width = document.width.div_ceil(reduction);
            let height = document.height.div_ceil(reduction);
            let mut pixels = Vec::with_capacity((width * height) as usize);
            let renderer = drawer_core::render::Renderer::preview(document, stroke);
            for y in 0..height {
                for x in 0..width {
                    let pixel = renderer
                        .pixel([(x * reduction) as f64 + 0.5, (y * reduction) as f64 + 0.5]);
                    let original = pixel;
                    let pixel = self.displayed_color(pixel);
                    if x == 0 && y == 0 {
                        self.preview_sample = Some((original, pixel));
                    }
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
            self.image = Some((
                (
                    document.id,
                    document.revision,
                    document.current_frame,
                    stroke.map_or((0, 0), |s| s.preview_key()),
                ),
                texture,
            ));
        }
    }
    fn paint_paper(
        &mut self,
        ui: &egui::Ui,
        document: &Document,
        view: drawer_core::view::View,
        rect: egui::Rect,
        stroke: Option<&drawer_core::painting::Stroke>,
    ) {
        let size = [document.width as f64, document.height as f64];
        let center = [rect.center().x as f64, rect.center().y as f64];
        let ppp = ui.ctx().pixels_per_point() as f64;
        let corners = [
            [0.0, 0.0],
            [size[0], 0.0],
            [size[0], size[1]],
            [0.0, size[1]],
        ]
        .map(|point| {
            let pos = view.document_to_screen(point, size, center, ppp);
            egui::pos2(pos[0] as f32, pos[1] as f32)
        });
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(32));
        let mut mesh = egui::Mesh::with_texture(self.checker.id());
        let uv_max = egui::pos2(document.width as f32 / 32.0, document.height as f32 / 32.0);
        let uv = [
            egui::Pos2::ZERO,
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
        {
            let scale = (2048.0 / (rect.width().max(rect.height()) as f64 * ppp)).min(1.0);
            let dimensions = [
                (rect.width() as f64 * ppp * scale).ceil().max(1.0) as usize,
                (rect.height() as f64 * ppp * scale).ceil().max(1.0) as usize,
            ];
            let key = ScreenKey {
                id: document.id,
                revision: document.revision,
                frame: document.current_frame,
                view,
                size: dimensions,
                ppp,
                filter: self.filter,
                stroke: stroke.map_or((0, 0), |s| s.preview_key()),
            };
            if self.screen_image.as_ref().map(|(k, _)| *k) != Some(key) {
                let renderer = drawer_core::render::Renderer::preview(document, stroke);
                let mut pixels = Vec::with_capacity(dimensions[0] * dimensions[1]);
                let origin = view.screen_to_document(
                    [rect.min.x as f64, rect.min.y as f64],
                    size,
                    center,
                    ppp,
                );
                let horizontal = view.screen_to_document(
                    [
                        rect.min.x as f64 + rect.width() as f64 / dimensions[0] as f64,
                        rect.min.y as f64,
                    ],
                    size,
                    center,
                    ppp,
                );
                let vertical = view.screen_to_document(
                    [
                        rect.min.x as f64,
                        rect.min.y as f64 + rect.height() as f64 / dimensions[1] as f64,
                    ],
                    size,
                    center,
                    ppp,
                );
                let map = |x: f64, y: f64| {
                    [
                        origin[0] - 0.5
                            + x * (horizontal[0] - origin[0])
                            + y * (vertical[0] - origin[0]),
                        origin[1] - 0.5
                            + x * (horizontal[1] - origin[1])
                            + y * (vertical[1] - origin[1]),
                    ]
                };
                let bounds = corners
                    .iter()
                    .fold(egui::Rect::NOTHING, |bounds, p| {
                        bounds.union(egui::Rect::from_min_max(*p, *p))
                    })
                    .intersect(rect);
                let start = [
                    ((bounds.min.x - rect.min.x).max(0.0) / rect.width() * dimensions[0] as f32)
                        .floor() as usize,
                    ((bounds.min.y - rect.min.y).max(0.0) / rect.height() * dimensions[1] as f32)
                        .floor() as usize,
                ];
                let end = [
                    ((bounds.max.x - rect.min.x).max(0.0) / rect.width() * dimensions[0] as f32)
                        .ceil() as usize,
                    ((bounds.max.y - rect.min.y).max(0.0) / rect.height() * dimensions[1] as f32)
                        .ceil() as usize,
                ];
                pixels.resize(dimensions[0] * dimensions[1], egui::Color32::TRANSPARENT);
                for y in start[1].min(dimensions[1])..end[1].min(dimensions[1]) {
                    for x in start[0].min(dimensions[0])..end[0].min(dimensions[0]) {
                        let point = map(x as f64 + 0.5, y as f64 + 0.5);
                        let footprint = [
                            map(x as f64, y as f64),
                            map(x as f64 + 1.0, y as f64),
                            map(x as f64 + 1.0, y as f64 + 1.0),
                            map(x as f64, y as f64 + 1.0),
                        ];
                        let outside = footprint.iter().all(|p| p[0] < -0.5)
                            || footprint.iter().all(|p| p[1] < -0.5)
                            || footprint.iter().all(|p| p[0] >= size[0] - 0.5)
                            || footprint.iter().all(|p| p[1] >= size[1] - 0.5);
                        let pixel = if outside {
                            [0; 4]
                        } else {
                            drawer_core::interpolation::sample(
                                point,
                                footprint,
                                self.filter,
                                |x, y| renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                            )
                        };
                        let original = pixel;
                        let pixel = self.displayed_color(pixel);
                        if original[3] > 0 {
                            self.preview_sample = Some((original, pixel));
                        }
                        pixels[y * dimensions[0] + x] = egui::Color32::from_rgba_unmultiplied(
                            pixel[0], pixel[1], pixel[2], pixel[3],
                        );
                    }
                }
                self.screen_image = Some((
                    key,
                    ui.ctx().load_texture(
                        "interpolated-canvas-view",
                        egui::ColorImage::new(dimensions, pixels),
                        egui::TextureOptions::NEAREST,
                    ),
                ));
            }
            painter.image(
                self.screen_image.as_ref().unwrap().1.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    pub fn show_only(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        view: &mut drawer_core::view::View,
        interactive: bool,
    ) {
        self.proof_notice(ui);
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(egui::vec2(1.0, 1.0)),
            egui::Sense::drag(),
        );
        #[cfg(test)]
        {
            self.viewport = Some(rect);
        }
        let size = [document.width as f64, document.height as f64];
        let center = [rect.center().x as f64, rect.center().y as f64];
        let ppp = ui.ctx().pixels_per_point() as f64;
        self.move_delta = None;
        if response.dragged_by(egui::PointerButton::Primary)
            && !ui.input(|i| i.key_down(egui::Key::Space))
        {
            let delta = response.drag_delta();
            let a = view.screen_to_document(center, size, center, ppp);
            let b = view.screen_to_document(
                [center[0] + delta.x as f64, center[1] + delta.y as f64],
                size,
                center,
                ppp,
            );
            self.move_delta = Some([b[0] - a[0], b[1] - a[1]]);
        }
        if interactive {
            if response.dragged_by(egui::PointerButton::Middle)
                || (response.dragged_by(egui::PointerButton::Primary)
                    && ui.input(|i| i.key_down(egui::Key::Space)))
            {
                let delta = response.drag_delta();
                view.pan_by([delta.x as f64, delta.y as f64]);
            }
            if response.hovered() {
                let wheel = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta).y);
                if wheel != 0.0
                    && let Some(pos) = response.hover_pos()
                {
                    view.zoom_about(
                        (wheel as f64 * 0.002).exp(),
                        [pos.x as f64, pos.y as f64],
                        size,
                        center,
                        ppp,
                    );
                }
            }
        }
        self.update_image(ui, document, None);
        self.paint_paper(ui, document, *view, rect, None);
    }
    #[cfg(test)]
    pub fn rendered_stroke(&self) -> (u64, u64) {
        self.image.as_ref().map_or((0, 0), |(key, _)| key.3)
    }
    #[cfg(test)]
    pub fn rendered_revision(&self) -> Option<(u64, u64)> {
        self.image.as_ref().map(|(key, _)| (key.0, key.1))
    }
    pub fn navigator_in_dock(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        ppp: f64,
        stroke: Option<&drawer_core::painting::Stroke>,
    ) {
        self.update_image(ui, document, stroke);
        let rect = self
            .last_viewport
            .unwrap_or_else(|| ui.ctx().content_rect());
        self.show_navigator(ui, document, rect, ppp);
    }
    fn show_navigator(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        viewport: egui::Rect,
        ppp: f64,
    ) {
        if !self.navigator.floating {
            ui.heading("Navigator");
        }
        ui.label(selection_status(document));
        let dimensions = if self.navigator.floating {
            let available = ui.available_size();
            [
                available
                    .x
                    .clamp(self.navigator.min_width, self.navigator.max_width),
                available.y.clamp(80.0, 640.0),
            ]
        } else {
            [
                self.navigator.size[0].min(ui.available_width()).max(1.0),
                self.navigator.size[1].min(ui.available_height()).max(1.0),
            ]
        };
        if self.navigator.floating {
            self.navigator.size = dimensions;
        }
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(dimensions[0], dimensions[1]),
            egui::Sense::click_and_drag(),
        );
        #[cfg(test)]
        {
            self.navigator_rect = Some(rect);
        }
        let size = [document.width as f64, document.height as f64];
        let scale = (rect.width() as f64 / size[0]).min(rect.height() as f64 / size[1]);
        let paper = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2((size[0] * scale) as f32, (size[1] * scale) as f32),
        );
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            response.request_focus();
            let point = [
                ((pos.x - paper.min.x) as f64 / scale).clamp(0.0, size[0]),
                ((pos.y - paper.min.y) as f64 / scale).clamp(0.0, size[1]),
            ];
            document.view.center_on(
                point,
                size,
                [viewport.center().x as f64, viewport.center().y as f64],
                ppp,
            );
            ui.ctx().request_repaint();
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(32));
        painter.rect_filled(paper, 0.0, egui::Color32::from_gray(196));
        painter.image(
            self.image.as_ref().unwrap().1.id(),
            paper,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        self.paint_selection(ui, document, paper, scale);
        let polygon = drawer_core::navigator::visible_polygon(
            document.view,
            size,
            [
                [viewport.min.x as f64, viewport.min.y as f64],
                [viewport.max.x as f64, viewport.max.y as f64],
            ],
            ppp,
        );
        let points: Vec<_> = drawer_core::navigator::clipped_polygon(polygon, size)
            .into_iter()
            .map(|point| {
                egui::pos2(
                    paper.min.x + (point[0] * scale) as f32,
                    paper.min.y + (point[1] * scale) as f32,
                )
            })
            .collect();
        let outline = self
            .navigator
            .outline_visible(response.hovered(), response.has_focus());
        #[cfg(test)]
        {
            self.navigator_outline = outline;
        }
        if points.len() >= 3 && outline {
            painter.add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(40, 230, 185)),
            ));
        } else if points.len() < 3 {
            ui.label("The view is outside the image");
        }
        ui.label("Click or drag to move the view");
    }
    fn paint_selection(
        &self,
        ui: &mut egui::Ui,
        document: &Document,
        paper: egui::Rect,
        scale: f64,
    ) {
        let map = |point| {
            let p = document.scene.selection_point(point);
            egui::pos2(
                paper.min.x + (p[0] * scale) as f32,
                paper.min.y + (p[1] * scale) as f32,
            )
        };
        let painter = ui.painter_at(paper);
        let quad = |rect: drawer_core::scene::Rect| {
            [
                rect.min,
                [rect.max[0], rect.min[1]],
                rect.max,
                [rect.min[0], rect.max[1]],
            ]
            .map(map)
            .to_vec()
        };
        let edges = if let Some(mask) = &document.scene.selection_mask {
            for run in &mask.runs {
                let rect = drawer_core::scene::Rect {
                    min: [run.x as f64, run.y as f64],
                    max: [run.x as f64 + run.length as f64, run.y as f64 + 1.0],
                };
                painter.add(egui::Shape::convex_polygon(
                    quad(rect),
                    egui::Color32::from_rgba_unmultiplied(
                        90,
                        130,
                        255,
                        (run.alpha as f64 * 0.35).round() as u8,
                    ),
                    egui::Stroke::NONE,
                ));
            }
            if self.navigator.selection_outline {
                mask.boundaries()
            } else {
                Vec::new()
            }
        } else if let Some(rect) = document.scene.selection {
            painter.add(egui::Shape::convex_polygon(
                quad(rect),
                egui::Color32::from_rgba_unmultiplied(90, 130, 255, 90),
                egui::Stroke::NONE,
            ));
            if self.navigator.selection_outline {
                let p = [
                    rect.min,
                    [rect.max[0], rect.min[1]],
                    rect.max,
                    [rect.min[0], rect.max[1]],
                ];
                (0..4).map(|n| [p[n], p[(n + 1) % 4]]).collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        for line in edges {
            dashed(&painter, line.map(map), paper);
        }
    }
    fn paint_aids(
        &self,
        ui: &egui::Ui,
        document: &Document,
        rect: egui::Rect,
        preferences: drawer_core::aids::AidPreferences,
        ppp: f64,
    ) {
        use drawer_core::aids;
        let painter = ui.painter_at(rect);
        let size = [document.width as f64, document.height as f64];
        let viewport = [
            [rect.min.x as f64, rect.min.y as f64],
            [rect.max.x as f64, rect.max.y as f64],
        ];
        let center = [rect.center().x as f64, rect.center().y as f64];
        let segment = |line: [[f64; 2]; 2], color: egui::Color32| {
            if let Some(line) = aids::clip_line(line, viewport) {
                painter.line_segment(
                    line.map(|p| egui::pos2(p[0] as f32, p[1] as f32)),
                    egui::Stroke::new(1.0, color),
                );
            }
        };
        if preferences.grid {
            for line in aids::grid_lines(
                document.view,
                document.scene.transform,
                document.scene.grid,
                size,
                viewport,
                ppp,
            ) {
                segment(line, egui::Color32::from_white_alpha(48));
            }
        }
        if preferences.guides {
            for line in &document.scene.guides {
                segment(
                    line.map(|point| {
                        document.view.document_to_screen(
                            document.scene.transform.map(point),
                            size,
                            center,
                            ppp,
                        )
                    }),
                    egui::Color32::from_rgb(90, 180, 255),
                );
            }
        }
        if preferences.rulers {
            let background = egui::Color32::from_gray(45);
            painter.rect_filled(
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + 20.0)),
                0.0,
                background,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 38.0, rect.max.y)),
                0.0,
                background,
            );
            for horizontal in [true, false] {
                let ruler = aids::ruler(
                    document.view,
                    size,
                    viewport,
                    ppp,
                    document.dpi,
                    preferences.unit,
                    horizontal,
                );
                for tick in ruler.ticks {
                    let pos = if horizontal {
                        egui::pos2(tick.screen as f32, rect.min.y + 4.0)
                    } else {
                        egui::pos2(rect.min.x + 3.0, tick.screen as f32)
                    };
                    painter.text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        format!("{:.3}", tick.value),
                        egui::FontId::proportional(9.0),
                        egui::Color32::LIGHT_GRAY,
                    );
                    let line = if horizontal {
                        [
                            egui::pos2(tick.screen as f32, rect.min.y + 16.0),
                            egui::pos2(tick.screen as f32, rect.min.y + 20.0),
                        ]
                    } else {
                        [
                            egui::pos2(rect.min.x + 32.0, tick.screen as f32),
                            egui::pos2(rect.min.x + 38.0, tick.screen as f32),
                        ]
                    };
                    painter.line_segment(line, egui::Stroke::new(1.0, egui::Color32::LIGHT_GRAY));
                }
                let label = format!("{} {}", ["X", "Y"][ruler.axis], preferences.unit.label());
                let pos = if horizontal {
                    egui::pos2(rect.max.x - 48.0, rect.min.y + 3.0)
                } else {
                    egui::pos2(rect.min.x + 2.0, rect.max.y - 14.0)
                };
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(10.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }
}

pub(super) fn selection_status(document: &Document) -> &'static str {
    let empty = if let Some(mask) = &document.scene.selection_mask {
        Some(mask.runs.is_empty())
    } else {
        document
            .scene
            .selection
            .map(|rect| rect.min[0] >= rect.max[0] || rect.min[1] >= rect.max[1])
    };
    match empty {
        None => "No selection: all pixels allowed",
        Some(true) => "Empty selection: all pixels blocked",
        Some(false) => "Selection coverage",
    }
}
fn dashed(painter: &egui::Painter, line: [egui::Pos2; 2], clip: egui::Rect) {
    let [a, b] = line;
    let delta = b - a;
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for (origin, d, min, max) in [
        (a.x, delta.x, clip.min.x, clip.max.x),
        (a.y, delta.y, clip.min.y, clip.max.y),
    ] {
        if d.abs() < 1e-8 {
            if origin < min || origin > max {
                return;
            }
        } else {
            let t1 = (min - origin) / d;
            let t2 = (max - origin) / d;
            lo = lo.max(t1.min(t2));
            hi = hi.min(t1.max(t2));
        }
    }
    if lo > hi {
        return;
    }
    let a = a + delta * lo;
    let b = line[0] + delta * hi;
    let delta = b - a;
    let length = delta.length();
    if length < 1e-6 {
        return;
    }
    let direction = delta / length;
    for n in 0..(length / 7.0).ceil() as usize {
        let begin = n as f32 * 7.0;
        let end = (begin + 4.0).min(length);
        painter.line_segment(
            [a + direction * begin, a + direction * end],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(155, 185, 255)),
        );
    }
}
