use super::*;
use crate::text_editor::verify::{click, settle};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::text_extensions_verify::owned_fonts(root)?;
    drawer_core::text_extensions_verify::circular(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("font-path-ui-user")).map_err(|e| e.to_string())?);
    app.add_document(drawer_core::text_layout_verify::fixture()?);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    app.begin_font_files()?;
    app.text_editor.paths.source = Some(root.join("font-source/same.ttf"));
    app.text_editor.paths.licensed = true;
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.paths.install.ok_or("Font copy button")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.paths.owned.len() != 1 || app.documents[0].snapshot() != original {
        return Err(format!(
            "Font UI copy failed: {:?}",
            app.text_editor.paths.error
        ));
    }
    app.text_editor.paths.confirmation = app.text_editor.paths.owned[0].faces[0].name().into();
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.paths.remove.ok_or("Font remove button")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if !app.text_editor.paths.owned.is_empty()
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err(format!(
            "Font UI removal changed drawing or failed: {:?}",
            app.text_editor.paths.error
        ));
    }
    app.text_editor.paths.files = false;
    app.begin_text_editor(&ctx)?;
    let e = app.text_editor.draft.as_mut().unwrap();
    e.settings.text = "한글 AB\n다음 줄".into();
    e.settings.style = drawer_core::text_typeset_verify::style();
    e.settings.style.flow = core::Flow::Vertical;
    e.settings.style.effects = Some(path::Effects {
        underline: true,
        strike: true,
        slant: 15.,
    });
    e.settings.style.ruby.push(core::Ruby {
        start: 0,
        end: 1,
        text: "한".into(),
        font: Font::Hack,
        ..Default::default()
    });
    let regular = e.settings.clone();
    let mut pending = Box::new(Pending {
        circle: path::Circle {
            radius: 55.,
            ..Default::default()
        },
        prepared: None,
    });
    app.prepare_circle_preview(&ctx, &mut pending)?;
    app.text_editor.paths.pending = Some(pending);
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.paths.cancel.ok_or("Circle cancel")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.paths.pending.is_some()
        || app.text_editor.draft.as_ref().unwrap().settings != regular
        || app.documents[0].snapshot() != original
    {
        return Err("Cancel circle conversion changed the draft/source".into());
    }
    let mut pending = Box::new(Pending {
        circle: path::Circle {
            radius: 55.,
            ..Default::default()
        },
        prepared: None,
    });
    app.prepare_circle_preview(&ctx, &mut pending)?;
    app.text_editor.paths.pending = Some(pending);
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.paths.confirm.ok_or("Circle confirm")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    let e = app.text_editor.draft.as_ref().unwrap();
    let mut restored = e.settings.clone();
    restored.style.circle = None;
    if e.settings.style.circle.is_none()
        || restored != regular
        || app.documents[0].snapshot() != original
    {
        return Err(
            "Confirmed circle conversion lost hidden formatting or mutated source before Apply"
                .into(),
        );
    }
    app.preview_text(&ctx)?;
    app.apply_text()?;
    if app.documents[0].history_counts().0 != counts.0 + 1 || app.text_output_guard(0).is_ok() {
        return Err("Circular Apply Undo/output loss guard".into());
    }
    let file = root.join("circle-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    let opened =
        drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let mut restored = core::Settings::from_object(&opened.scene.objects[0]).unwrap();
    restored.style.circle = None;
    if restored != regular {
        return Err("UI native roundtrip lost regular formatting".into());
    }
    app.begin_text_output_review();
    settle(&ctx, &mut app, &mut time);
    let r = app
        .text_editor
        .paths
        .output_ack
        .ok_or("Output review acknowledgement")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_output_guard(0).is_err() {
        return Err("Output review did not acknowledge the captured source".into());
    }
    if !app
        .workspaces
        .store
        .current
        .font_lists
        .recent
        .contains(&Font::Ubuntu)
    {
        return Err("Applied font missing from recent list".into());
    }
    let saved = drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
        .map_err(|e| e.to_string())?
        .ok_or("Saved recent settings missing")?;
    if saved.current.font_lists.recent != app.workspaces.store.current.font_lists.recent {
        return Err("Recent font settings restart".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Circular UI document Undo".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn owned_font_and_circle_ui_preview_cancel_confirm_native_loss_guard_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
