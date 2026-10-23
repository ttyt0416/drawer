use super::verify::{click, settle, wait};
use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::cleanup_filters_verify::verify(root)?;
    use drawer_core::cleanup_filters::{Dust, Mode, Operation as C, SelectionKind};
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 17,
        height: 17,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0]
        .commit_state(drawer_core::cleanup_filters_verify::fixture(true)?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.execute_command(&ctx, "cleanup-dust");
    settle(&ctx, &mut app, &mut time);
    wait(&ctx, &mut app, &mut time)?;
    if app
        .detail_filter
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none())
        || app.documents[0].snapshot() != original
    {
        return Err(format!(
            "Initial automatic cleanup preview: {:?}",
            app.status
        ));
    }
    let modify = |app: &mut Drawer, mode, max_area| {
        let draft = app.detail_filter.draft.as_mut().unwrap();
        draft.settings.operation = Operation::Pixels(
            drawer_core::pixel_effects::Operation::Cleanup(C::Dust(Dust {
                mode,
                max_area,
                ..Default::default()
            })),
        );
    };
    modify(&mut app, Mode::Surrounding, 2);
    settle(&ctx, &mut app, &mut time);
    wait(&ctx, &mut app, &mut time)?;
    let prepared = app
        .detail_filter
        .draft
        .as_ref()
        .and_then(|e| e.prepared.as_ref())
        .ok_or("Mode change did not refresh cleanup preview")?;
    if prepared.1.layers[0].pixels[144] != [40, 80, 120, 255] {
        return Err("Automatic surrounding fill result".into());
    }
    let pointer = prepared.1.layers[0].pixels.as_ptr();
    settle(&ctx, &mut app, &mut time);
    if app.detail_filter.job.is_some()
        || app
            .detail_filter
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .1
            .layers[0]
            .pixels
            .as_ptr()
            != pointer
    {
        return Err("Unchanged cleanup settings recomputed preview".into());
    }
    modify(&mut app, Mode::Surrounding, 1);
    settle(&ctx, &mut app, &mut time);
    wait(&ctx, &mut app, &mut time)?;
    if app
        .detail_filter
        .draft
        .as_ref()
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .1
        .layers[0]
        .pixels[144]
        != original.layers[0].pixels[144]
    {
        return Err("Area change did not refresh cleanup preview".into());
    }
    modify(&mut app, Mode::Surrounding, 2);
    settle(&ctx, &mut app, &mut time);
    wait(&ctx, &mut app, &mut time)?;
    let path = root.join("cleanup-unconfirmed.drawer");
    if app.save_to(0, &path).is_ok() || path.exists() {
        return Err("Unconfirmed cleanup save allowed".into());
    }
    let count = app.documents[0].history_counts();
    settle(&ctx, &mut app, &mut time);
    let apply = app.detail_filter.apply.ok_or("Cleanup Apply missing")?;
    click(&ctx, &mut app, &mut time, apply);
    if app.detail_filter.draft.is_some() || app.documents[0].history_counts().0 != count.0 + 1 {
        return Err(format!("Cleanup Apply/Undo: {:?}", app.status));
    }
    let path = root.join("cleanup-ui.drawer");
    app.save_to(0, &path)?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Cleanup UI native reopen".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Cleanup UI Undo".into());
    }
    for command in SelectionKind::COMMANDS {
        app.execute_command(&ctx, command);
        settle(&ctx, &mut app, &mut time);
        wait(&ctx, &mut app, &mut time)?;
        if app.detail_filter.draft.as_ref().is_none_or(|e| {
            e.candidates.is_none()
                || e.prepared
                    .as_ref()
                    .is_none_or(|(_, s)| s.layers != original.layers)
        }) {
            return Err(format!(
                "Selection-only preview {command}: {:?}",
                app.status
            ));
        }
        settle(&ctx, &mut app, &mut time);
        let apply = app.detail_filter.apply.ok_or("Selection Apply missing")?;
        click(&ctx, &mut app, &mut time, apply);
        if app.detail_filter.draft.is_some() || app.documents[0].layers != original.layers {
            return Err("Dust selection Apply changed pixels".into());
        }
        app.documents[0].undo();
    }
    app.execute_command(&ctx, "cleanup-dust");
    settle(&ctx, &mut app, &mut time);
    let cancel = app.detail_filter.cancel.ok_or("Cleanup cancel missing")?;
    click(&ctx, &mut app, &mut time, cancel);
    if app.detail_filter.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Cleanup cancellation changed document".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn cleanup_auto_preview_same_value_scope_selection_apply_native_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
