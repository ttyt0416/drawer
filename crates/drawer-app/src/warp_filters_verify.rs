use super::verify::{click, frame, settle, wait};
use super::*;
use drawer_core::{pixel_effects::Operation as P, warp_filters::Operation as W};
fn op(w: W) -> Operation {
    Operation::Pixels(P::Warp(w))
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::warp_filters_verify::verify(root)?;
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
    app.documents[0].commit_state(drawer_core::warp_filters_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for command in [
        "fisheye-filter",
        "panorama-filter",
        "wave-filter",
        "swirl-filter",
    ] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.detail_filter.draft.is_none() {
            return Err("Distortion command missing".into());
        }
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.preview.ok_or("Warp Preview missing")?;
        click(&ctx, &mut app, &mut time, button);
        wait(&ctx, &mut app, &mut time)?;
        if app
            .detail_filter
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
            || app.documents[0].snapshot() != original
        {
            return Err(format!(
                "Distortion preview source isolation: {:?}",
                app.status
            ));
        }
        let pending = root.join("pending-distortion.drawer");
        if app.save_to(0, &pending).is_ok() || pending.exists() {
            return Err("Unconfirmed distortion save must require resolution".into());
        }
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.apply.ok_or("Warp Apply missing")?;
        click(&ctx, &mut app, &mut time, button);
        if app.detail_filter.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!(
                "Distortion mouse Apply / one Undo: {:?}",
                app.status
            ));
        }
        let path = root.join("distortion-app.drawer");
        app.save_to(0, &path)?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Distortion app native/Undo".into());
        }
    }
    let counts = app.documents[0].history_counts();
    for w in W::defaults() {
        app.begin_detail_filter(op(drawer_core::warp_filters_verify::zero(w)))?;
        app.preview_detail_filter(&ctx)?;
        wait(&ctx, &mut app, &mut time)?;
        app.apply_detail_filter()?;
    }
    if app.documents[0].snapshot() != original || app.documents[0].history_counts() != counts {
        return Err("Zero distortions changed history".into());
    }
    app.begin_detail_filter(op(W::defaults()[3]))?;
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
        return Err("Distortion Escape changed source".into());
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
    app.begin_detail_filter(op(W::defaults()[0]))?;
    app.preview_detail_filter(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.documents[0].current_frame += 1;
    if app.apply_detail_filter().is_ok() {
        return Err("Stale distortion accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.detail_filter.draft.as_mut().unwrap().settings.operation = op(W::Panorama {
        field_of_view: 180.,
        center: None,
    });
    if app.preview_detail_filter(&ctx).is_ok() || app.apply_detail_filter().is_ok() {
        return Err("Failed warp retained previous plan".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_mouse_preview_apply_cancel_stale_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
