use super::verify::{click, frame, settle, wait};
use super::*;
use drawer_core::pixel_effects::{DustTarget, Operation as P};
pub(super) fn seed_text(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: &mut f64,
    text: &str,
) -> Result<(), String> {
    settle(ctx, app, time);
    let r = app
        .detail_filter
        .draft
        .as_ref()
        .unwrap()
        .pixel_controls
        .seed_rect
        .ok_or("Seed input missing")?;
    click(ctx, app, time, r);
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frame(
        ctx,
        app,
        time,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Text(text.into()),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
        ],
    );
    settle(ctx, app, time);
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::pixel_effects_verify::verify(root)?;
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
    app.documents[0].commit_state(drawer_core::detail_filter_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for command in [
        "mosaic",
        "generate-noise",
        "remove-specks",
        "adjust-line-width",
    ] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        if app.detail_filter.draft.is_none() {
            return Err("Pixel effect command missing".into());
        }
        if command == "generate-noise" {
            app.detail_filter.draft.as_mut().unwrap().settings.operation =
                Operation::Pixels(P::Noise {
                    amount: 0.2,
                    seed: u64::MAX,
                    monochrome: false,
                });
        }
        settle(&ctx, &mut app, &mut time);
        if command == "generate-noise" {
            seed_text(&ctx, &mut app, &mut time, "18446744073709551614")?;
            if !matches!(app.detail_filter.draft.as_ref().unwrap().settings.operation,Operation::Pixels(P::Noise{seed,..})if seed==u64::MAX-1)
            {
                return Err("64-bit seed text entry was rounded".into());
            }
        }
        let button = app.detail_filter.preview.ok_or("Pixel Preview missing")?;
        click(&ctx, &mut app, &mut time, button);
        wait(&ctx, &mut app, &mut time)?;
        if app.detail_filter.draft.as_ref().is_none_or(|e| {
            e.prepared.is_none()
                || e.images.is_none()
                || command == "remove-specks" && e.candidates.is_none()
        }) || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Pixel mouse preview/source isolation: {:?}",
                app.status
            ));
        }
        let file = root.join("unconfirmed-pixel.drawer");
        if app.save_to(0, &file).is_ok() || file.exists() {
            return Err("Unconfirmed pixel save did not prompt resolution".into());
        }
        settle(&ctx, &mut app, &mut time);
        let button = app.detail_filter.apply.ok_or("Pixel Apply missing")?;
        click(&ctx, &mut app, &mut time, button);
        if app.detail_filter.draft.is_some()
            || app.documents[0].snapshot() == original
            || app.documents[0].history_counts().0 != counts.0 + 1
        {
            return Err(format!("Pixel mouse Apply/one Undo: {:?}", app.status));
        }
        let file = root.join("applied-pixel.drawer");
        app.save_to(0, &file)?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Pixel options/native/Undo".into());
        }
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.begin_detail_filter(Operation::Pixels(P::defaults()[1]))?;
    seed_text(&ctx, &mut app, &mut time, "18446744073709551616")?;
    if app
        .detail_filter
        .draft
        .as_ref()
        .unwrap()
        .pixel_controls
        .valid
        || app.preview_detail_filter(&ctx).is_ok()
        || app.apply_detail_filter().is_ok()
    {
        return Err("Invalid seed reused an old valid value".into());
    }
    app.detail_filter.draft = None;
    for op in [
        P::Mosaic {
            cell: 1,
            origin: [0; 2],
        },
        P::Noise {
            amount: 0.,
            seed: 99,
            monochrome: false,
        },
        P::Dust {
            max_area: 0,
            threshold: 0.25,
            target: DustTarget::Dark,
        },
        P::LineWidth { delta: 0 },
    ] {
        app.begin_detail_filter(Operation::Pixels(op))?;
        app.preview_detail_filter(&ctx)?;
        wait(&ctx, &mut app, &mut time)?;
        app.apply_detail_filter()?;
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Pixel identity changed dirty/Redo".into());
    }
    let before = drawer_core::detail_filter_verify::fixture()?;
    let p = drawer_core::pixel_effects::prepare(
        &before,
        0,
        P::defaults()[2],
        false,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    let map = crate::pixel_effects::candidate_image(&before, &p, 0, &AtomicBool::new(false))?;
    if map
        .pixels
        .iter()
        .filter(|p| **p == egui::Color32::RED)
        .count()
        != 2
    {
        return Err("Speck candidate map omitted/added changed points".into());
    }
    if crate::pixel_effects::candidate_image(&before, &p, 0, &AtomicBool::new(true)).is_ok() {
        return Err("Candidate map ignored cancellation".into());
    }
    app.begin_detail_filter(Operation::Pixels(P::defaults()[0]))?;
    app.preview_detail_filter(&ctx)?;
    wait(&ctx, &mut app, &mut time)?;
    app.detail_filter.draft.as_mut().unwrap().settings.operation = Operation::Pixels(P::Mosaic {
        cell: 0,
        origin: [0; 2],
    });
    if app.preview_detail_filter(&ctx).is_ok()
        || app.detail_filter.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_detail_filter().is_ok()
    {
        return Err("Invalid pixel preview retained old plan".into());
    }
    app.begin_detail_filter(Operation::Pixels(P::defaults()[1]))?;
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
        return Err("Pixel Escape committed source".into());
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
    app.begin_detail_filter(Operation::Pixels(P::defaults()[3]))?;
    app.preview_detail_filter(&ctx)?;
    app.documents[0].current_frame += 1;
    wait(&ctx, &mut app, &mut time)?;
    if app.detail_filter.draft.as_ref().unwrap().prepared.is_some() {
        return Err("Stale pixel frame result accepted".into());
    }
    app.documents[0].current_frame -= 1;
    app.detail_filter.draft = None;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pixel_mouse_commands_candidate_map_identity_options_native_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
