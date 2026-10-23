use super::*;
type Undo = egui::util::undoer::Undoer<(egui::text::CCursorRange, String)>;
pub(super) struct Composition {
    value: String,
    state: egui::text_edit::TextEditState,
    undo: Undo,
}
pub(super) struct Change {
    base: Composition,
    active: bool,
}
pub(super) fn widget_id(e: &Draft) -> egui::Id {
    egui::Id::new(("drawer-text-content", e.document, e.id))
}
pub(super) fn cancel(ctx: &egui::Context, e: &mut Draft) {
    if let Some(mut c) = e.composition.take() {
        e.settings.text = c.value;
        c.state.set_undoer(c.undo);
        c.state.store(ctx, widget_id(e));
    }
}
fn cursor(c: &Composition) -> egui::text::CCursorRange {
    c.state.cursor.char_range().unwrap_or_else(|| {
        egui::text::CCursorRange::one(egui::text::CCursor::new(c.value.chars().count()))
    })
}
fn commit(c: &mut Composition, text: &str) {
    let before = cursor(c);
    c.undo.add_undo(&(before, c.value.clone()));
    let count = c.value.chars().count();
    let start = usize::from(before.primary.index)
        .min(usize::from(before.secondary.index))
        .min(count);
    let end = usize::from(before.primary.index)
        .max(usize::from(before.secondary.index))
        .min(count);
    let a = c
        .value
        .char_indices()
        .nth(start)
        .map_or(c.value.len(), |(n, _)| n);
    let b = c
        .value
        .char_indices()
        .nth(end)
        .map_or(c.value.len(), |(n, _)| n);
    c.value.replace_range(a..b, text);
    let after =
        egui::text::CCursorRange::one(egui::text::CCursor::new(start + text.chars().count()));
    c.state.cursor.set_char_range(Some(after));
}
pub(super) fn before(ctx: &egui::Context, e: &mut Draft) -> (Option<Change>, bool) {
    let id = widget_id(e);
    let focused = ctx.memory(|m| m.has_focus(id));
    let events = ctx.input(|i| i.events.clone());
    if e.composition.is_some()
        && ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|v| matches!(v, egui::Event::WindowFocused(false)))
        })
    {
        let mut base = *e.composition.take().unwrap();
        if let Some(end) = events
            .iter()
            .position(|v| matches!(v, egui::Event::WindowFocused(false)))
        {
            for event in &events[..end] {
                if let egui::Event::Ime(egui::ImeEvent::Commit(text)) = event
                    && !text.is_empty()
                    && text != "\n"
                    && text != "\r"
                {
                    commit(&mut base, text);
                }
            }
        }
        e.settings.text = base.value;
        base.state.set_undoer(base.undo);
        base.state.store(ctx, id);
        ctx.input_mut(|i| i.events.retain(|v| !matches!(v, egui::Event::Ime(_))));
        return (None, false);
    }

    let cancel_key = e.composition.is_some()
        && ctx.input(|i| {
            i.key_pressed(egui::Key::Escape) || i.modifiers.command && i.key_pressed(egui::Key::Z)
        });
    if cancel_key {
        cancel(ctx, e);
        ctx.input_mut(|i| {
            i.events.retain(|v| {
                !matches!(
                    v,
                    egui::Event::Key {
                        key: egui::Key::Escape | egui::Key::Z,
                        ..
                    }
                )
            })
        });
        ctx.memory_mut(|m| m.request_focus(id));
        return (None, true);
    }
    if !focused
        || !events.iter().any(|v| {
            matches!(
                v,
                egui::Event::Ime(egui::ImeEvent::Preedit { .. } | egui::ImeEvent::Commit(_))
            )
        })
    {
        return (None, false);
    }
    let mut active = e.composition.is_some();
    let mut base = e.composition.take().map(|c| *c).unwrap_or_else(|| {
        let state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let undo = state.undoer();
        Composition {
            value: e.settings.text.clone(),
            state,
            undo,
        }
    });
    for event in events {
        match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => active = !text.is_empty(),
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                if text == "\n" || text == "\r" {
                    continue;
                }
                if !text.is_empty() {
                    commit(&mut base, &text);
                }
                active = false;
            }
            _ => {}
        }
    }
    (Some(Change { base, active }), false)
}
pub(super) fn after(
    ctx: &egui::Context,
    e: &mut Draft,
    output: &mut egui::text_edit::TextEditOutput,
    change: Option<Change>,
) {
    if let Some(Change { mut base, active }) = change {
        if active {
            output.state.set_undoer(base.undo.clone());
            output.state.clone().store(ctx, widget_id(e));
            e.composition = Some(Box::new(base));
        } else {
            e.settings.text = base.value;
            base.state.set_undoer(base.undo);
            base.state.store(ctx, widget_id(e));
        }
    } else if let Some(c) = &e.composition {
        if output.response.has_focus() {
            output.state.set_undoer(c.undo.clone());
            output.state.clone().store(ctx, widget_id(e));
        } else {
            cancel(ctx, e);
        }
    }
}
