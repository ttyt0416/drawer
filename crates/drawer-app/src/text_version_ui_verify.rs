use super::*;
use crate::text_editor::verify::{click, settle};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::text_version_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.documents
        .push(drawer_core::text_version_verify::fixture()?);
    app.active = Some(0);
    let id = app.documents[0].scene.catalog.objects.id(0);
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(id)),
    );
    let original = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let mut time = 0.;
    app.execute_command(&ctx, "text-layout-convert");
    settle(&ctx, &mut app, &mut time);
    let r = app
        .text_editor
        .version
        .prepare
        .ok_or("Conversion preview button")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app
        .text_editor
        .version
        .review
        .as_ref()
        .is_none_or(|e| e.images.is_none() || e.prepared.as_ref().is_none_or(|p| p.changed != 1))
    {
        return Err(format!(
            "Conversion before/after missing: {:?}",
            app.text_editor
                .version
                .review
                .as_ref()
                .and_then(|e| e.error.as_ref())
        ));
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != history
        || app.text_save_guard(0).is_ok()
    {
        return Err("Preview mutated original or permitted save".into());
    }
    let r = app.text_editor.version.cancel.unwrap();
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.version.open() || app.documents[0].snapshot() != original {
        return Err("Cancel conversion changed text".into());
    }
    app.execute_command(&ctx, "text-layout-convert");
    settle(&ctx, &mut app, &mut time);
    let mut e = app.text_editor.version.review.take().unwrap();
    e.all = true;
    app.prepare_text_conversion(&ctx, &mut e)?;
    e.ack = true;
    app.text_editor.version.review = Some(e);
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.version.apply.unwrap();
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.version.open()
        || app.documents[0]
            .scene
            .objects
            .iter()
            .any(|o| drawer_core::text_version::generation(o) != Some(2))
    {
        return Err("Apply conversion did not convert all page text".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Conversion UI one Undo".into());
    }
    // Changed-source and unacknowledged previews cannot commit.
    app.execute_command(&ctx, "text-layout-convert");
    let mut e = app.text_editor.version.review.take().unwrap();
    e.revision += 1;
    if app.prepare_text_conversion(&ctx, &mut e).is_ok() {
        return Err("Stale conversion preview accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn text_generation_preview_cancel_apply_and_stale_guards() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
