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
    while app.local_color.working() {
        if std::time::Instant::now() > until {
            return Err("Local color worker timed out".into());
        }
        frame(ctx, app, time, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::local_color_verify::verify(root)?;
    let reference = drawer_core::local_color_verify::write_reference(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("local-color-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 2,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::local_color_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for reference_mode in [true, false] {
        let counts = app.documents[0].history_counts();
        app.execute_command(
            &ctx,
            if reference_mode {
                "local-reference-color"
            } else {
                "local-light-shadow"
            },
        );
        let e = app
            .local_color
            .draft
            .as_mut()
            .ok_or("Local color command failed")?;
        e.config = if reference_mode {
            Config::Reference {
                path: reference.to_string_lossy().into_owned(),
                page: 1,
                strength: 0.8,
            }
        } else {
            Config::Lighting(Correction::Lighting {
                light: [1., 0.5, 0.25],
                shadow: [0.25, 0.5, 1.],
                strength: 0.8,
            })
        };
        settle(&ctx, &mut app, &mut time);
        let rect = app
            .local_color
            .preview
            .ok_or("Local color Preview missing")?;
        click(&ctx, &mut app, &mut time, rect);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .local_color
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!("Local preview isolation: {:?}", app.status));
        }
        let pending = root.join("pending-local-color.drawer");
        if app.save_to(0, &pending).is_ok() || pending.exists() {
            return Err("Pending color correction saved as confirmed".into());
        }
        settle(&ctx, &mut app, &mut time);
        let rect = app.local_color.apply.ok_or("Local color Apply missing")?;
        click(&ctx, &mut app, &mut time, rect);
        if app.local_color.draft.is_some()
            || app.documents[0].history_counts().0 != counts.0 + 1
            || app.documents[0].layers[..original.layers.len()] != original.layers
        {
            return Err(format!(
                "Local Apply must be one Undo, retaining pixels: {:?}",
                app.status
            ));
        }
        let path = root.join("local-color-app.drawer");
        app.save_to(0, &path)?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Local color application native mismatch".into());
        }
        // Saved parameters remain editable without opening the reference again.
        app.begin_adjustment()?;
        app.adjustments.draft = None;
        if !app.documents[0].undo() || app.documents[0].snapshot() != original {
            return Err("Local correction Undo failed".into());
        }
    }
    app.begin_local_color(true)?;
    app.local_color.draft.as_mut().unwrap().config = Config::Reference {
        path: reference.to_string_lossy().into_owned(),
        page: 1,
        strength: 0.,
    };
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.apply_local_color()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Strength zero changed pixels/history/dirty".into());
    }
    app.begin_local_color(true)?;
    app.local_color.draft.as_mut().unwrap().config = Config::Reference {
        path: reference.to_string_lossy().into_owned(),
        page: 1,
        strength: 1.,
    };
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if let Config::Reference { path, .. } = &mut app.local_color.draft.as_mut().unwrap().config {
        *path = root
            .join("missing-reference.png")
            .to_string_lossy()
            .into_owned();
    }
    app.preview_local_color(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if app.local_color.draft.as_ref().unwrap().prepared.is_some() || app.apply_local_color().is_ok()
    {
        return Err("Failed reference reused an old preview".into());
    }
    app.local_color.draft = None;
    app.begin_local_color(false)?;
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
        return Err("Escape did not discard local correction".into());
    }
    app.begin_local_color(false)?;
    app.preview_local_color(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.apply_local_color().is_ok() {
        return Err("Stale frame adopted color preview".into());
    }
    app.documents[0].current_frame -= 1;
    app.local_color.draft = None;
    app.begin_local_color(false)?;
    app.active = None;
    frame(&ctx, &mut app, &mut time, vec![]);
    if app.local_color.draft.is_some() {
        return Err("Closed source retained color draft".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_reference_lighting_mouse_preview_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
