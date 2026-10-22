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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.spatial_blur.job.is_some() {
        if std::time::Instant::now() > deadline {
            return Err("Blur worker timeout".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::spatial_blur_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("blur-app-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 17,
        height: 17,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::spatial_blur_verify::fixture()?.snapshot())?;
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    let original = app.documents[0].snapshot();
    for (command, mode) in ["motion-blur", "radial-blur", "spin-blur", "lens-blur"]
        .into_iter()
        .zip(Mode::ALL)
    {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        let e = app
            .spatial_blur
            .draft
            .as_mut()
            .ok_or("Blur command missing")?;
        e.settings = Settings {
            mode,
            length: 2.,
            radius: 4.,
            center: [6., 6.],
            angle: if mode == Mode::Spin { 30. } else { 0. },
            ..Default::default()
        };
        settle(&ctx, &mut app, &mut time);
        let button = app.spatial_blur.preview.ok_or("Blur Preview missing")?;
        click(&ctx, &mut app, &mut time, button);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .spatial_blur
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Blur mouse preview/source isolation {mode:?} at {button:?}: status={:?} draft={} prepared={} source={} history={:?}/{counts:?}",
                app.status,
                app.spatial_blur.draft.is_some(),
                app.spatial_blur
                    .draft
                    .as_ref()
                    .is_some_and(|e| e.prepared.is_some()),
                app.documents[0].snapshot() == original,
                app.documents[0].history_counts()
            ));
        }
        let file = root.join("unconfirmed-spatial-blur.drawer");
        if app.save_to(0, &file).is_ok() || file.exists() {
            return Err("Unconfirmed blur save did not offer apply/cancel resolution".into());
        }
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != original
        {
            return Err("Unconfirmed blur was saved".into());
        }
        std::fs::remove_file(&file).map_err(|e| e.to_string())?;
        settle(&ctx, &mut app, &mut time);
        let button = app.spatial_blur.apply.ok_or("Blur Apply missing")?;
        click(&ctx, &mut app, &mut time, button);
        if app.spatial_blur.draft.is_some()
            || app.documents[0].snapshot() == original
            || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Blur mouse Apply/one Undo: {:?}", app.status));
        }
        if app.workspaces.store.current.spatial_blur.values[mode.index()]
            != app.documents[0].scene.last_spatial_blur
            || drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
                .map_err(|e| e.to_string())?
                .unwrap()
                .current
                .spatial_blur
                != app.workspaces.store.current.spatial_blur
        {
            return Err("Applied blur settings not persisted locally".into());
        }
        let file = root.join("applied-spatial-blur.drawer");
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Blur app native/Undo".into());
        }
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    let presets = app.workspaces.store.current.spatial_blur.clone();
    app.begin_spatial_blur(Mode::Motion)?;
    app.spatial_blur.draft.as_mut().unwrap().settings.length = 0.;
    app.preview_spatial_blur(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.apply_spatial_blur()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
        || app.workspaces.store.current.spatial_blur != presets
    {
        return Err("Zero blur lost dirty/Redo/presets".into());
    }
    app.begin_spatial_blur(Mode::Lens)?;
    app.preview_spatial_blur(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.documents[0].current_frame += 1;
    if app.apply_spatial_blur().is_ok() {
        return Err("Stale blur frame accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.spatial_blur.draft.as_mut().unwrap().settings.sides = 2;
    if app.preview_spatial_blur(&ctx).is_ok()
        || app.spatial_blur.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_spatial_blur().is_ok()
    {
        return Err("Failed blur retained old plan".into());
    }
    app.begin_spatial_blur(Mode::Lens)?;
    app.preview_spatial_blur(&ctx)?;
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
    if app.spatial_blur.draft.is_some()
        || app.documents[0].snapshot() != original
        || app.workspaces.store.current.spatial_blur != presets
    {
        return Err("Escape committed blur/preset".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    app.begin_spatial_blur(Mode::Motion)?;
    app.preview_spatial_blur(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.spatial_blur.draft.as_ref().unwrap().prepared.is_some() {
        return Err("Stale worker result accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.spatial_blur.draft = None;
    app.create_canvas(CanvasSpec {
        width: 128,
        height: 128,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let n = app.active.unwrap();
    app.documents[n].layers[0].pixels[64 * 128 + 64] = [255; 4];
    let before = app.documents[n].snapshot();
    let counts = app.documents[n].history_counts();
    app.begin_spatial_blur(Mode::Lens)?;
    app.spatial_blur.draft.as_mut().unwrap().settings.radius = 16.;
    settle(&ctx, &mut app, &mut time);
    let button = app.spatial_blur.preview.unwrap();
    click(&ctx, &mut app, &mut time, button);
    let button = app.spatial_blur.cancel.ok_or("Blur cancel missing")?;
    let start = std::time::Instant::now();
    click(&ctx, &mut app, &mut time, button);
    if start.elapsed() > std::time::Duration::from_secs(2) || app.spatial_blur.draft.is_some() {
        return Err("Large blur cancel blocked UI".into());
    }
    wait(&ctx, &mut app, &mut time)?;
    if app.documents[n].snapshot() != before
        || app.documents[n].history_counts() != counts
        || app.workspaces.store.current.spatial_blur != presets
    {
        return Err("Cancelled background blur committed pixels/history/presets".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_blurs_async_cancel_native_undo_stale_and_local_settings() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
