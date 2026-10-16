use super::*;
use drawer_core::clone_stamp::Settings;
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    crate::help::checkbox(
        ui,
        &mut s.enabled,
        "Clone stamp using the selected brush mask",
        "clone-stamp",
    );
    if let Some(point) = &mut s.origin {
        for (n, coordinate) in point.iter_mut().enumerate() {
            ui.push_id(n, |ui| {
                super::edit::number(
                    ui,
                    coordinate,
                    -1e12..=1e12,
                    1.0,
                    if n == 0 {
                        " source X document px"
                    } else {
                        " source Y document px"
                    },
                    "clone-stamp",
                )
            });
        }
        if crate::help::button(ui, "Clear clone source", "clone-stamp").clicked() {
            s.origin = None;
        }
    } else {
        ui.label("Source is not set. Pick it on the canvas using the Clone controls, or initialize coordinates.");
        if crate::help::button(ui, "Initialize clone source at (0,0)", "clone-stamp").clicked() {
            s.origin = Some([0.0; 2]);
        }
    }
    ui.label("Copies the current layer's RGBA from its unchanged start-of-stroke pixels. Each stroke starts at the chosen source; outside the canvas is transparent. Color/pigment supply is replaced by the source pixels.");
}
impl Drawer {
    pub(super) fn clone_controls(&mut self, ui: &mut egui::Ui) {
        let mut enabled = self.workspaces.store.current.input.clone_stamp.enabled;
        if crate::help::checkbox(ui, &mut enabled, "Clone stamp", "clone-stamp").changed() {
            self.finish_stroke();
            self.workspaces.store.current.input.clone_stamp.enabled = enabled;
        }
        if enabled {
            let label = if self.drawing.clone_pick {
                "Cancel picking clone source"
            } else {
                "Pick clone source on next canvas press"
            };
            if ui
                .add_enabled(self.active_tab.is_some(), egui::Button::new(label))
                .clicked()
            {
                let was = self.drawing.clone_pick;
                self.finish_stroke();
                self.drawing.clone_pick = !was;
            }
            if let Some(point) = self.workspaces.store.current.input.clone_stamp.origin {
                ui.label(format!(
                    "Source: {:.2}, {:.2} (current layer)",
                    point[0], point[1]
                ));
            } else {
                ui.colored_label(egui::Color32::YELLOW, "Set a clone source before painting");
            }
        }
    }
}

fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64) {
    super::input_replay::frame(ctx, app, time, vec![]);
}
fn pointer(
    ctx: &egui::Context,
    app: &mut Drawer,
    phase: drawer_core::pointer_input::Phase,
    point: [f64; 2],
    time: f64,
) -> Result<(), String> {
    let rect = app
        .canvas_view
        .viewport_rect()
        .ok_or("Clone canvas unavailable")?;
    let d = &app.documents[0];
    let position = d.view.document_to_screen(
        point,
        [d.width as f64, d.height as f64],
        [rect.center().x as f64, rect.center().y as f64],
        ctx.pixels_per_point() as f64,
    );
    app.drawing.virtual_events(
        vec![drawer_core::pointer_input::Event {
            source: drawer_core::pointer_input::Source::MOUSE,
            phase,
            position,
            pressure: None,
            origin: Default::default(),
            attributes: drawer_core::pointer_input::Attributes {
                time_micros: Some((time * 1e6) as u64),
                ..Default::default()
            },
        }],
        false,
    );
    frame(ctx, app, time);
    Ok(())
}
pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::pointer_input::Phase;
    drawer_core::clone_stamp::verify()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 16,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.set_tool(BrushTip::PixelPen);
    for y in 0..16 {
        for x in 0..64 {
            app.documents[0].layers[0].pixels[y * 64 + x] = [
                x as u8 * 3,
                255 - x as u8 * 3,
                (x ^ y) as u8,
                if x % 7 == 0 { 0 } else { 180 },
            ];
        }
    }
    app.workspaces.store.current.input.clone_stamp = Settings {
        enabled: true,
        origin: None,
    };
    frame(&ctx, &mut app, 0.0);
    frame(&ctx, &mut app, 0.01);
    let before = app.documents[0].snapshot();
    let dirty = app.documents[0].dirty;
    pointer(&ctx, &mut app, Phase::Down, [12.5, 8.5], 0.1)?;
    pointer(&ctx, &mut app, Phase::Up, [12.5, 8.5], 0.11)?;
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != (0, 0) {
        return Err("Missing clone source edited the drawing".into());
    }
    app.drawing.clone_pick = true;
    pointer(&ctx, &mut app, Phase::Down, [4.5, 8.5], 0.2)?;
    pointer(&ctx, &mut app, Phase::Up, [4.5, 8.5], 0.21)?;
    if app.workspaces.store.current.input.clone_stamp.origin != Some([4.5, 8.5])
        || app.documents[0].snapshot() != before
        || app.documents[0].dirty != dirty
    {
        return Err("Origin picker did not persist the document point or changed drawing".into());
    }
    pointer(&ctx, &mut app, Phase::Down, [12.5, 8.5], 0.3)?;
    pointer(&ctx, &mut app, Phase::Move, [52.5, 8.5], 0.31)?;
    pointer(&ctx, &mut app, Phase::Up, [52.5, 8.5], 0.32)?;
    let d = &mut app.documents[0];
    for y in 0..16 {
        for x in 0..64 {
            let n = y * 64 + x;
            let source = if y == 8 && (12..=52).contains(&x) {
                y * 64 + x - 8
            } else {
                n
            };
            if d.layers[0].pixels[n] != before.layers[0].pixels[source] {
                return Err(format!("Production clone read pending ink at {x},{y}"));
            }
        }
    }
    let after = d.snapshot();
    if d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Production clone history diverged".into());
    }
    pointer(&ctx, &mut app, Phase::Down, [25.5, 8.5], 0.4)?;
    super::input_replay::frame(
        &ctx,
        &mut app,
        0.41,
        vec![egui::Event::WindowFocused(false)],
    );
    if app.documents[0].snapshot() != after
        || app.documents[0].history_counts() != (1, 0)
        || app.drawing.pending()
    {
        return Err("Cancelled clone changed original or retained contact".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_picker_missing_origin_overlap_and_cancel() {
        super::smoke().unwrap();
    }
}
