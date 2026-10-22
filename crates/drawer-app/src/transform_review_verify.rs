use super::*;
fn frame(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn settle(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    for _ in 0..3 {
        frame(ctx, app, time, vec![]);
    }
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, rect: egui::Rect) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
fn begin(app: &mut Drawer, ctx: &egui::Context) -> Result<(), String> {
    app.begin_affine_transform()?;
    let e = app.affine_transform.draft.as_mut().unwrap();
    e.pixels = true;
    e.settings.scale = [1.3, 0.8];
    e.settings.translation = [3., 2.];
    app.preview_affine_transform(ctx)
}
fn aspect_mouse() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut scale = [1., 2.];
    let mut locked = true;
    let mut time = 0.;
    let mut step = |events: Vec<egui::Event>| {
        time += 0.1;
        let mut rects = ([egui::Rect::NOTHING; 2], egui::Rect::NOTHING);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600., 300.),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let (_, axes, lock) = scales(ui, &mut scale, &mut locked, -1e4..=1e4);
            rects = (axes, lock);
        });
        output.textures_delta.clear();
        (rects, scale, locked)
    };
    let (rects, _, _) = step(vec![]);
    let p = rects.0[0].center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    step(vec![egui::Event::PointerMoved(p), button(p, true)]);
    step(vec![egui::Event::PointerMoved(p + egui::vec2(60., 0.))]);
    let (rects, changed, locked) = step(vec![button(p + egui::vec2(60., 0.), false)]);
    if !locked || changed[0] <= 1. || (changed[1] / changed[0] - 2.).abs() > 1e-10 {
        return Err("Mouse scale drag did not preserve locked aspect".into());
    }
    let lock = rects.1.center();
    step(vec![egui::Event::PointerMoved(lock), button(lock, true)]);
    let (rects, before, locked) = step(vec![button(lock, false)]);
    if locked {
        return Err("Aspect checkbox did not unlock".into());
    }
    let p = rects.0[0].center();
    step(vec![egui::Event::PointerMoved(p), button(p, true)]);
    step(vec![egui::Event::PointerMoved(p + egui::vec2(60., 0.))]);
    let (_, after, _) = step(vec![button(p + egui::vec2(60., 0.), false)]);
    if after[0] <= before[0] || after[1] != before[1] {
        return Err("Unlocked mouse scale changed the other axis".into());
    }
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    aspect_mouse()?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 128,
        height: 96,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for (y, row) in app.documents[0].layers[0]
        .pixels
        .chunks_mut(128)
        .enumerate()
    {
        for (x, p) in row.iter_mut().enumerate() {
            *p = [(x * 2) as u8, (y * 2) as u8, 30, 255];
        }
    }
    drawer_core::selection_tools::rectangle(&mut app.documents[0], [20., 10.], [70., 60.])?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    begin(&mut app, &ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    let mut result = None;
    for q in [Quality::Fast, Quality::Normal, Quality::Fine] {
        app.affine_transform.draft.as_mut().unwrap().quality = q;
        app.preview_affine_transform(&ctx)?;
        app.wait_transform_preview(&ctx, &mut time)?;
        let e = app.affine_transform.draft.as_ref().unwrap();
        let state = &e.prepared.as_ref().ok_or("Quality preview missing")?.state;
        if result.as_ref().is_some_and(|s| s != state)
            || e.images.as_ref().unwrap()[1].size()
                != [128.min(q.size()) as usize, 96.min(q.size()) as usize]
        {
            return Err(format!(
                "Preview quality must change image resolution only: quality={q:?}, size={:?}, same_state={}",
                e.images.as_ref().unwrap()[1].size(),
                result.as_ref().is_none_or(|s| s == state)
            ));
        }
        result = Some(state.clone());
    }
    let prepared = result.unwrap();
    let path = root.join("transform-keep.drawer");
    if app.save_to(0, &path).is_ok() || path.exists() {
        return Err("Unconfirmed affine save bypassed decision".into());
    }
    settle(&ctx, &mut app, &mut time);
    let r = app
        .transform_review
        .keep
        .ok_or("Keep editing choice missing")?;
    click(&ctx, &mut app, &mut time, r);
    if app.transform_save_pending()
        || app.documents[0].snapshot() != original
        || app
            .affine_transform
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .state
            != prepared
    {
        return Err("Cancel save must retain the entire preview/selection".into());
    }
    if app.save_as_from(0, || None)? || app.documents[0].snapshot() != original {
        return Err("Cancelled filename chooser changed original".into());
    }
    // Native chooser cancellation is resolved before applying/discarding any transform.
    if app.save_as(0)? || !app.transform_save_pending() {
        return Err("Transform Save As did not defer".into());
    }
    app.resolve_transform_save_from(&ctx, true, || None)?;
    if app.transform_save_pending()
        || app
            .affine_transform
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .state
            != prepared
        || app.documents[0].snapshot() != original
    {
        return Err("Cancelled native chooser lost a prepared transform".into());
    }
    let path = root.join("transform-apply.drawer");
    let _ = app.save_to(0, &path);
    settle(&ctx, &mut app, &mut time);
    let r = app
        .transform_review
        .apply
        .ok_or("Apply and save choice missing")?;
    click(&ctx, &mut app, &mut time, r);
    if app.affine_transform.draft.is_some()
        || app.documents[0].snapshot() != prepared
        || app.documents[0].history_counts().0 != counts.0 + 1
        || drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != prepared
    {
        return Err(format!(
            "Apply/save must commit exactly one transform: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Transform Undo did not restore pixels and selection".into());
    }
    begin(&mut app, &ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    let path = root.join("transform-discard.drawer");
    let before_counts = app.documents[0].history_counts();
    let _ = app.save_to(0, &path);
    settle(&ctx, &mut app, &mut time);
    let r = app
        .transform_review
        .discard
        .ok_or("Discard and save choice missing")?;
    click(&ctx, &mut app, &mut time, r);
    if app.affine_transform.draft.is_some()
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != before_counts
        || drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != original
    {
        return Err("Discard/save failed original state/history preservation".into());
    }
    begin(&mut app, &ctx)?;
    app.cancel_transform_work();
    app.affine_transform.draft = None;
    app.wait_transform_preview(&ctx, &mut time)?;
    if app.documents[0].snapshot() != original {
        return Err("Cancelled worker changed original".into());
    }
    begin(&mut app, &ctx)?;
    app.affine_transform
        .draft
        .as_mut()
        .unwrap()
        .settings
        .degrees = 33.;
    app.wait_transform_preview(&ctx, &mut time)?;
    if app
        .affine_transform
        .draft
        .as_ref()
        .unwrap()
        .prepared
        .is_some()
        || app.apply_affine_transform().is_ok()
    {
        return Err("Changed transform options adopted an old worker result".into());
    }
    app.affine_transform.draft = None;
    app.begin_deform(&ctx)?;
    {
        let e = app.deform.draft.as_mut().unwrap();
        e.kind = 1;
        e.depth = 0.3;
    }
    app.preview_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    if app.deform.draft.as_ref().unwrap().prepared.is_none() {
        return Err("Perspective depth preview missing".into());
    }
    let quad = depth(drawer_core::material_mapping::UNIT, 0.3)?;
    if quad == drawer_core::material_mapping::UNIT
        || !Warp::Perspective(quad).valid()
        || depth(drawer_core::material_mapping::UNIT, f64::NAN).is_ok()
    {
        return Err("Depth is not independently validated geometry".into());
    }
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
    app.wait_transform_preview(&ctx, &mut time)?;
    if app.deform.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Depth Escape changed pixels/selection".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn transform_quality_atomic_save_choices_and_cancellable_workers() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
