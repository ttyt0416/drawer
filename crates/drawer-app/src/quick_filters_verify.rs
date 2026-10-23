use super::verify::{click, settle, wait};
use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::quick_filters_verify::verify(root)?;
    use drawer_core::quick_filters::{self as q, Input, Kind, Mode, Output};
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("quick-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 17,
        height: 17,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::quick_filters_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    for (_, recipe) in q::factory(Kind::Raster) {
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Raster(0),
        );
        app.execute_command(&ctx, "quick-filters");
        let e = app
            .batch_filters
            .draft
            .as_mut()
            .ok_or("Quick command missing")?;
        e.config.task = recipe.task;
        e.config.route = Some(recipe.route);
        settle(&ctx, &mut app, &mut time);
        let preview = app.batch_filters.preview.ok_or("Quick preview button")?;
        click(&ctx, &mut app, &mut time, preview);
        wait(&ctx, &mut app, &mut time)?;
        if app.documents[0].snapshot() != original
            || app
                .batch_filters
                .draft
                .as_ref()
                .is_none_or(|e| e.prepared.is_none())
        {
            return Err(format!("Quick preview isolation: {:?}", app.status));
        }
        let path = root.join("quick-pending.drawer");
        if app.save_to(0, &path).is_ok() || path.exists() {
            return Err("Unconfirmed routed filter save".into());
        }
        settle(&ctx, &mut app, &mut time);
        let apply = app.batch_filters.apply.ok_or("Quick Apply button")?;
        click(&ctx, &mut app, &mut time, apply);
        if app.batch_filters.draft.is_some()
            || app.documents[0].layers.len() != 3
            || app.documents[0].layers[..2] != original.layers
        {
            return Err(format!("Quick Apply source preservation: {:?}", app.status));
        }
        let file = root.join("quick-ui.drawer");
        app.save_to(0, &file)?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Quick UI native reopen".into());
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != original {
            return Err("Quick UI Undo".into());
        }
    }
    quick::preset(
        &mut app,
        quick::PresetAction::Save(
            "Whole page copy".into(),
            q::Recipe {
                task: drawer_core::quick_filters_verify::identity(q::Route::default()).task,
                route: q::Route {
                    input: Input::Composite,
                    kind: Kind::Composite,
                    ..Default::default()
                },
            },
        ),
    )?;
    if app.workspaces.store.current.quick_presets.values.len() != 1 {
        return Err("Quick local preset save".into());
    }
    app.documents[0].commit_state(drawer_core::raster_conversion_verify::mixed()?.snapshot())?;
    let objects = app.documents[0].snapshot();
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(1)),
    );
    app.execute_command(&ctx, "pixel-filter-route");
    let e = app
        .batch_filters
        .draft
        .as_mut()
        .ok_or("Pixel output route command")?;
    e.config.route = Some(q::Route {
        mode: Mode::Pixels,
        output: Output::Current,
        kind: Kind::Object,
        ..Default::default()
    });
    settle(&ctx, &mut app, &mut time);
    app.preview_batch_filters(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    if !app.batch_filters.draft.as_ref().unwrap().loss
        || app.apply_batch_filters().is_ok()
        || app.documents[0].snapshot() != objects
    {
        return Err("Editable current output bypassed loss confirmation".into());
    }
    app.batch_filters.draft.as_mut().unwrap().confirm_loss = true;
    app.apply_batch_filters()?;
    if app.documents[0].scene.objects.len() >= objects.scene.objects.len() {
        return Err("Confirmed current output did not rasterize source".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != objects {
        return Err("Raster loss Undo".into());
    }
    app.execute_command(&ctx, "quick-filters");
    settle(&ctx, &mut app, &mut time);
    let cancel = app.batch_filters.cancel.ok_or("Quick Cancel")?;
    click(&ctx, &mut app, &mut time, cancel);
    if app.batch_filters.draft.is_some() || app.documents[0].snapshot() != objects {
        return Err("Quick Cancel changed source".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn quick_routes_mouse_new_results_presets_loss_guard_native_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
