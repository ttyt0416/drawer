use super::*;
use crate::text_editor::verify::{click, frame, settle};
fn select(ctx: &egui::Context, e: &Draft, start: usize, end: usize) {
    let id = composition::widget_id(e);
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(start),
            egui::text::CCursor::new(end),
        )));
    state.store(ctx, id);
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
    drawer_core::text_reading_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
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
    let field = app.text_editor.text.ok_or("Reading body field")?;
    click(&ctx, &mut app, &mut time, field);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Text("한글 abc".into())],
    );
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 0, 2);
    settle(&ctx, &mut app, &mut time);
    let add = app.text_editor.add_reading.ok_or("Add reading control")?;
    click(&ctx, &mut app, &mut time, add);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.draft.as_ref().is_none_or(|e| {
        e.settings.style.ruby.len() != 1
            || e.settings.style.ruby[0].start != 0
            || e.settings.style.ruby[0].end != 2
    }) {
        return Err("Reading did not use body selection".into());
    }
    let reading = app
        .text_editor
        .reading_text
        .ok_or("Reading input missing")?;
    click(&ctx, &mut app, &mut time, reading);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Text("독음 ".into())],
    );
    let preedit = |text: &str| {
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: text.into(),
            active_range_chars: None,
        })
    };
    frame(&ctx, &mut app, &mut time, vec![preedit("읽")]);
    if app.preview_text(&ctx).is_ok() {
        return Err("Reading preedit was applied".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("읽".into()))],
    );
    frame(&ctx, &mut app, &mut time, vec![preedit("ㄱ")]);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::Escape, Default::default())],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings.style.ruby[0].text != "독음 읽")
    {
        return Err(format!(
            "Ruby IME commit/cancel: {:?}",
            app.text_editor
                .draft
                .as_ref()
                .map(|e| &e.settings.style.ruby)
        ));
    }
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 0, 2);
    frame(&ctx, &mut app, &mut time, vec![preedit("ㅎ")]);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::Escape, Default::default())],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.text != "한글 abc" || e.settings.style.ruby[0].end != 2)
    {
        return Err("Body IME cancel failed reading mapping restore".into());
    }
    select(&ctx, app.text_editor.draft.as_ref().unwrap(), 0, 2);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::Delete, Default::default())],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| !e.settings.style.ruby.is_empty())
    {
        return Err("Body delete did not remove reading".into());
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
    if app.text_editor.draft.as_ref().is_none_or(|e| {
        e.settings.text != "한글 abc"
            || e.settings.style.ruby.len() != 1
            || e.settings.style.ruby[0].text != "독음 읽"
    }) {
        return Err(format!(
            "Field Undo failed reading relationship restoration: {:?}",
            app.text_editor.draft.as_ref().map(|e| (
                &e.settings.text,
                &e.settings.style.ruby,
                &e.reading_history
            ))
        ));
    }
    settle(&ctx, &mut app, &mut time);
    let vertical = app.text_editor.vertical.ok_or("Vertical control")?;
    click(&ctx, &mut app, &mut time, vertical);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.settings.style.flow != core::Flow::Vertical)
    {
        return Err("Vertical mouse control".into());
    }
    app.text_editor
        .draft
        .as_mut()
        .unwrap()
        .settings
        .style
        .tracking = 0.1;
    settle(&ctx, &mut app, &mut time);
    let preview = app.text_editor.preview.unwrap();
    click(&ctx, &mut app, &mut time, preview);
    settle(&ctx, &mut app, &mut time);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none())
        || app.documents[0].snapshot() != original
    {
        return Err(format!("Ruby preview/source: {:?}", app.status));
    }
    let counts = app.documents[0].history_counts();
    let apply = app.text_editor.apply.unwrap();
    click(&ctx, &mut app, &mut time, apply);
    if app.text_editor.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1 {
        return Err(format!("Ruby mouse Apply/one Undo: {:?}", app.status));
    }
    let saved = app.documents[0].snapshot();
    let file = root.join("reading-ui.drawer");
    app.save_to(0, &file)?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != saved
    {
        return Err("Ruby UI native roundtrip".into());
    }
    app.execute_command(&ctx, "text-editor");
    app.text_editor
        .draft
        .as_mut()
        .unwrap()
        .settings
        .style
        .ruby
        .clear();
    settle(&ctx, &mut app, &mut time);
    let cancel = app.text_editor.cancel.unwrap();
    click(&ctx, &mut app, &mut time, cancel);
    if app.documents[0].snapshot() != saved {
        return Err("Cancel discarded original reading".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Ruby Apply document Undo".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn text_reading_body_range_ruby_ime_field_undo_vertical_mouse_native_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
