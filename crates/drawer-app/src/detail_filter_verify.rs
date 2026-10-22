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
    while app.detail_filter.job.is_some() {
        if std::time::Instant::now() > deadline {
            return Err("Detail worker timeout".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::detail_filter_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 7,
        height: 7,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::detail_filter_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for command in ["sharpen", "unsharp-mask", "reduce-noise"] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.detail_filter.draft.is_none() {
            return Err("Detail command missing".into());
        }
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.preview.ok_or("Detail Preview missing")?;
        click(&ctx, &mut app, &mut time, button);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .detail_filter
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Detail mouse preview/source isolation: {:?}",
                app.status
            ));
        }
        let file = root.join("unconfirmed-detail.drawer");
        if app.save_to(0, &file).is_ok() || file.exists() {
            return Err("Unconfirmed detail save did not prompt resolution".into());
        }
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != original
        {
            return Err("Unconfirmed detail was saved".into());
        }
        std::fs::remove_file(&file).map_err(|e| e.to_string())?;
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.apply.ok_or("Detail Apply missing")?;
        click(&ctx, &mut app, &mut time, button);
        if app.detail_filter.draft.is_some()
            || app.documents[0].snapshot() == original
            || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Detail mouse Apply/one Undo: {:?}", app.status));
        }
        let file = root.join("applied-detail.drawer");
        app.save_to(0, &file)?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Detail app native/Undo".into());
        }
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    for op in [
        Operation::Sharpen { strength: 0. },
        Operation::Unsharp {
            radius: 0,
            amount: 1.,
            threshold: 0.,
        },
        Operation::Denoise {
            luminance: 0.,
            chroma: 0.,
            detail: 0.,
        },
    ] {
        app.begin_detail_filter(op)?;
        app.preview_detail_filter(&ctx)?;
        wait(&ctx, &mut app, &mut time)?;
        app.apply_detail_filter()?;
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Zero detail lost dirty/Redo".into());
    }
    app.begin_detail_filter(Operation::defaults()[0])?;
    app.preview_detail_filter(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.documents[0].current_frame += 1;
    if app.apply_detail_filter().is_ok() {
        return Err("Stale detail frame accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.detail_filter.draft.as_mut().unwrap().settings.operation =
        Operation::Sharpen { strength: f64::NAN };
    if app.preview_detail_filter(&ctx).is_ok()
        || app.detail_filter.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_detail_filter().is_ok()
    {
        return Err("Invalid detail retained an old plan".into());
    }
    app.begin_detail_filter(Operation::defaults()[1])?;
    app.preview_detail_filter(&ctx)?;
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
    if app.detail_filter.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Escape committed detail filter".into());
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
    app.begin_detail_filter(Operation::defaults()[0])?;
    app.preview_detail_filter(&ctx)?;
    app.detail_filter.draft.as_mut().unwrap().settings.operation = Operation::defaults()[2];
    wait(&ctx, &mut app, &mut time)?;
    if app.detail_filter.draft.as_ref().unwrap().prepared.is_some() {
        return Err("Old detail settings result accepted".into());
    }
    app.detail_filter.draft = None;
    app.create_canvas(CanvasSpec {
        width: 128,
        height: 128,
        dpi: 300.,
        background: Background::Color([80; 3]),
    })
    .map_err(|e| e.to_string())?;
    let n = app.active.unwrap();
    app.documents[n].layers[0].pixels[64 * 128 + 64] = [255; 4];
    let before = app.documents[n].snapshot();
    let counts = app.documents[n].history_counts();
    app.begin_detail_filter(Operation::Unsharp {
        radius: 32,
        amount: 1.,
        threshold: 0.,
    })?;
    settle(&ctx, &mut app, &mut time);
    let button = app.detail_filter.preview.unwrap();
    click(&ctx, &mut app, &mut time, button);
    let button = app.detail_filter.cancel.ok_or("Detail Cancel missing")?;
    let start = std::time::Instant::now();
    click(&ctx, &mut app, &mut time, button);
    if start.elapsed() > std::time::Duration::from_secs(2) || app.detail_filter.draft.is_some() {
        return Err("Detail cancellation blocked UI".into());
    }
    wait(&ctx, &mut app, &mut time)?;
    if app.documents[n].snapshot() != before || app.documents[n].history_counts() != counts {
        return Err("Cancelled detail modified pixels/history".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn detail_mouse_preview_apply_save_undo_zero_stale_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
