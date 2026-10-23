use super::verify::{click, settle, wait};
use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::filter_options_verify::verify(root)?;
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
    for (y, row) in app.documents[0].layers[0]
        .pixels
        .as_chunks_mut::<17>()
        .0
        .iter_mut()
        .enumerate()
    {
        for (x, p) in row.iter_mut().enumerate() {
            *p = [(x * 15) as u8, (y * 15) as u8, 0, 255];
        }
    }
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    app.execute_command(&ctx, "wave-filter");
    let settings = {
        let draft = app.detail_filter.draft.as_mut().ok_or("Wave command")?;
        draft.settings.options = drawer_core::filter_options::Options {
            boundary: drawer_core::filter_options::Boundary::Mirror,
            waveform: drawer_core::filter_options::Waveform::Rectangular,
            ..Default::default()
        };
        draft.settings
    };
    settle(&ctx, &mut app, &mut time);
    let button = app.detail_filter.preview.ok_or("Options preview button")?;
    click(&ctx, &mut app, &mut time, button);
    wait(&ctx, &mut app, &mut time)?;
    if app.documents[0].snapshot() != before
        || app
            .detail_filter
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!("Options preview isolation: {:?}", app.status));
    }
    settle(&ctx, &mut app, &mut time);
    let apply = app.detail_filter.apply.ok_or("Options Apply button")?;
    click(&ctx, &mut app, &mut time, apply);
    if app.detail_filter.draft.is_some()
        || app.documents[0].scene.last_detail_filter != Some(settings)
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!("Options Apply one Undo: {:?}", app.status));
    }
    let path = root.join("options-ui.drawer");
    app.save_to(0, &path)?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Options native UI".into());
    }
    app.save_detail_preset("Rectangular mirror".into(), settings)?;
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Options UI Undo".into());
    }
    app.execute_command(&ctx, "wave-filter");
    app.detail_filter
        .draft
        .as_mut()
        .ok_or("Reopen wave")?
        .settings = settings;
    settle(&ctx, &mut app, &mut time);
    let cancel = app.detail_filter.cancel.ok_or("Options cancel button")?;
    click(&ctx, &mut app, &mut time, cancel);
    if app.detail_filter.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Options UI cancel".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn filter_options_mouse_preview_apply_native_preset_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
