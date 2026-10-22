use super::*;
fn frame(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn settle(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    for _ in 0..3 {
        frame(ctx, app, time, vec![]);
    }
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, r: egui::Rect) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(r.center()),
                egui::Event::PointerButton {
                    pos: r.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
fn wait(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) -> Result<(), String> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.local_color.working() {
        if std::time::Instant::now() > until {
            return Err("Color-deepen worker timeout".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::color_deepen_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("deepen-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::color_deepen_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for correction in [false, true] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, "color-deepen");
        if app.local_color.draft.is_none() {
            return Err("Color-deepen command missing".into());
        }
        settle(&ctx, &mut app, &mut time);
        let r = if correction {
            app.local_color.layer_mode
        } else {
            app.local_color.pixel_mode
        }
        .ok_or("Color-deepen destination radio missing")?;
        click(&ctx, &mut app, &mut time, r);
        if !matches!(app.local_color.draft.as_ref().unwrap().config,Config::Deepen{correction_layer,..}if correction_layer==correction)
        {
            return Err("Destination radio did not choose pixels/layer".into());
        }
        settle(&ctx, &mut app, &mut time);
        let r = app
            .local_color
            .preview
            .ok_or("Color-deepen Preview missing")?;
        click(&ctx, &mut app, &mut time, r);
        wait(&ctx, &mut app, &mut time)?;
        let e = app.local_color.draft.as_ref().unwrap();
        if e.prepared.is_none()
            || e.note.as_ref().is_none_or(|n| !n.contains("limits"))
            || app.local_color.input.is_none()
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Color-deepen clipping preview/isolation: {:?}",
                app.status
            ));
        }
        if app.save_to(0, &root.join("pending-deepen.drawer")).is_ok() {
            return Err("Pending color-deepen saved as normal".into());
        }
        settle(&ctx, &mut app, &mut time);
        let r = app.local_color.apply.ok_or("Color-deepen Apply missing")?;
        click(&ctx, &mut app, &mut time, r);
        if app.local_color.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1 {
            return Err(format!("Color-deepen Apply/Undo count: {:?}", app.status));
        }
        if correction && app.documents[0].layers[0] != original.layers[0]
            || !correction && app.documents[0].layers[0] == original.layers[0]
        {
            return Err("Pixel/layer destination changed the wrong source".into());
        }
        let path = root.join("color-deepen-app.drawer");
        app.save_to(0, &path)?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Color-deepen app native mismatch".into());
        }
        if correction {
            app.begin_adjustment()?;
            let e = app
                .adjustments
                .draft
                .as_mut()
                .ok_or("Saved deepen correction did not reopen")?;
            let Some(Correction::Deepen(settings)) = &mut e.value.local else {
                return Err("Saved deepen parameters did not survive".into());
            };
            settings.brightness = 0.05;
            app.preview_adjustment(&ctx)?;
            app.apply_adjustment()?;
            if !app.documents[0].undo() {
                return Err("Edited deepen correction Undo missing".into());
            }
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != original {
            return Err("Color-deepen Undo did not restore original sources/metadata".into());
        }
    }
    app.begin_color_deepen()?;
    if let Config::Deepen { settings, .. } = &mut app.local_color.draft.as_mut().unwrap().config {
        *settings = drawer_core::color_deepen::Settings {
            brightness: 0.,
            saturation: 0.,
            contrast: 0.,
        };
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.apply_local_color()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Neutral deepen destroyed dirty/Redo".into());
    }
    app.begin_color_deepen()?;
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if let Config::Deepen { settings, .. } = &mut app.local_color.draft.as_mut().unwrap().config {
        settings.contrast = f64::NAN;
    }
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if app.apply_local_color().is_ok() || app.local_color.draft.as_ref().unwrap().prepared.is_some()
    {
        return Err("Invalid deepen retained old plan".into());
    }
    app.local_color.draft = None;
    app.begin_color_deepen()?;
    app.preview_local_color(&ctx)?;
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    wait(&ctx, &mut app, &mut time)?;
    if app.local_color.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Deepen Escape modified original".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_deepen_destination_clipping_preview_apply_edit_undo_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
