use super::*;
use drawer_core::{pointer_input::Phase, precision::Target, scene::Rect};
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn event(
    ctx: &egui::Context,
    app: &Drawer,
    p: [f64; 2],
    phase: Phase,
    pen: bool,
) -> drawer_core::pointer_input::Event {
    let mut e = super::precision::pointer(ctx, app, p, phase);
    if pen {
        e.source = drawer_core::pointer_input::Source {
            kind: drawer_core::pointer_input::Kind::Pen,
            device: 71,
            contact: 9,
        };
        e.pressure = Some(0.5);
    }
    e
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    for (command, target) in [
        ("move-selection-boundary", Target::Boundary),
        ("move-selected-pixels", Target::Pixels),
    ] {
        for pen in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            let mut time = 0.0;
            app.create_canvas(CanvasSpec {
                width: 16,
                height: 12,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            app.documents[0].layers[0].pixels[2 * 16 + 2] = [40, 50, 60, 128];
            app.documents[0].scene.selection = Some(Rect {
                min: [2.0; 2],
                max: [3.0; 2],
            });
            app.documents[0].view.zoom = 1.5;
            app.documents[0].view.angle = 90.0;
            drawer_core::files::save(
                &mut app.documents[0],
                &root.join(format!("transfer-{command}-{pen}.drawer")),
            )
            .map_err(|e| e.to_string())?;
            step(&ctx, &mut app, &mut time, vec![]);
            step(&ctx, &mut app, &mut time, vec![]);
            let before = app.documents[0].snapshot();
            app.execute_command(&ctx, command);
            for (p, phase) in [([2.5, 2.5], Phase::Down), ([-3.5, 6.5], Phase::Move)] {
                let e = event(&ctx, &app, p, phase, pen);
                app.drawing.virtual_events(vec![e], pen);
                step(&ctx, &mut app, &mut time, vec![]);
            }
            if !app.drawing.pending()
                || app.drawing.movement_preview(app.documents[0].id).is_none()
                || app.documents[0].snapshot() != before
                || app.documents[0].dirty
            {
                return Err("Movement preview mutated document or missing".into());
            }
            let e = event(&ctx, &app, [-3.5, 6.5], Phase::Up, pen);
            app.drawing.virtual_events(vec![e], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            let d = &mut app.documents[0];
            if d.scene.selection_translation != [-6.0, 4.0]
                || d.history_counts() != (1, 0)
                || !d.dirty
            {
                return Err("Movement position/history".into());
            }
            if target == Target::Boundary {
                if d.layers != before.layers {
                    return Err("Boundary moved pixels".into());
                }
            } else if d.layers[0].pixel(16, 12, -4, 6) != [40, 50, 60, 128]
                || d.layers[0].pixel(16, 12, 2, 2)[3] != 0
            {
                return Err("Selected cut destination/source".into());
            }
            let after = d.snapshot();
            d.undo();
            if d.snapshot() != before || d.dirty {
                return Err("Movement Undo".into());
            }
            d.redo();
            if d.snapshot() != after {
                return Err("Movement Redo".into());
            }
            d.undo();
            if app.drawing.pending() || !app.workspaces.input_idle() {
                return Err("Movement input not released".into());
            }
            for cancel in 0..5 {
                app.documents[0].view.zoom = 1.5;
                app.documents[0].view.angle = 90.0;
                app.execute_command(&ctx, command);
                let e = event(&ctx, &app, [2.5, 2.5], Phase::Down, pen);
                app.drawing.virtual_events(vec![e], pen);
                step(&ctx, &mut app, &mut time, vec![]);
                let e = event(&ctx, &app, [8.5, 7.5], Phase::Move, pen);
                app.drawing.virtual_events(vec![e], pen);
                step(&ctx, &mut app, &mut time, vec![]);
                let events = match cancel {
                    0 => vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    1 => vec![egui::Event::WindowFocused(false)],
                    2 => {
                        app.documents[0].view.zoom = 2.0;
                        vec![]
                    }
                    3 => {
                        app.execute_command(&ctx, "select-rectangle");
                        vec![]
                    }
                    _ => {
                        app.begin_paste(&ctx, || {
                            Ok(drawer_core::paste::ClipboardImage {
                                color: None,
                                image: drawer_core::references::LocalImage {
                                    width: 1,
                                    height: 1,
                                    pixels: vec![[0; 4]],
                                },
                                origin: None,
                            })
                        })?;
                        vec![]
                    }
                };
                step(&ctx, &mut app, &mut time, events);
                if app.drawing.pending() || app.documents[0].snapshot() != before {
                    return Err(format!("Movement cancellation {cancel}"));
                }
                app.paste_dialog = None;
                step(
                    &ctx,
                    &mut app,
                    &mut time,
                    vec![
                        egui::Event::WindowFocused(true),
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        }
    }
    // Active layer (rather than top), clipboard round-trip, cancellation and animation preservation.
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 2,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels[0] = [11, 22, 33, 0];
    app.documents[0].layers[0].pixels[1] = [40, 50, 60, 128];
    let top = app.documents[0].layers[0].clone();
    app.documents[0].layers.push(top);
    app.documents[0].layers[1].pixels.fill([255, 0, 0, 255]);
    app.documents[0].scene.selection = Some(Rect {
        min: [0.0; 2],
        max: [2.0, 1.0],
    });
    use drawer_core::animation::{Cell, Exposure, Fps, Timeline, Track};
    app.documents[0].animation = Some(Timeline {
        fps: Fps {
            numerator: 24,
            denominator: 1,
        },
        frames: 2,
        tracks: vec![Track {
            name: "Two cells".into(),
            cells: (0..2)
                .map(|i| Cell {
                    name: format!("Cell {i}"),
                    width: 1,
                    height: 1,
                    offset: [3, 1],
                    pixels: std::sync::Arc::new(vec![[i * 100, 20, 30, 255]]),
                })
                .collect(),
            exposures: vec![
                Exposure { frame: 0, cell: 0 },
                Exposure { frame: 1, cell: 1 },
            ],
            end: 2,
        }],
        audio: vec![],
    });
    app.documents[0].current_frame = 1;
    let before = app.documents[0].snapshot();
    let mut packet = None;
    app.copy_selection_from(|p, b| {
        packet = Some(drawer_core::paste::ClipboardImage::decode(
            &p.encode(b)?,
            b,
        )?);
        Ok(())
    })?;
    if app.documents[0].snapshot() != before {
        return Err("Copy changed source".into());
    }
    let p = packet.take().unwrap();
    if p.image.pixels != vec![[11, 22, 33, 0], [40, 50, 60, 128]] {
        return Err("Copy ignored active raster/selection".into());
    }
    app.begin_paste(&ctx, || Ok(p))?;
    app.paste_dialog.as_mut().unwrap().move_to([-20, 10])?;
    if app.paste_dialog.as_ref().unwrap().preview.animation != before.animation
        || app.paste_dialog.as_ref().unwrap().preview.current_frame != 1
    {
        return Err("Paste preview discarded animation".into());
    }
    app.accept_paste()?;
    let d = &mut app.documents[0];
    if d.animation != before.animation {
        return Err("Paste discarded animation".into());
    }
    if d.layers.len() != 3
        || d.layers[2].pixel(4, 2, -20, 10) != [11, 22, 33, 0]
        || d.layers[0] != before.layers[0]
        || d.history_counts() != (1, 0)
    {
        return Err("Paste changed source or history".into());
    }
    let path = root.join("selected-paste.drawer");
    drawer_core::files::save(d, &path).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Selection paste reopen".into());
    }
    d.undo();
    if d.snapshot() != before {
        return Err("Paste Undo".into());
    }
    verify_picker(root)?;
    Ok(())
}
fn verify_picker(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::{layer_pick::Settings, thumbnail::Target as Layer};
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    let mut time = 0.0;
    app.create_canvas(CanvasSpec {
        width: 12,
        height: 10,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels[..2].fill([20, 30, 40, 255]);
    for _ in 0..2 {
        let l = app.documents[0].layers[0].clone();
        app.documents[0].layers.push(l);
    }
    app.documents[0].layers[2].pixels[0][3] = 0;
    app.documents[0].view.zoom = 2.0;
    app.documents[0].view.angle = 90.0;
    let id = app.documents[0].id;
    drawer_core::files::save(&mut app.documents[0], &root.join("layer-pick.drawer"))
        .map_err(|e| e.to_string())?;
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    app.execute_command(&ctx, "Move");
    let before = app.documents[0].snapshot();
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    for phase in [Phase::Down, Phase::Up] {
        let e = event(&ctx, &app, [0.5, 0.5], phase, false);
        app.drawing.virtual_events(vec![e], false);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(alt)],
        );
    }
    step(&ctx, &mut app, &mut time, vec![]);
    if app.panels.targets.get(&id) != Some(&Layer::Raster(1))
        || app.documents[0].snapshot() != before
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Canvas pick alpha hole or selection edited pixels".into());
    }
    app.execute_command(&ctx, "pick-next-layer");
    if app.panels.targets[&id] != Layer::Raster(0) {
        return Err("Pick next overlap".into());
    }
    app.execute_command(&ctx, "pick-next-layer");
    if app.panels.targets[&id] != Layer::Raster(1) {
        return Err("Pick overlap wrap".into());
    }
    // Empty click must not start moving the formerly selected layer.
    for phase in [Phase::Down, Phase::Up] {
        let e = event(&ctx, &app, [8.5, 8.5], phase, false);
        app.drawing.virtual_events(vec![e], false);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(alt)],
        );
    }
    if app.panels.targets[&id] != Layer::Raster(1) || app.documents[0].snapshot() != before {
        return Err("Empty pick altered target/document".into());
    }
    for (p, phase, mods) in [
        ([1.5, 0.5], Phase::Down, alt),
        ([4.5, 2.5], Phase::Move, Default::default()),
        ([4.5, 2.5], Phase::Up, Default::default()),
    ] {
        let e = event(&ctx, &app, p, phase, false);
        app.drawing.virtual_events(vec![e], false);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(mods)],
        );
    }
    if app.panels.targets[&id] != Layer::Raster(2)
        || app.documents[0].layers[2].extent(12, 10).offset != [3, 2]
        || app.documents[0].history_counts() != (1, 0)
    {
        return Err("Pick then drag target/modifier release".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != before {
        return Err("Pick drag Undo".into());
    }
    app.documents[0].layers[2].locked = true;
    app.workspaces.store.current.layer_pick.include_locked = true;
    for phase in [Phase::Down, Phase::Up] {
        let e = event(&ctx, &app, [1.5, 0.5], phase, false);
        app.drawing.virtual_events(vec![e], false);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(alt)],
        );
    }
    step(&ctx, &mut app, &mut time, vec![]);
    if app.panels.targets[&id] != Layer::Raster(2)
        || !app.documents[0].layers[2].locked
        || app.drawing.pending()
        || app.documents[0].history_counts() != (0, 1)
    {
        return Err("Pick removed lock or committed pixels".into());
    }
    app.workspaces.store.current.layer_pick = Settings {
        modifier: drawer_core::layer_pick::Modifier::Ctrl,
        include_locked: true,
        folders: true,
    };
    let policy_path = root.join("layer-pick-workspace.json");
    app.workspaces
        .store
        .current
        .export(&policy_path)
        .map_err(|e| e.to_string())?;
    let restored =
        drawer_core::workspaces::Snapshot::read(&policy_path).map_err(|e| e.to_string())?;
    if restored.layer_pick != app.workspaces.store.current.layer_pick {
        return Err("Layer pick policy persistence".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn movement_copy_paste_production_input_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
