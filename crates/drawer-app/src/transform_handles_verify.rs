use super::*;
use drawer_core::{
    material_mapping::UNIT,
    scene::{Point, Rect},
};
fn frame(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn settle(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    for _ in 0..3 {
        frame(ctx, app, time, vec![]);
    }
}
fn pointer(p: egui::Pos2, down: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(p),
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        },
    ]
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, r: egui::Rect) {
    for down in [true, false] {
        frame(ctx, app, time, pointer(r.center(), down));
    }
}
fn drag(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: &mut f64,
    a: egui::Pos2,
    b: egui::Pos2,
) -> Result<(), String> {
    frame(ctx, app, time, pointer(a, true));
    frame(ctx, app, time, vec![egui::Event::PointerMoved(b)]);
    frame(ctx, app, time, pointer(b, false));
    app.wait_transform_preview(ctx, time)?;
    settle(ctx, app, time);
    Ok(())
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn key(k: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::transform_handles_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.workspaces.store.current.transform_snap = Default::default();
    app.storage =
        Some(UserStorage::at(root.join("transform-handles-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Color([220, 40, 20]),
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let bounds = Rect {
        min: [0.; 2],
        max: [8.; 2],
    };
    let mut time = 0.;
    app.execute_command(&ctx, "scale-rotate");
    settle(&ctx, &mut app, &mut time);
    let point = |app: &Drawer, p: Point| {
        app.affine_transform
            .handles
            .point(app.affine_transform.handles.rect.unwrap(), p)
    };
    let a = point(&app, [8.; 2]);
    let b = point(&app, [9.; 2]);
    drag(&ctx, &mut app, &mut time, a, b)?;
    let e = app
        .affine_transform
        .draft
        .as_ref()
        .ok_or("Affine draft closed during corner drag")?;
    check(
        e.settings.scale.iter().all(|v| (*v - 1.125).abs() < 1e-4) && e.prepared.is_some(),
        &format!(
            "Corner handle/auto preview failed: scale={:?}, {:?}",
            e.settings.scale, app.status
        ),
    )?;
    check(
        app.documents[0].snapshot() == original && app.documents[0].history_counts() == counts,
        "Handle preview modified document",
    )?;
    let matrix = e.settings.matrix()?;
    let a = point(&app, matrix.map([8., 4.]));
    let b = point(
        &app,
        [matrix.map([8., 4.])[0] + 0.5, matrix.map([8., 4.])[1]],
    );
    let old_y = e.settings.scale[1];
    drag(&ctx, &mut app, &mut time, a, b)?;
    check(
        (app.affine_transform.draft.as_ref().unwrap().settings.scale[1] - old_y).abs() < 1e-6,
        "Side handle changed the other axis",
    )?;
    let fit = app
        .affine_transform
        .handles
        .fit_rect
        .ok_or("Fit handles missing")?;
    click(&ctx, &mut app, &mut time, fit);
    settle(&ctx, &mut app, &mut time);
    let matrix = app
        .affine_transform
        .draft
        .as_ref()
        .unwrap()
        .settings
        .matrix()?;
    let old_move = app
        .affine_transform
        .draft
        .as_ref()
        .unwrap()
        .settings
        .translation;
    let a = point(&app, matrix.map([4., 4.]));
    let b = a + egui::vec2(8., 6.);
    drag(&ctx, &mut app, &mut time, a, b)?;
    check(
        app.affine_transform
            .draft
            .as_ref()
            .unwrap()
            .settings
            .translation
            != old_move,
        "Interior move handle failed",
    )?;
    // Clicking the snap option changes only snapping; aspect remains separate.
    app.affine_transform.draft.as_mut().unwrap().aspect = false;
    settle(&ctx, &mut app, &mut time);
    let r = app.affine_transform.snap.ok_or("Snap control missing")?;
    click(&ctx, &mut app, &mut time, r);
    check(
        app.workspaces.store.current.transform_snap.enabled
            && !app.affine_transform.draft.as_ref().unwrap().aspect,
        "Angle snapping changed aspect lock",
    )?;
    let storage = app.storage.as_ref().unwrap();
    check(
        drawer_core::workspaces::Store::load(storage)
            .map_err(|e| e.to_string())?
            .ok_or("Saved transform settings missing")?
            .current
            .transform_snap
            .enabled,
        "Snap option did not persist locally",
    )?;
    // Fit around the current box and rotate from its outer margin.
    let t = app.affine_transform.draft.as_ref().unwrap().settings;
    app.affine_transform.handles.fit(bounds, t);
    settle(&ctx, &mut app, &mut time);
    let center = [0, 1].map(|n| t.pivot[n] + t.translation[n]);
    let a = point(&app, [center[0] + 7., center[1]]);
    let b = point(&app, [center[0] + 6.8, center[1] + 1.4]);
    drag(&ctx, &mut app, &mut time, a, b)?;
    let angle = app
        .affine_transform
        .draft
        .as_ref()
        .unwrap()
        .settings
        .degrees;
    check(
        (angle - 15.).abs() < 1e-8,
        "Rotation handle ignored angle snapping",
    )?;
    // A ready transform can confirm with Enter and is a single document command.
    app.preview_affine_transform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    frame(&ctx, &mut app, &mut time, key(egui::Key::Enter));
    check(
        app.affine_transform.draft.is_none() && app.documents[0].history_counts().0 == counts.0 + 1,
        &format!("Enter did not confirm ready transform: {:?}", app.status),
    )?;
    check(
        app.documents[0].undo() && app.documents[0].snapshot() == original,
        "Handle transform Undo mismatch",
    )?;
    app.begin_affine_transform()?;
    settle(&ctx, &mut app, &mut time);
    frame(&ctx, &mut app, &mut time, key(egui::Key::Escape));
    check(
        app.affine_transform.draft.is_none() && app.documents[0].snapshot() == original,
        "Handle Escape changed document",
    )?;
    app.begin_deform(&ctx)?;
    app.deform
        .draft
        .as_mut()
        .unwrap()
        .offer_quad(UNIT.map(|p| [p[0] + 0.125, p[1]]));
    app.preview_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    settle(&ctx, &mut app, &mut time);
    let before = app
        .deform
        .draft
        .as_ref()
        .unwrap()
        .prepared
        .clone()
        .ok_or("Valid quad preview missing")?;
    let images = app
        .deform
        .draft
        .as_ref()
        .unwrap()
        .images
        .as_ref()
        .unwrap()
        .each_ref()
        .map(egui::TextureHandle::id);
    let pad = app.deform.pad.ok_or("Quad pad missing")?;
    let a = egui::pos2(pad.min.x + pad.width() * 0.125, pad.min.y + 0.5);
    let b = egui::pos2(pad.max.x - 0.5, pad.max.y - 0.5);
    drag(&ctx, &mut app, &mut time, a, b)?;
    let e = app.deform.draft.as_ref().unwrap();
    check(
        e.invalid.is_some()
            && e.prepared.as_ref() == Some(&before)
            && e.images
                .as_ref()
                .unwrap()
                .each_ref()
                .map(egui::TextureHandle::id)
                == images,
        &format!(
            "Invalid quad did not retain last valid preview: error={:?}, quad={:?}, candidate={:?}, prepared={}",
            e.invalid,
            e.quad,
            e.candidate,
            e.prepared.as_ref() == Some(&before)
        ),
    )?;
    check(
        app.apply_deform().is_err() && app.preview_deform(&ctx).is_err(),
        "Invalid quadrilateral could confirm",
    )?;
    check(
        app.deform.draft.as_ref().unwrap().prepared.is_none()
            && app
                .deform
                .draft
                .as_ref()
                .unwrap()
                .images
                .as_ref()
                .unwrap()
                .each_ref()
                .map(egui::TextureHandle::id)
                == images,
        "Failed preview must discard its plan while retaining the last image",
    )?;
    frame(&ctx, &mut app, &mut time, key(egui::Key::Enter));
    check(
        app.deform.draft.is_some() && app.documents[0].snapshot() == original,
        "Enter applied invalid quadrilateral",
    )?;
    app.deform
        .draft
        .as_mut()
        .unwrap()
        .offer_quad(UNIT.map(|p| [p[0] + 0.25, p[1]]));
    app.preview_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.apply_deform()?;
    check(
        matches!(
            app.documents[0].scene.last_transform,
            Some(drawer_core::transform_handles::Record::Quad { .. })
        ),
        "Quad geometry receipt missing",
    )?;
    let native = root.join("quad-handles.drawer");
    app.save_to(0, &native)?;
    check(
        drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == app.documents[0].snapshot(),
        "Quad geometry native mismatch",
    )?;
    check(
        app.documents[0].undo() && app.documents[0].snapshot() == original,
        "Quad confirmation Undo mismatch",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_transform_handle_drags_independent_snap_enter_invalid_quad_retention_cancel_and_undo()
    {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
