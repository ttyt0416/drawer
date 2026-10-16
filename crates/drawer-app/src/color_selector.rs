use super::*;
use drawer_core::color_selector::{self as core, Model};
#[derive(Default)]
pub(super) struct Fields {
    source: Option<[u8; 4]>,
    hex: String,
    error: Option<String>,
    wheel: Option<egui::Rect>,
    hex_button: Option<egui::Rect>,
    rgb_rects: [Option<egui::Rect>; 4],
}
impl Drawer {
    pub(super) fn choose_color(&mut self, color: [u8; 4]) {
        self.workspaces.store.current.color_modes.observe(color);
        self.workspaces.store.current.color_modes.transparent = false;
        self.drawing.transparent_mode = false;
        if color != self.references.session.workspace.color {
            self.panels.previous_color = self.references.session.workspace.color;
            self.references.session.workspace.color = color;
            self.workspaces.store.current.eyedropper.observe(color);
        }
        self.workspaces.store.current.color_picker.observe(color);
        self.color_fields.source = Some(color);
        self.color_fields.hex = core::hex(color);
        self.color_fields.error = None;
    }
    pub(super) fn color_controls(&mut self, ui: &mut egui::Ui) {
        self.color_mode_controls(ui);
        let color = self.references.session.workspace.color;
        if self.color_fields.source != Some(color) {
            self.color_fields.source = Some(color);
            self.color_fields.hex = core::hex(color);
            self.color_fields.error = None;
        }
        self.workspaces.store.current.color_picker.observe(color);
        let picker = self.workspaces.store.current.color_picker;
        let hsv = core::hsv(color, picker.hue);
        let side = ui.available_width().clamp(48.0, 180.0);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
        self.color_fields.wheel = Some(rect);
        let center = rect.center();
        let radius = side / 2.0 - 2.0;
        let mut mesh = egui::Mesh::default();
        let gray = [255; 4];
        mesh.colored_vertex(center, egui::Color32::from_rgb(gray[0], gray[1], gray[2]));
        for i in 0..=96 {
            let angle = i as f32 / 96.0 * std::f32::consts::TAU;
            let c = core::from_hsv([i as f64 / 96.0 * 360.0, 1.0, 1.0], 255).unwrap();
            mesh.colored_vertex(
                center + egui::vec2(angle.cos(), angle.sin()) * radius,
                egui::Color32::from_rgb(c[0], c[1], c[2]),
            );
            if i > 0 {
                mesh.add_triangle(0, i, i + 1)
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        let angle = hsv[0].to_radians() as f32;
        let mark = center + egui::vec2(angle.cos(), angle.sin()) * radius * hsv[1] as f32;
        ui.painter()
            .circle_stroke(mark, 3.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
        if (response.clicked() || response.drag_stopped())
            && let Some(p) = response.interact_pointer_pos()
        {
            let delta = (p - center) / radius;
            if response.drag_stopped() || delta.length() <= 1.0 {
                match core::wheel([delta.x as f64, delta.y as f64], hsv[2], color[3]) {
                    Ok((c, h)) => {
                        self.workspaces.store.current.color_picker.hue =
                            if delta.length_sq() > 1e-12 {
                                h
                            } else {
                                picker.hue
                            };
                        self.choose_color(c)
                    }
                    Err(e) => self.color_fields.error = Some(e),
                }
            }
        }
        ui.label("Color wheel: angle = hue · radius = saturation");
        let mut value = core::hsv(
            self.references.session.workspace.color,
            self.workspaces.store.current.color_picker.hue,
        )[2];
        if super::edit::slider(ui, &mut value, 0.0..=1.0, "Value", "color-selector").changed() {
            let mut h = core::hsv(
                self.references.session.workspace.color,
                self.workspaces.store.current.color_picker.hue,
            );
            h[2] = value;
            if let Ok(c) = core::from_hsv(h, self.references.session.workspace.color[3]) {
                self.choose_color(c)
            }
        }

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.workspaces.store.current.color_picker.model,
                Model::Rgb,
                "RGB",
            );
            ui.selectable_value(
                &mut self.workspaces.store.current.color_picker.model,
                Model::Hsv,
                "HSV",
            );
        });
        let current = self.references.session.workspace.color;
        let mut selected = None;
        match self.workspaces.store.current.color_picker.model {
            Model::Rgb => {
                let mut channels = current;
                let mut changed = false;
                for (i, label) in ["R", "G", "B", "Alpha"].into_iter().enumerate() {
                    ui.push_id(("rgb", i), |ui| {
                        let response = super::edit::number(
                            ui,
                            &mut channels[i],
                            0.0..=255.0,
                            1.0,
                            label,
                            "color-selector",
                        );
                        self.color_fields.rgb_rects[i] = Some(response.rect);
                        changed |= super::edit::slider(
                            ui,
                            &mut channels[i],
                            0..=255,
                            label,
                            "color-tables",
                        )
                        .changed();
                        changed |= response.changed();
                    });
                }
                if changed {
                    selected = Some(Ok(channels));
                }
            }
            Model::Hsv => {
                let mut h = core::hsv(current, self.workspaces.store.current.color_picker.hue);
                h[1] *= 100.0;
                h[2] *= 100.0;
                let mut changed = false;
                for (i, label, max) in [
                    (0, "Hue °", 360.0),
                    (1, "Saturation %", 100.0),
                    (2, "Value %", 100.0),
                ] {
                    ui.push_id(("hsv", i), |ui| {
                        changed |= super::edit::number(
                            ui,
                            &mut h[i],
                            0.0..=max,
                            1.0,
                            label,
                            "color-selector",
                        )
                        .changed();
                        changed |=
                            super::edit::slider(ui, &mut h[i], 0.0..=max, label, "color-tables")
                                .changed();
                    });
                }
                let mut alpha = current[3];
                changed |= super::edit::number(
                    ui,
                    &mut alpha,
                    0.0..=255.0,
                    1.0,
                    "Alpha",
                    "color-selector",
                )
                .changed();
                if changed {
                    self.workspaces.store.current.color_picker.hue = h[0].rem_euclid(360.0);
                    selected = Some(core::from_hsv([h[0], h[1] / 100.0, h[2] / 100.0], alpha));
                }
            }
        }
        if let Some(result) = selected {
            match result {
                Ok(c) => self.choose_color(c),
                Err(e) => self.color_fields.error = Some(e),
            }
        }
        ui.horizontal(|ui| {
            super::edit::text(ui, &mut self.color_fields.hex, "color-selector");
            let r = ui.button("Apply HEX");
            self.color_fields.hex_button = Some(r.rect);
            if r.clicked() {
                match core::parse_hex(
                    &self.color_fields.hex,
                    self.references.session.workspace.color[3],
                ) {
                    Ok(c) => {
                        self.choose_color(c);
                        self.color_fields.error = None
                    }
                    Err(e) => self.color_fields.error = Some(e),
                }
            }
        });
        if let Some(error) = &self.color_fields.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.label(core::hex(self.references.session.workspace.color));
        if ui.button("Sample color from canvas").clicked() {
            self.execute_command(ui.ctx(), "Eyedropper");
        }
        if ui.button("Intermediate and similar colors…").clicked() {
            self.workspaces.store.current.related_colors.visible = true;
        }
        self.color_table_controls(ui);
        ui.collapsing("Sampler and used colors", |ui| self.eyedropper_controls(ui));
    }
}

fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, events: Vec<egui::Event>) {
    let mut events = events;
    let releases = events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } => Some(egui::Event::Key {
                key: *key,
                physical_key: *physical_key,
                pressed: false,
                repeat: false,
                modifiers: *modifiers,
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    events.extend(releases);
    let mut result = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 1000.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| app.color_controls(ui),
    );
    result.textures_delta.clear();
}
fn click(p: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(p),
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
fn key(key: egui::Key, ctrl: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl,
            command: ctrl,
            ..Default::default()
        },
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify()?;
    let ctx = egui::Context::default();
    let user = root.join("color-selector-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("color-panel-document.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    app.references.session.workspace.color = [255, 255, 255, 0];
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.1, vec![]);
    let rect = app.color_fields.wheel.ok_or("Color wheel missing")?;
    let point = rect.center() + egui::vec2((rect.width() / 2.0 - 2.0) * 0.75, 0.0);
    frame(&ctx, &mut app, 0.2, click(point, true));
    frame(&ctx, &mut app, 0.3, click(point, false));
    if app.references.session.workspace.color != [255, 64, 64, 0] {
        return Err(format!(
            "Actual wheel alpha/hidden RGB failed: {:?}",
            app.references.session.workspace.color
        ));
    }
    let p = app
        .color_fields
        .hex_button
        .ok_or("HEX button missing")?
        .center();
    app.color_fields.hex = "#12345680".into();
    frame(&ctx, &mut app, 0.4, click(p, true));
    frame(&ctx, &mut app, 0.5, click(p, false));
    if app.references.session.workspace.color != [18, 52, 86, 128] {
        return Err("Actual HEX input did not apply".into());
    }
    frame(&ctx, &mut app, 0.6, vec![]);
    app.color_fields.hex = "#bad!".into();
    let p = app.color_fields.hex_button.unwrap().center();
    frame(&ctx, &mut app, 0.7, click(p, true));
    frame(&ctx, &mut app, 0.8, click(p, false));
    if app.references.session.workspace.color != [18, 52, 86, 128]
        || app.color_fields.error.is_none()
    {
        return Err("Invalid HEX changed the foreground color".into());
    }
    let p = app.color_fields.rgb_rects[0]
        .ok_or("RGB number missing")?
        .center();
    frame(&ctx, &mut app, 0.9, click(p, true));
    frame(&ctx, &mut app, 1.0, click(p, false));
    frame(
        &ctx,
        &mut app,
        1.1,
        vec![
            key(egui::Key::A, true),
            egui::Event::Text("256".into()),
            key(egui::Key::Enter, false),
        ],
    );
    if app.references.session.workspace.color != [18, 52, 86, 128] {
        return Err("Invalid numeric input changed RGBA".into());
    }
    frame(&ctx, &mut app, 1.2, vec![key(egui::Key::Escape, false)]);
    frame(&ctx, &mut app, 1.3, vec![]);
    let p = app.color_fields.rgb_rects[0].unwrap().center();
    frame(&ctx, &mut app, 1.4, click(p, true));
    frame(&ctx, &mut app, 1.5, click(p, false));
    frame(
        &ctx,
        &mut app,
        1.6,
        vec![
            key(egui::Key::A, true),
            egui::Event::Text("64".into()),
            key(egui::Key::Enter, false),
        ],
    );
    if app.references.session.workspace.color != [64, 52, 86, 128] {
        return Err("Valid RGB numeric input did not commit".into());
    }
    for _ in 0..3 {
        app.choose_color([18, 52, 86, 128]);
    }
    let history = &app.workspaces.store.current.eyedropper.recent;
    if history.iter().filter(|c| **c == [18, 52, 86, 128]).count() != 1 {
        return Err("Used-color history duplicated identical RGBA".into());
    }
    app.choose_color([18, 52, 86, 0]);
    if app
        .workspaces
        .store
        .current
        .eyedropper
        .recent
        .iter()
        .filter(|c| c[..3] == [18, 52, 86])
        .count()
        != 2
    {
        return Err("History merged different alpha colors".into());
    }
    let selected = app.references.session.workspace.color;
    app.workspaces.store.current.color_picker.model = Model::Hsv;
    frame(&ctx, &mut app, 1.7, vec![]);
    if app.references.session.workspace.color != selected {
        return Err("Changing the input model requantized RGBA".into());
    }
    app.workspaces.store.current.color_picker.model = Model::Hsv;
    app.workspaces.store.current.color_picker.hue = 217.0;
    app.choose_color([100, 100, 100, 0]);
    app.persist_workspace()?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    let s = restarted.capture_workspace();
    if s.color != [100, 100, 100, 0]
        || s.color_picker.model != Model::Hsv
        || s.color_picker.hue != 217.0
        || s.eyedropper.recent != app.capture_workspace().eyedropper.recent
    {
        return Err("Color/model/gray hue/history restart mismatch".into());
    }
    if app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
        || app.documents[0].dirty
    {
        return Err("Color panel changed the document".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_wheel_hex_invalid_numbers_history_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
