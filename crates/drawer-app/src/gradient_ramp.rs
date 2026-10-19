use super::*;
use drawer_core::gradient_ramp::{Brightness, Curve, History, Library, Ramp, Space, Stop};
#[derive(Default)]
pub(super) struct Panel {
    history: History,
    name: String,
    selected: Option<u64>,
    path: String,
    pub apply: Option<egui::Rect>,
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    settings: &mut drawer_core::gradient::Settings,
    panel: &mut Panel,
    library: &mut Library,
    status: &mut Option<String>,
) {
    ui.collapsing("Color stops, mixing and local presets", |ui| {
        body(ui, settings, panel, library, status)
    });
}
fn body(
    ui: &mut egui::Ui,
    settings: &mut drawer_core::gradient::Settings,
    panel: &mut Panel,
    library: &mut Library,
    status: &mut Option<String>,
) {
    let mut enabled = settings.ramp.is_some();
    if ui
        .checkbox(&mut enabled, "Edit multiple color/alpha stops")
        .changed()
    {
        settings.ramp = enabled.then(|| Ramp::two(settings.first, settings.last));
    }
    if let Some(ramp) = &mut settings.ramp {
        egui::ComboBox::from_id_salt("gradient-mix-space")
            .selected_text(ramp.space.label())
            .show_ui(ui, |ui| {
                for space in Space::ALL {
                    ui.selectable_value(&mut ramp.space, space, space.label());
                }
            });
        egui::ComboBox::from_id_salt("gradient-brightness")
            .selected_text(format!("Brightness {:?}", ramp.brightness))
            .show_ui(ui, |ui| {
                for b in Brightness::ALL {
                    ui.selectable_value(&mut ramp.brightness, b, format!("{b:?}"));
                }
            });
        let mut remove = None;
        for (index, stop) in ramp.stops.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Stop {}", index + 1));
                    ui.add(
                        egui::DragValue::new(&mut stop.position)
                            .range(0.0..=1.0)
                            .speed(0.005),
                    );
                    super::edit::rgba(ui, &mut stop.color);
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut stop.midpoint, 0.01..=0.99)
                            .text("Next segment midpoint"),
                    );
                    egui::ComboBox::from_id_salt("stop-curve")
                        .selected_text(format!("{:?}", stop.curve))
                        .show_ui(ui, |ui| {
                            for c in Curve::ALL {
                                ui.selectable_value(&mut stop.curve, c, format!("{c:?}"));
                            }
                        });
                });
            });
        }
        if let Some(index) = remove {
            if ramp.stops.len() > 2 {
                ramp.stops.remove(index);
            } else {
                *status = Some("Keep at least two gradient stops".into());
            }
        }
        if ui.button("Add stop at midpoint").clicked() {
            if ramp.stops.len() < 32 {
                let c = ramp.sample(0.5).map(|v| v.round() as u8);
                ramp.stops.push(Stop::new(0.5, c));
            } else {
                *status = Some("A gradient supports at most 32 stops".into());
            }
        }
        ramp.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        ui.label("Equal-position stops form a hard edge; the last stop at that position wins. Curves and midpoints affect the following segment. Perceptual A/B use independent Oklab/CIELAB mixing.");
    }
    if settings.valid() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(256.0, 20.0), egui::Sense::hover());
        let mut preview = settings.clone();
        preview.start = [0.0, 0.0];
        preview.end = [256.0, 0.0];
        preview.shape = drawer_core::gradient::Shape::StraightLine;
        preview.repeat = drawer_core::gradient::Repeat::DoNotRepeat;
        for x in 0..256 {
            let p = preview.color([x as f64 + 0.5, 0.0], None);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    rect.min + egui::vec2(x as f32, 0.0),
                    egui::vec2(1.0, 20.0),
                ),
                0.0,
                egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]),
            );
        }
    }
    let before = library.clone();
    let mut undo = false;
    let mut redo = false;
    egui::ComboBox::from_id_salt("local-gradient-preset")
        .selected_text(
            library
                .presets
                .iter()
                .find(|p| Some(p.id) == panel.selected)
                .map_or("Select local preset", |p| p.name.as_str()),
        )
        .show_ui(ui, |ui| {
            for p in &library.presets {
                if ui
                    .selectable_value(&mut panel.selected, Some(p.id), &p.name)
                    .changed()
                {
                    panel.name = p.name.clone();
                }
            }
        });
    ui.text_edit_singleline(&mut panel.name);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Load preset into draft").clicked() {
            if let Some(p) = library
                .presets
                .iter()
                .find(|p| Some(p.id) == panel.selected)
            {
                *settings = p.settings.clone();
            } else {
                *status = Some("Select a gradient preset".into());
            }
        }
        let save = ui.button("Save as new preset");
        panel.apply = Some(save.rect);
        if save.clicked() {
            match library.save(None, &panel.name, settings.clone()) {
                Ok(id) => panel.selected = Some(id),
                Err(e) => *status = Some(e),
            }
        }
        if ui.button("Update/rename selected preset").clicked() {
            if let Some(id) = panel.selected {
                if let Err(e) = library.save(Some(id), &panel.name, settings.clone()) {
                    *status = Some(e)
                }
            } else {
                *status = Some("Select a gradient preset".into())
            }
        }
        if ui.button("Delete preset").clicked()
            && let Some(id) = panel.selected
        {
            match library.remove(id) {
                Ok(()) => panel.selected = None,
                Err(e) => *status = Some(e),
            }
        }
        undo = ui.button("Undo presets").clicked();
        redo = ui.button("Redo presets").clicked();
        if ui.button("Reset local presets").clicked() {
            *library = Library::default();
            panel.selected = None;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Local JSON path");
        ui.text_edit_singleline(&mut panel.path);
    });
    ui.horizontal(|ui| {
        if ui.button("Export presets").clicked()
            && let Err(e) = library.export(std::path::Path::new(&panel.path))
        {
            *status = Some(e)
        }
        if ui.button("Import with undo backup").clicked() {
            match Library::import(std::path::Path::new(&panel.path)) {
                Ok(next) => {
                    *library = next;
                    panel.selected = None;
                }
                Err(e) => *status = Some(e),
            }
        }
    });
    panel.history.record(before, library);
    if undo {
        panel.history.undo(library);
    }
    if redo {
        panel.history.redo(library);
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::gradient_ramp_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 12,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for space in Space::ALL {
        app.execute_command(&ctx, "gradient-tool");
        let draft = app
            .gradient
            .draft
            .as_mut()
            .ok_or("Gradient draft missing")?;
        let mut ramp = Ramp::two([255, 0, 0, 255], [0, 0, 255, 64]);
        ramp.space = space;
        ramp.brightness = Brightness::Medium;
        ramp.stops.insert(
            1,
            Stop {
                position: 0.4,
                color: [0, 200, 40, 128],
                midpoint: 0.3,
                curve: Curve::Smooth,
            },
        );
        draft.settings.ramp = Some(ramp);
        let before = app.documents[0].snapshot();
        let expected = drawer_core::gradient::prepare(
            &app.documents[0],
            draft.layer,
            draft.settings.clone(),
            true,
            draft.reference,
        )?;
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let rect = app.gradient.apply.ok_or("Gradient apply missing")?;
        for pressed in [true, false] {
            step(
                &mut app,
                &mut time,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        if app.gradient.draft.is_some() || app.documents[0].snapshot() != expected {
            return Err(format!("Ramp production apply failed: {:?}", app.status));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Ramp production Undo".into());
        }
    }
    // Exercise the same preset controls through real button events, without page changes.
    let mut panel = Panel {
        name: "Local ramp".into(),
        ..Default::default()
    };
    let mut library = Library::default();
    let mut settings = drawer_core::gradient::Settings {
        ramp: Some(Ramp::two([255, 0, 0, 255], [0, 0, 255, 0])),
        ..Default::default()
    };
    let mut status = None;
    let before = app.documents[0].snapshot();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1000.0),
                )),
                ..Default::default()
            },
            |ui| body(ui, &mut settings, &mut panel, &mut library, &mut status),
        );
        output.textures_delta.clear();
    }
    let rect = panel.apply.ok_or("Preset button missing")?;
    for pressed in [true, false] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1000.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| body(ui, &mut settings, &mut panel, &mut library, &mut status),
        );
        output.textures_delta.clear();
    }
    if library.presets.len() != 1 || app.documents[0].snapshot() != before {
        return Err("Local preset operation affected page".into());
    }
    if !panel.history.undo(&mut library)
        || !library.presets.is_empty()
        || !panel.history.redo(&mut library)
    {
        return Err("Local preset history".into());
    }
    let mut snapshot = drawer_core::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.gradient_presets = library;
    let path = root.join("gradient-workspace.json");
    snapshot.export(&path).map_err(|e| e.to_string())?;
    if drawer_core::workspaces::Snapshot::read(&path)
        .map_err(|e| e.to_string())?
        .gradient_presets
        != snapshot.gradient_presets
    {
        return Err("Gradient workspace reopen".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mixing_apply_and_local_preset_settings() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
