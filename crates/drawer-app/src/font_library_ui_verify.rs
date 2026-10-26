use super::*;
use crate::text_editor::verify::{click, frame, settle};
fn install_fixture(
    app: &mut Drawer,
    ctx: &egui::Context,
    root: &std::path::Path,
) -> Result<Arc<Catalog>, String> {
    let catalog = Arc::new(drawer_core::font_library_verify::fixture(root)?);
    app.text_editor.fonts.scan = None;
    app.text_editor.fonts.loaded = true;
    app.text_editor.fonts.catalog = catalog.clone();
    app.publish_fonts(ctx);
    Ok(catalog)
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::font_library_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    let user = UserStorage::at(root.join("font-library-user")).map_err(|e| e.to_string())?;
    app.storage = Some(user);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 240,
        height: 180,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    app.execute_command(&ctx, "font-library");
    let catalog = install_fixture(&mut app, &ctx, &root.join("font-ui-fixture"))?;
    settle(&ctx, &mut app, &mut time);
    let rename = app
        .text_editor
        .fonts
        .rename
        .ok_or("Font list disabled action")?;
    click(&ctx, &mut app, &mut time, rename);
    if app.text_editor.fonts.error.is_some() {
        return Err("Rename active with no font lists".into());
    }
    app.text_editor.fonts.name = "나의 글꼴".into();
    let create = app.text_editor.fonts.create.ok_or("Create font list")?;
    click(&ctx, &mut app, &mut time, create);
    settle(&ctx, &mut app, &mut time);
    let draft = app
        .text_editor
        .fonts
        .draft
        .as_ref()
        .ok_or("Font preferences draft")?;
    if draft.lists.len() != 1
        || !draft.lists[0].members.is_empty()
        || !app.workspaces.store.current.font_lists.lists.is_empty()
    {
        return Err("Font-list create was not empty / was persisted before Apply".into());
    }
    let cancel = app
        .text_editor
        .fonts
        .cancel
        .ok_or("Cancel font preferences")?;
    click(&ctx, &mut app, &mut time, cancel);
    if app.text_editor.fonts.open || !app.workspaces.store.current.font_lists.lists.is_empty() {
        return Err("Cancel failed font preference restoration".into());
    }
    app.execute_command(&ctx, "font-library");
    app.text_editor.fonts.scan = None;
    app.text_editor.fonts.catalog = catalog.clone();
    app.text_editor.fonts.name = "나의 글꼴".into();
    settle(&ctx, &mut app, &mut time);
    let create = app.text_editor.fonts.create.unwrap();
    click(&ctx, &mut app, &mut time, create);
    let entry = catalog
        .entries
        .iter()
        .find(|e| matches!(e.font, Font::Local(_)) && e.name.to_lowercase().contains("ubuntu"))
        .ok_or("Local UI font")?;
    let font = entry.font;
    let file = entry.path.as_ref().unwrap();
    let before = std::fs::read(file).map_err(|e| e.to_string())?;
    let settings = app.text_editor.fonts.draft.as_mut().unwrap();
    let id = settings.active.unwrap();
    settings.include(id, font, true)?;
    settings.favorite(font, true);
    settings.lists[0].display = Display::OwnFont;
    settle(&ctx, &mut app, &mut time);
    let apply = app.text_editor.fonts.apply.unwrap();
    click(&ctx, &mut app, &mut time, apply);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.fonts.open
        || choices(&ctx).0 != vec![font]
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err("Font list Apply changed document or did not filter selector".into());
    }
    let storage = app.storage.as_ref().unwrap();
    let restored = drawer_core::workspaces::Store::load(storage)
        .map_err(|e| e.to_string())?
        .ok_or("Font settings missing after restart")?;
    if restored.current.font_lists != app.workspaces.store.current.font_lists
        || restored.font_restore.is_none()
    {
        return Err("Font preferences/favorites/restore record were not persistent".into());
    }
    app.execute_command(&ctx, "font-library");
    app.text_editor.fonts.scan = None;
    app.text_editor.fonts.catalog = catalog.clone();
    app.text_editor.fonts.character_font = font;
    app.text_editor.fonts.code = 0x41;
    settle(&ctx, &mut app, &mut time);
    let insert = app
        .text_editor
        .fonts
        .insert
        .ok_or("Character insert control")?;
    click(&ctx, &mut app, &mut time, insert);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.text != "A")
        || app.documents[0].snapshot() != original
    {
        return Err(format!(
            "Character insertion was not isolated editable Unicode: {:?}",
            app.text_editor.fonts.error
        ));
    }
    // Preferences Escape closes this window, preserving the pending text edit behind it.
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.text_editor.fonts.open || app.text_editor.draft.is_none() {
        return Err("Font preference Escape discarded the pending text".into());
    }
    settle(&ctx, &mut app, &mut time);
    let preview = app.text_editor.preview.unwrap();
    click(&ctx, &mut app, &mut time, preview);
    settle(&ctx, &mut app, &mut time);
    let apply = app.text_editor.apply.unwrap();
    click(&ctx, &mut app, &mut time, apply);
    if app.text_editor.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1 {
        return Err(format!(
            "Inserted character document Apply: {:?}",
            app.status
        ));
    }
    let saved = app.documents[0].snapshot();
    let path = root.join("font-char-ui.drawer");
    app.save_to(0, &path)?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != saved
    {
        return Err("Local character native roundtrip".into());
    }
    if std::fs::read(file).map_err(|e| e.to_string())? != before {
        return Err("Font preferences / character insertion changed the source font file".into());
    }
    let previous = app
        .workspaces
        .store
        .font_restore
        .as_deref()
        .cloned()
        .unwrap();
    app.commit_fonts(&ctx, previous)?;
    if !app.workspaces.store.current.font_lists.lists.is_empty()
        || app.documents[0].snapshot() != saved
    {
        return Err("Font settings restore changed the drawing or failed".into());
    }
    settle(&ctx, &mut app, &mut time);
    std::fs::remove_file(file).map_err(|e| e.to_string())?;
    app.accept_font_catalog(
        &ctx,
        Catalog::discover(&[file.parent().unwrap().to_path_buf()], || false),
    );
    if app.canvas_view.rendered_document_id().is_some() {
        return Err("Font refresh retained the previous canvas texture".into());
    }
    if app.documents[0].snapshot() != saved {
        return Err("Font refresh changed document content".into());
    }
    std::fs::write(file, &before).map_err(|e| e.to_string())?;
    app.accept_font_catalog(
        &ctx,
        Catalog::discover(&[file.parent().unwrap().to_path_buf()], || false),
    );
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Character insertion document Undo".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn font_library_pending_lists_selector_preview_escape_character_native_undo_and_readonly_fonts()
    {
        let r = crate::tests::TestDirectory::new();
        super::smoke(&r.0).unwrap();
    }
}
