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
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, r: egui::Rect) {
    for pressed in [true, false] {
        frame(
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
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::text_layout_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 240,
        height: 160,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.execute_command(&ctx, "text-editor");
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.ok_or("Text input missing")?;
    click(&ctx, &mut app, &mut time, field);
    let typed = "한글 123 😀\nsecond line";
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Text(typed.into())],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.text != typed)
        || app.documents[0].snapshot() != original
    {
        return Err("Text typing did not remain an isolated Unicode edit".into());
    }
    let path = root.join("unconfirmed-text.drawer");
    if app.save_to(0, &path).is_ok() || path.exists() {
        return Err("Unconfirmed text save".into());
    }
    let tabs = app.tabs.len();
    app.request_close(0);
    if app.tabs.len() != tabs || app.text_editor.draft.is_none() {
        return Err("Close discarded unconfirmed text".into());
    }
    settle(&ctx, &mut app, &mut time);
    let preview = app.text_editor.preview.ok_or("Text Preview")?;
    click(&ctx, &mut app, &mut time, preview);
    settle(&ctx, &mut app, &mut time);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none() || e.image.is_none())
    {
        return Err(format!("Text preview: {:?}", app.status));
    }
    let count = app.documents[0].history_counts();
    let apply = app.text_editor.apply.ok_or("Text Apply")?;
    click(&ctx, &mut app, &mut time, apply);
    if app.text_editor.draft.is_some()
        || app.documents[0].history_counts().0 != count.0 + 1
        || app.documents[0].layers != original.layers
    {
        return Err(format!("Text Apply/one Undo: {:?}", app.status));
    }
    let file = root.join("text-ui.drawer");
    app.save_to(0, &file)?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Text UI native reopen".into());
    }
    app.execute_command(&ctx, "text-editor");
    let e = app.text_editor.draft.as_mut().ok_or("Edit selected text")?;
    if e.id.is_none() || e.settings.text != typed {
        return Err("Selected text not loaded for editing".into());
    }
    e.settings.text = "discard".into();
    settle(&ctx, &mut app, &mut time);
    let cancel = app.text_editor.cancel.ok_or("Text Cancel")?;
    click(&ctx, &mut app, &mut time, cancel);
    if app.text_editor.draft.is_some()
        || Settings::from_object(&app.documents[0].scene.objects[0]).is_none_or(|s| s.text != typed)
    {
        return Err("Text cancel changed original".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Text UI create Undo".into());
    }
    Ok(())
}
pub(super) fn composition_smoke() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 160,
        height: 120,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.execute_command(&ctx, "text-editor");
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.text.ok_or("IME field")?;
    click(&ctx, &mut app, &mut time, r);
    let preedit = |text: &str| {
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: text.into(),
            active_range_chars: None,
        })
    };
    frame(&ctx, &mut app, &mut time, vec![preedit("ㅎ")]);
    for _ in 0..20 {
        frame(&ctx, &mut app, &mut time, vec![]);
    }
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_none() || e.settings.text != "ㅎ")
        || app.preview_text(&ctx).is_ok()
        || app.documents[0].snapshot() != original
    {
        return Err("IME preedit escaped its temporary state".into());
    }
    frame(&ctx, &mut app, &mut time, vec![preedit("한")]);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![
            egui::Event::Ime(egui::ImeEvent::Commit("한".into())),
            preedit("ㄱ"),
        ],
    );
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("글".into()))],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings.text != "한글")
    {
        return Err("IME commit followed by preedit lost Unicode".into());
    }
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::Z,
                physical_key: Some(egui::Key::Z),
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
    );
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::ModifiersChanged(Default::default())],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.text != "한")
    {
        return Err(format!(
            "IME commit Undo split a composition: {:?}",
            app.text_editor.draft.as_ref().map(|e| &e.settings.text)
        ));
    }
    frame(&ctx, &mut app, &mut time, vec![preedit("ㄱ")]);
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
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings.text != "한")
        || app.documents[0].snapshot() != original
    {
        return Err("Escape failed to cancel only the active IME composition".into());
    }
    app.text_editor.draft = None;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn ime_preedit_commit_units_undo_and_escape_are_local() {
        super::composition_smoke().unwrap();
    }
    #[test]
    fn text_typing_mouse_preview_apply_edit_cancel_close_save_native_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
