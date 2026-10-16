use super::*;
use drawer_core::eyedropper::Source;
impl Drawer {
    pub(super) fn eyedropper_controls(&mut self, ui: &mut egui::Ui) {
        let state = &mut self.workspaces.store.current.eyedropper;
        egui::ComboBox::from_id_salt("eyedropper-source")
            .selected_text(state.settings.source.label())
            .show_ui(ui, |ui| {
                for source in Source::ALL {
                    ui.selectable_value(&mut state.settings.source, source, source.label());
                }
            });
        egui::ComboBox::from_id_salt("eyedropper-kernel")
            .selected_text(if state.settings.radius == 0 {
                "Single pixel".into()
            } else {
                format!(
                    "Average {}×{}",
                    state.settings.radius * 2 + 1,
                    state.settings.radius * 2 + 1
                )
            })
            .show_ui(ui, |ui| {
                for (radius, label) in [
                    (0, "Single pixel"),
                    (1, "Average 3×3"),
                    (2, "Average 5×5"),
                    (4, "Average 9×9"),
                ] {
                    ui.selectable_value(&mut state.settings.radius, radius, label);
                }
            });
        ui.label("Layer sampling reads stored RGBA before effects and selection. RGB-only keeps drawing alpha; RGBA includes alpha. A zero-alpha pixel with no stored RGB leaves the current color unchanged.");
        let current = &mut self.references.session.workspace.color;
        state.observe(*current);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(state.can_undo(), egui::Button::new("Undo color choice"))
                .clicked()
                && let Some(c) = state.undo(*current)
            {
                *current = c;
            }
            if ui
                .add_enabled(state.can_redo(), egui::Button::new("Redo color choice"))
                .clicked()
                && let Some(c) = state.redo(*current)
            {
                *current = c;
            }
        });
        ui.horizontal_wrapped(|ui| {
            for color in state.recent.clone() {
                if ui
                    .add(
                        egui::Button::new(format!(
                            "#{:02X}{:02X}{:02X}/{:02X}",
                            color[0], color[1], color[2], color[3]
                        ))
                        .fill(egui::Color32::from_rgba_unmultiplied(
                            color[0], color[1], color[2], color[3],
                        )),
                    )
                    .clicked()
                {
                    *current = color;
                    state.observe(color);
                }
            }
        });
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use super::input_replay::frame;
    use drawer_core::{document::RasterLayer, eyedropper::Settings, pointer_input::Phase};
    drawer_core::tip_profile::verify_pixel(root)?;
    let ctx = egui::Context::default();
    let user = root.join("eyedropper-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 4,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for y in 0..4 {
        for x in 0..3 {
            app.documents[0].layers[0].pixels[y * 8 + x] = [20, 80, 160, [0, 128, 255][x]];
        }
    }
    app.documents[0].layers.push(RasterLayer {
        locked: true,
        name: "Composite overlay".into(),
        pixels: vec![[160, 30, 40, 128]; 32],
        placement: None,
    });
    app.documents[0].scene.selection_mask = Some(Default::default());
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.1, vec![]);
    app.execute_command(&ctx, "Eyedropper");
    let original = app.documents[0].snapshot();
    let dirty = app.documents[0].dirty;
    let mut time = 0.2;
    for source in Source::ALL {
        for (x, alpha) in [(0, 0), (1, 128), (2, 255)] {
            let expected = match source {
                Source::LayerRgb => [20, 80, 160, 47],
                Source::LayerRgba => [20, 80, 160, alpha],
                Source::Composite => app.documents[0].composite_pixel(x, 1),
            };
            app.workspaces.store.current.eyedropper.settings = Settings { source, radius: 0 };
            app.references.session.workspace.color = [1, 2, 3, 47];
            frame(&ctx, &mut app, time, vec![]);
            time += 0.1;
            let events = vec![
                super::precision::pointer(&ctx, &app, [x as f64 + 0.5, 1.5], Phase::Down),
                super::precision::pointer(&ctx, &app, [x as f64 + 0.5, 1.5], Phase::Up),
            ];
            app.drawing.virtual_events(events, false);
            frame(&ctx, &mut app, time, vec![]);
            time += 0.1;
            frame(&ctx, &mut app, time, vec![]);
            time += 0.1;
            if app.references.session.workspace.color != expected
                || app.documents[0].snapshot() != original
                || app.documents[0].dirty != dirty
                || app.documents[0].history_counts() != (0, 0)
            {
                return Err(format!(
                    "Actual eyedropper {source:?}/{alpha} sampling changed document or returned wrong RGBA"
                ));
            }
        }
    }
    app.workspaces.store.current.eyedropper.settings = Settings {
        source: Source::LayerRgba,
        radius: 0,
    };
    let color = app.references.session.workspace.color;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    let events = vec![
        super::precision::pointer(&ctx, &app, [3.5, 1.5], Phase::Down),
        super::precision::pointer(&ctx, &app, [3.5, 1.5], Phase::Up),
    ];
    app.drawing.virtual_events(events, false);
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    frame(&ctx, &mut app, time, vec![]);
    if app.references.session.workspace.color != color {
        return Err("Empty transparent sample replaced current color".into());
    }
    let previous = app
        .workspaces
        .store
        .current
        .eyedropper
        .undo(color)
        .ok_or("Color history missing")?;
    app.references.session.workspace.color = previous;
    let redone = app
        .workspaces
        .store
        .current
        .eyedropper
        .redo(previous)
        .ok_or("Color redo missing")?;
    app.references.session.workspace.color = redone;
    if color != redone || app.documents[0].snapshot() != original {
        return Err("Color history changed document or lost color".into());
    }
    app.workspaces.store.current.eyedropper.settings.radius = 2;
    app.sync_tool_properties();
    app.workspaces.store.current.input.profiles[6].pixel = drawer_core::tip_profile::Pixel {
        pressure: false,
        grid: false,
    };
    let p = drawer_core::brush_presets::Preset::capture(
        &app.capture_workspace(),
        BrushTip::PixelPen,
        "Pixel off",
    )?;
    let restored =
        drawer_core::brush_presets::Preset::decode(&p.encode().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if restored.profile.pixel != p.profile.pixel {
        return Err("Pixel settings lost from preset".into());
    }
    app.persist_workspace()?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().eyedropper != app.capture_workspace().eyedropper
        || restarted.workspaces.store.current.input.profiles[6].pixel != p.profile.pixel
    {
        return Err("Eyedropper mode/colors or pixel settings lost on restart".into());
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicUsize::new(0);
    let mut seq = drawer_core::input_replay::Sequence {
        tool: BrushTip::PixelPen,
        size: 1.0,
        ..Default::default()
    };
    seq.profiles[6].pixel.pressure = false;
    for sample in &mut seq.samples {
        sample.event.pressure = Some(0.0);
    }
    let low = super::input_replay::run(&seq, &cancel, &progress)?;
    for sample in &mut seq.samples {
        sample.event.pressure = Some(1.0);
    }
    let high = super::input_replay::run(&seq, &cancel, &progress)?;
    if !low.record.passed
        || !high.record.passed
        || low.pixels != high.pixels
        || low.pixels.iter().all(|p| p[3] == 0)
    {
        return Err("Pixel pressure bypass failed in actual input".into());
    }
    for (n, sample) in seq.samples.iter_mut().enumerate() {
        sample.event.pressure = Some(if n % 2 == 0 { 0.2 } else { 0.8 });
    }
    let steps = super::input_replay::run(&seq, &cancel, &progress)?;
    if !steps.record.passed || steps.pixels != low.pixels {
        return Err("Stepped pixel pressure was not bypassed".into());
    }
    seq.mouse_pressure[6].enabled = true;
    for sample in &mut seq.samples {
        sample.event.source = drawer_core::pointer_input::Source::MOUSE;
        sample.event.pressure = None;
    }
    let mouse = super::input_replay::run(&seq, &cancel, &progress)?;
    if !mouse.record.passed || mouse.pixels != low.pixels {
        return Err("Mouse virtual pressure changed pressure-off pixel output".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn raw_rgb_rgba_composite_real_canvas_color_history_and_pixel_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
