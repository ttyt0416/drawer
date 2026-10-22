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
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, rect: egui::Rect) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            time,
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
}
fn wait(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.surface_effects.working() {
        if std::time::Instant::now() > deadline {
            return Err("Effects worker timeout".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::surface_effects_verify::verify(root)?;
    drawer_core::surface_effects_verify::precision_and_limits(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("effects-app-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 25,
        height: 21,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::surface_effects_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for command in [
        "drop-shadow",
        "glow-effect",
        "emboss-effect",
        "crystallize-effect",
    ] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.surface_effects.draft.is_none() {
            return Err(format!(
                "Effects command missing: {command}: {:?}",
                app.status
            ));
        }
        if command == "crystallize-effect"
            && let Operation::Crystal { seed, .. } = &mut app
                .surface_effects
                .draft
                .as_mut()
                .unwrap()
                .config
                .stack
                .effects[0]
                .operation
        {
            *seed = u64::MAX - 1;
        }
        settle(&ctx, &mut app, &mut time);
        let rect = app
            .surface_effects
            .preview
            .ok_or("Preview button missing")?;
        click(&ctx, &mut app, &mut time, rect);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .surface_effects
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Effects mouse preview/source isolation: {:?}",
                app.status
            ));
        }
        let pending = root.join("pending-effects.drawer");
        if app.save_to(0, &pending).is_ok() || pending.exists() {
            return Err("Unconfirmed effect save did not require resolution".into());
        }
        settle(&ctx, &mut app, &mut time);
        let rect = app.surface_effects.apply.ok_or("Apply button missing")?;
        click(&ctx, &mut app, &mut time, rect);
        if app.surface_effects.draft.is_some()
            || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Effects mouse Apply / one Undo: {:?}", app.status));
        }
        let settings = app.workspaces.store.current.surface_effects.clone();
        if drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
            .map_err(|e| e.to_string())?
            .ok_or("Missing saved effect preferences")?
            .current
            .surface_effects
            != settings
        {
            return Err("Effects confirmed settings did not survive local restart".into());
        }
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != original
            || app.workspaces.store.current.surface_effects != settings
        {
            return Err("Effects Undo modified local recent settings".into());
        }
    }
    let recent = app.workspaces.store.current.surface_effects.clone();
    app.begin_surface_effects(Some(0))?;
    if let Operation::Shadow { distance, .. } = &mut app
        .surface_effects
        .draft
        .as_mut()
        .unwrap()
        .config
        .stack
        .effects[0]
        .operation
    {
        *distance = 27.;
    }
    app.preview_surface_effects(&ctx)?;
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
    if app.surface_effects.draft.is_some()
        || app.documents[0].snapshot() != original
        || app.workspaces.store.current.surface_effects != recent
    {
        return Err("Cancelled effect preview changed source or last-confirmed preset".into());
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
    app.begin_surface_effects(Some(0))?;
    app.preview_surface_effects(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if let Operation::Shadow { blur, .. } = &mut app
        .surface_effects
        .draft
        .as_mut()
        .unwrap()
        .config
        .stack
        .effects[0]
        .operation
    {
        *blur = 33;
    }
    if app.preview_surface_effects(&ctx).is_ok()
        || app.apply_surface_effects().is_ok()
        || app.workspaces.store.current.surface_effects != recent
    {
        return Err("Failed effects preview accepted stale state or changed last settings".into());
    }
    app.surface_effects.draft = None;
    app.begin_surface_effects(Some(0))?;
    app.preview_surface_effects(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.apply_surface_effects().is_ok()
        || app
            .surface_effects
            .draft
            .as_ref()
            .is_some_and(|e| e.prepared.is_some())
    {
        return Err("Stale effect worker was accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.surface_effects.draft = None;
    app.begin_surface_effects(Some(1))?;
    app.surface_effects.draft.as_mut().unwrap().config.raster = true;
    app.preview_surface_effects(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.apply_surface_effects()?;
    if app.documents[0].layers.len() != 2 || app.documents[0].layers[0] != original.layers[0] {
        return Err("Effects separate raster confirmation".into());
    }
    let file = root.join("effects-app.drawer");
    app.save_to(0, &file)?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Effects app native reopen".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_preview_apply_cancel_and_presets() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
