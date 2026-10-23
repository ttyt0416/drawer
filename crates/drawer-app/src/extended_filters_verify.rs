use super::verify::{click, settle, wait};
use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::extended_filters_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::extended_filters_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    for command in drawer_core::extended_filters::Operation::COMMANDS {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.detail_filter.draft.is_none() {
            return Err(format!("Missing filter command {command}"));
        }
        settle(&ctx, &mut app, &mut time);
        let preview = app
            .detail_filter
            .preview
            .ok_or("Finishing preview missing")?;
        click(&ctx, &mut app, &mut time, preview);
        wait(&ctx, &mut app, &mut time)?;
        let e = app.detail_filter.draft.as_ref().unwrap();
        if e.prepared.is_none() || app.documents[0].snapshot() != original {
            return Err(format!("Finishing preview {command}: {:?}", app.status));
        }
        let changed = e.prepared.as_ref().unwrap().1 != original;
        settle(&ctx, &mut app, &mut time);
        let apply = app.detail_filter.apply.ok_or("Finishing Apply missing")?;
        click(&ctx, &mut app, &mut time, apply);
        if app.detail_filter.draft.is_some()
            || app.documents[0].history_counts().0 != counts.0 + usize::from(changed)
        {
            return Err(format!(
                "Finishing mouse Apply/Undo {command}: {:?}",
                app.status
            ));
        }
        let file = root.join("finishing-ui.drawer");
        app.save_to(0, &file)?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Finishing UI native roundtrip".into());
        }
        if changed && (!app.documents[0].undo() || app.documents[0].snapshot() != original) {
            return Err("Finishing UI Undo".into());
        }
    }
    app.execute_command(&ctx, "normal-map");
    let settings = app.detail_filter.draft.as_ref().unwrap().settings;
    app.save_detail_preset("Own normal".into(), settings)?;
    if app.workspaces.store.current.finishing_presets.values.len() != 1 {
        return Err("Save local filter preset".into());
    }
    app.detail_filter.draft = None;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn finishing_commands_mouse_preview_apply_presets_native_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
