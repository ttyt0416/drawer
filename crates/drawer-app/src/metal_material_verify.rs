use super::*;
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn settle(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    for _ in 0..3 {
        step(ctx, app, time, vec![]);
    }
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, r: egui::Rect) {
    for pressed in [true, false] {
        step(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(r.center()),
                egui::Event::PointerButton {
                    pos: r.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::metal_material_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("metallic-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let texture = drawer_core::metal_material_verify::texture();
    let image = drawer_core::references::LocalImage {
        width: 2,
        height: 2,
        pixels: texture.pixels.as_ref().clone(),
    };
    let image_doc = image
        .document(DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let path = root.join("user-metal-texture.png");
    drawer_core::png_export::export(&image_doc, &path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let mut time = 0.;
    app.execute_command(&ctx, "metallic-material");
    check(
        app.material_layer
            .draft
            .as_ref()
            .is_some_and(|e| matches!(e.layer.source, Source::Metal(_))),
        "Metal command missing",
    )?;
    app.material_layer.draft.as_mut().unwrap().path = path.to_string_lossy().into_owned();
    settle(&ctx, &mut app, &mut time);
    let r = app.material_layer.load.ok_or("Metal load missing")?;
    click(&ctx, &mut app, &mut time, r);
    if let Source::Metal(m) = &mut app.material_layer.draft.as_mut().unwrap().layer.source {
        check(m.texture.is_some(), "Metal load replaced the material type")?;
        m.strength = 0.;
    }
    settle(&ctx, &mut app, &mut time);
    let r = app.material_layer.preview.ok_or("Metal preview missing")?;
    click(&ctx, &mut app, &mut time, r);
    check(
        app.material_layer.draft.as_ref().unwrap().state.is_some()
            && app.documents[0].snapshot() == original
            && app.documents[0].history_counts() == counts,
        "Metal preview changed original/history",
    )?;
    settle(&ctx, &mut app, &mut time);
    let r = app.material_layer.apply.ok_or("Metal Apply missing")?;
    click(&ctx, &mut app, &mut time, r);
    check(
        app.material_layer.draft.is_none()
            && app.documents[0].history_counts().0 == counts.0 + 1
            && app.documents[0].layers == original.layers,
        "Metal Apply/one Undo/source preservation",
    )?;
    check(
        drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5, 0.5])
            == [64, 64, 64, 255],
        "Strength zero rendered incorrectly",
    )?;
    let saved = app.documents[0].snapshot();
    let native = root.join("metallic-app.drawer");
    app.save_to(0, &native)?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    check(
        drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved,
        "Metal depended on deleted external texture",
    )?;
    app.execute_command(&ctx, "material-layer-edit");
    let e = app
        .material_layer
        .draft
        .as_mut()
        .ok_or("Metal material edit unavailable")?;
    if let Source::Metal(m) = &mut e.layer.source {
        m.strength = 1.;
        m.gloss = 0.9;
        m.direction = 90.;
    } else {
        return Err("Saved metal lost editable parameters".into());
    }
    app.preview_material_layer(&ctx)?;
    app.apply_material_layer()?;
    check(
        app.documents[0].snapshot() != saved && app.documents[0].history_counts().0 == counts.0 + 2,
        "Metal edit was not one command",
    )?;
    check(
        app.documents[0].undo() && app.documents[0].snapshot() == saved,
        "Metal parameter edit Undo mismatch",
    )?;
    app.begin_material_layer(true)?;
    app.material_layer.draft.as_mut().unwrap().path = path.to_string_lossy().into_owned();
    settle(&ctx, &mut app, &mut time);
    let r = app
        .material_layer
        .load
        .ok_or("Metal missing-texture load unavailable")?;
    click(&ctx, &mut app, &mut time, r);
    check(
        app.status
            .as_ref()
            .is_some_and(|s| s.contains("procedural foil"))
            && matches!(&app.material_layer.draft.as_ref().unwrap().layer.source,Source::Metal(m)if m.texture.is_none()),
        "Missing user texture did not select explicit procedural fallback",
    )?;
    app.preview_material_layer(&ctx)?;
    check(
        app.documents[0].snapshot() == saved,
        "Fallback preview mutated document",
    )?;
    settle(&ctx, &mut app, &mut time);
    let r = app
        .material_layer
        .fallback
        .ok_or("Fallback control missing")?;
    click(&ctx, &mut app, &mut time, r);
    check(
        app.material_layer.draft.as_ref().unwrap().state.is_none(),
        "Changing fallback retained stale material preview",
    )?;
    step(
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
    check(
        app.material_layer.draft.is_none() && app.documents[0].snapshot() == saved,
        "Metal Escape changed document",
    )?;
    app.begin_material_layer(true)?;
    settle(&ctx, &mut app, &mut time);
    let r = app.material_layer.metal.ok_or("Metal selector missing")?;
    click(&ctx, &mut app, &mut time, r);
    app.preview_material_layer(&ctx)?;
    app.documents[0].current_frame += 1;
    check(
        app.apply_material_layer().is_err(),
        "Stale metallic preview applied",
    )?;
    app.documents[0].current_frame -= 1;
    app.material_layer.draft = None;
    check(
        app.documents[0].undo() && app.documents[0].snapshot() == original,
        "Original metal creation Undo mismatch",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_metallic_controls_texture_load_preview_edit_fallback_cancel_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
