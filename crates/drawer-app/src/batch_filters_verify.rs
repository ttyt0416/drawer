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
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.batch_filters.working() {
        if std::time::Instant::now() > until {
            return Err("Batch worker timeout".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::batch_filters_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("batch-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 7,
        height: 7,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::batch_filters_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for task in drawer_core::batch_filters_verify::tasks() {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, "batch-filters");
        if app.batch_filters.draft.is_none() {
            return Err("Batch command missing".into());
        }
        app.batch_filters.draft.as_mut().unwrap().config.task = task;
        settle(&ctx, &mut app, &mut time);
        let rect = *app
            .batch_filters
            .targets
            .get(&Target::Raster(2))
            .ok_or("Batch target checkbox missing")?;
        click(&ctx, &mut app, &mut time, rect);
        settle(&ctx, &mut app, &mut time);
        if app
            .batch_filters
            .draft
            .as_ref()
            .unwrap()
            .config
            .targets
            .len()
            != 2
        {
            return Err("Actual batch target checkbox did not select second layer".into());
        }
        let rect = app.batch_filters.preview.ok_or("Batch Preview missing")?;
        click(&ctx, &mut app, &mut time, rect);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .batch_filters
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!("Batch preview isolation: {:?}", app.status));
        }
        let pending = root.join("pending-batch.drawer");
        if app.save_to(0, &pending).is_ok() || pending.exists() {
            return Err("Unconfirmed batch must resolve before saving".into());
        }
        settle(&ctx, &mut app, &mut time);
        let rect = app.batch_filters.apply.ok_or("Batch Apply missing")?;
        click(&ctx, &mut app, &mut time, rect);
        if app.batch_filters.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Batch Apply all / one Undo: {:?}", app.status));
        }
        let path = root.join("batch-app.drawer");
        app.save_to(0, &path)?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Batch app native / Undo all".into());
        }
    }
    app.begin_batch_filters()?;
    app.batch_filters
        .draft
        .as_mut()
        .unwrap()
        .config
        .targets
        .push(Target::Vector(1));
    if app.preview_batch_filters(&ctx).is_ok()
        || app.batch_filters.working()
        || app.documents[0].snapshot() != original
    {
        return Err("Unsupported batch target was processed".into());
    }
    app.batch_filters.draft = None;
    app.begin_batch_filters()?;
    app.batch_filters
        .draft
        .as_mut()
        .unwrap()
        .config
        .targets
        .push(Target::Raster(2));
    app.preview_batch_filters(&ctx)?;
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
    if app.batch_filters.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Batch Escape committed partial results".into());
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
    app.begin_batch_filters()?;
    app.preview_batch_filters(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.apply_batch_filters().is_ok() {
        return Err("Stale batch accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.batch_filters.draft = None;
    app.begin_batch_filters()?;
    app.preview_batch_filters(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.batch_filters
        .draft
        .as_mut()
        .unwrap()
        .config
        .targets
        .push(Target::Vector(1));
    if app.preview_batch_filters(&ctx).is_ok() || app.apply_batch_filters().is_ok() {
        return Err("Failed batch retained old plan".into());
    }
    app.batch_filters.draft = None;
    app.begin_batch_filters()?;
    app.active = None;
    frame(&ctx, &mut app, &mut time, vec![]);
    if app.batch_filters.draft.is_some() {
        return Err("Closing source page retained invalid batch".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_multi_target_mouse_atomic_preview_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
