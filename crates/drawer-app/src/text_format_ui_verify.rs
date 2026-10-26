use super::*;
use crate::text_editor::verify::{click, frame, settle};
fn select(ctx: &egui::Context, e: &Draft, a: usize, b: usize) {
    let id = composition::widget_id(e);
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(a),
            egui::text::CCursor::new(b),
        )));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::text_format_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.storage = Some(UserStorage::at(root.join("mixed-ui-user")).map_err(|e| e.to_string())?);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 240,
        height: 180,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.execute_command(&ctx, "text-editor");
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Text("a\u{301}한글".into())],
    );
    let toggle = app.text_editor.format.toggle.ok_or("Formatting toggle")?;
    click(&ctx, &mut app, &mut time, toggle);
    settle(&ctx, &mut app, &mut time);
    let enable = app.text_editor.format.enable.ok_or("Enable formatting")?;
    click(&ctx, &mut app, &mut time, enable);
    settle(&ctx, &mut app, &mut time);
    app.text_editor.format.font = Font::Ubuntu;
    app.text_editor.format.color = [220, 30, 40, 255];
    app.text_editor.format.border = true;
    app.text_editor.format.edge = [0, 150, 0, 255];
    app.text_editor.format.width = 3.;
    app.text_editor.format.first = 1;
    app.text_editor.format.last = 1;
    settle(&ctx, &mut app, &mut time);
    let apply = app
        .text_editor
        .format
        .apply
        .ok_or("Apply range formatting")?;
    click(&ctx, &mut app, &mut time, apply);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.draft.as_ref().is_none_or(|e| {
        e.settings
            .style
            .rich
            .as_ref()
            .is_none_or(|r| r.spans.len() != 1 || r.spans[0].start != 0 || r.spans[0].end != 2)
    }) {
        return Err("Format control split a combining grapheme or did not apply".into());
    }
    let style = app
        .text_editor
        .draft
        .as_ref()
        .unwrap()
        .settings
        .style
        .rich
        .clone();
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 2, 2);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "ㅎ".into(),
            active_range_chars: None,
        })],
    );
    app.text_editor.format.color = [0, 0, 255, 255];
    let apply = app.text_editor.format.apply.unwrap();
    click(&ctx, &mut app, &mut time, apply);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.style.rich != style)
    {
        return Err("Formatting changed during IME composition".into());
    }
    if app
        .text_editor
        .draft
        .as_ref()
        .unwrap()
        .composition
        .is_some()
    {
        frame(
            &ctx,
            &mut app,
            &mut time,
            vec![key(egui::Key::Escape, Default::default())],
        );
    }
    let mapping = Mapping::default();
    preset(
        &mut app,
        Action::Create("혼합 이름".into(), mapping.clone()),
    )?;
    if preset(
        &mut app,
        Action::Create("혼합 이름".into(), mapping.clone()),
    )
    .is_ok()
    {
        return Err("Duplicate local mixed preset accepted".into());
    }
    app.text_editor
        .draft
        .as_mut()
        .unwrap()
        .settings
        .style
        .rich
        .as_mut()
        .unwrap()
        .mapping = Some(mapping.clone());
    let frozen = app.text_editor.draft.as_ref().unwrap().settings.clone();
    let mut changed = mapping;
    changed.base = Font::Hack;
    preset(
        &mut app,
        Action::Update("혼합 이름".into(), "이름 수정".into(), changed.clone()),
    )?;
    preset(&mut app, Action::Delete("이름 수정".into()))?;
    if app.text_editor.draft.as_ref().unwrap().settings != frozen
        || app.documents[0].snapshot() != original
    {
        return Err("Preset edit/delete modified pending text or original document".into());
    }
    app.text_editor
        .draft
        .as_mut()
        .unwrap()
        .settings
        .style
        .rich
        .as_mut()
        .unwrap()
        .mapping = Some(changed);
    settle(&ctx, &mut app, &mut time);
    let preview = app.text_editor.preview.unwrap();
    click(&ctx, &mut app, &mut time, preview);
    settle(&ctx, &mut app, &mut time);
    let counts = app.documents[0].history_counts();
    let apply = app.text_editor.apply.unwrap();
    click(&ctx, &mut app, &mut time, apply);
    if app.text_editor.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1 {
        return Err(format!("Mixed text document Apply/Undo: {:?}", app.status));
    }
    let saved = app.documents[0].snapshot();
    let path = root.join("mixed-text-ui.drawer");
    app.save_to(0, &path)?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != saved
    {
        return Err("Mixed text native UI reopen".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Mixed text one Undo".into());
    }
    Ok(())
}
pub(super) fn editing() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 160,
        height: 100,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
    app.execute_command(&ctx, "text-editor");
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
    let original = format!("a\u{301}{family}🇰🇷Z");
    {
        let e = app.text_editor.draft.as_mut().unwrap();
        e.settings.text = original.clone();
        e.settings.style.rich = Some(Box::default());
    }
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 3, 3);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::Delete, Default::default())],
    );
    if app.text_editor.draft.as_ref().unwrap().settings.text != "a\u{301}🇰🇷Z" {
        return Err("Delete split an emoji ZWJ grapheme".into());
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
            key(egui::Key::Z, modifiers),
        ],
    );
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::ModifiersChanged(Default::default())],
    );
    if app.text_editor.draft.as_ref().unwrap().settings.text != original {
        return Err("Grapheme delete Undo lost original Unicode".into());
    }
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 9, 9);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::ArrowLeft, Default::default())],
    );
    let e = app.text_editor.draft.as_ref().unwrap();
    let cursor = egui::text_edit::TextEditState::load(&ctx, composition::widget_id(e))
        .and_then(|s| s.cursor.char_range())
        .unwrap();
    if usize::from(cursor.primary.index) != 2 {
        return Err("Arrow movement split a ZWJ sequence".into());
    }
    {
        let e = app.text_editor.draft.as_mut().unwrap();
        e.settings.text = "Z".repeat(4095);
        e.settings.style.rich = Some(Box::default());
    }
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 4095, 4095);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Paste(family.into())],
    );
    if app.text_editor.draft.as_ref().unwrap().settings.text != "Z".repeat(4095)
        || app.text_editor.format.error.is_none()
    {
        return Err("Length limit silently truncated a pasted emoji".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn text_format_widgets_ime_presets_native_and_document_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
    #[test]
    fn text_format_grapheme_delete_arrow_undo_and_paste_limit() {
        super::editing().unwrap();
    }
}
