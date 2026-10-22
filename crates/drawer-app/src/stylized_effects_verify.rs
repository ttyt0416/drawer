use super::verify::{click, frame, settle, wait};
use super::*;
use drawer_core::{pixel_effects::Operation as P, stylized_effects::Operation as S};
fn op(style: S) -> Operation {
    Operation::Pixels(P::Style(style))
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::stylized_effects_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 5,
        height: 5,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::stylized_effects_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for command in ["chromatic-aberration", "film-effect", "pencil-effect"] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.detail_filter.draft.is_none() {
            return Err("Style command missing".into());
        }
        settle(&ctx, &mut app, &mut time);
        if command == "film-effect" {
            super::pixel_verify::seed_text(&ctx, &mut app, &mut time, "18446744073709551614")?;
            if !matches!(app.detail_filter.draft.as_ref().unwrap().settings.operation,Operation::Pixels(P::Style(S::Film{seed,..}))if seed==u64::MAX-1)
            {
                return Err("Film seed input was rounded or did not reach the effect".into());
            }
        }
        let button = app.detail_filter.preview.ok_or("Style Preview missing")?;
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
                "Style mouse preview/source isolation: {:?}",
                app.status
            ));
        }
        let pending = root.join("unconfirmed-style.drawer");
        if app.save_to(0, &pending).is_ok() || pending.exists() {
            return Err("Unconfirmed style save did not prompt resolution".into());
        }
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.apply.ok_or("Style Apply missing")?;
        click(&ctx, &mut app, &mut time, button);
        if app.detail_filter.draft.is_some()
            || app.documents[0].snapshot() == original
            || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Style mouse Apply/one Undo: {:?}", app.status));
        }
        let file = root.join("applied-style.drawer");
        app.save_to(0, &file)?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Style settings/seed/native/Undo".into());
        }
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    for style in S::defaults() {
        app.begin_detail_filter(op(drawer_core::stylized_effects_verify::zero(style)))?;
        app.preview_detail_filter(&ctx)?;
        wait(&ctx, &mut app, &mut time)?;
        app.apply_detail_filter()?;
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Style strength0 changed dirty/Redo".into());
    }
    app.begin_detail_filter(op(S::defaults()[0]))?;
    app.preview_detail_filter(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.detail_filter.draft.as_mut().unwrap().settings.operation = op(S::Aberration {
        offset: [33, 0],
        strength: 1.,
    });
    if app.preview_detail_filter(&ctx).is_ok()
        || app.detail_filter.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_detail_filter().is_ok()
    {
        return Err("Failed style preview retained an old plan".into());
    }
    app.begin_detail_filter(op(S::defaults()[1]))?;
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
        return Err("Style Escape committed source".into());
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
    app.begin_detail_filter(op(S::defaults()[2]))?;
    app.preview_detail_filter(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.detail_filter.draft.as_ref().unwrap().prepared.is_some() {
        return Err("Stale style frame result accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.detail_filter.draft = None;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn style_mouse_preview_apply_exact_seed_native_zero_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
