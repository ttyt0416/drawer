use super::*;
type Undo = egui::util::undoer::Undoer<(egui::text::CCursorRange, String)>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Body,
    Reading(usize),
}
pub(super) struct Composition {
    field: Field,
    value: String,
    ruby: Vec<core::Ruby>,
    state: egui::text_edit::TextEditState,
    undo: Undo,
}
pub(super) struct Change {
    base: Composition,
    active: bool,
}
pub(super) fn field_id(e: &Draft, field: Field) -> egui::Id {
    match field {
        Field::Body => egui::Id::new(("drawer-text-content", e.document, e.id)),
        Field::Reading(n) => egui::Id::new(("drawer-reading-content", e.document, e.id, n)),
    }
}
pub(super) fn widget_id(e: &Draft) -> egui::Id {
    field_id(e, Field::Body)
}
fn value(e: &Draft, field: Field) -> Option<&String> {
    match field {
        Field::Body => Some(&e.settings.text),
        Field::Reading(n) => e.settings.style.ruby.get(n).map(|r| &r.text),
    }
}
fn restore(ctx: &egui::Context, e: &mut Draft, mut c: Composition) {
    let id = field_id(e, c.field);
    match c.field {
        Field::Body => {
            e.settings.text = c.value;
            e.settings.style.ruby = c.ruby;
        }
        Field::Reading(n) => {
            if let Some(r) = e.settings.style.ruby.get_mut(n) {
                r.text = c.value;
            }
        }
    }
    c.state.set_undoer(c.undo);
    c.state.store(ctx, id);
}
pub(super) fn cancel(ctx: &egui::Context, e: &mut Draft) {
    if let Some(c) = e.composition.take() {
        restore(ctx, e, *c);
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
    let old = c.value.clone();
    c.value.replace_range(a..b, text);
    if c.field == Field::Body {
        core::remap_ruby(&old, &c.value, &mut c.ruby);
    }
    c.state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(start + text.chars().count()),
        )));
}
pub(super) fn before(ctx: &egui::Context, e: &mut Draft) -> (Option<Change>, bool) {
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
        restore(ctx, e, base);
        ctx.input_mut(|i| i.events.retain(|v| !matches!(v, egui::Event::Ime(_))));
        return (None, false);
    }
    if e.composition.is_some()
        && ctx.input(|i| {
            i.key_pressed(egui::Key::Escape) || i.modifiers.command && i.key_pressed(egui::Key::Z)
        })
    {
        let id = field_id(e, e.composition.as_ref().unwrap().field);
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
    let focused = std::iter::once(Field::Body)
        .chain((0..e.settings.style.ruby.len()).map(Field::Reading))
        .find(|f| ctx.memory(|m| m.has_focus(field_id(e, *f))));
    if e.composition
        .as_ref()
        .is_some_and(|c| Some(c.field) != focused)
    {
        cancel(ctx, e);
    }
    let Some(field) = focused else {
        return (None, false);
    };
    if !events.iter().any(|v| {
        matches!(
            v,
            egui::Event::Ime(egui::ImeEvent::Preedit { .. } | egui::ImeEvent::Commit(_))
        )
    }) {
        return (None, false);
    }
    if e.composition.is_none() && field == Field::Body {
        reading_ui::remember(e, e.settings.text.clone(), e.settings.style.ruby.clone());
    }
    let mut active = e.composition.is_some();
    let mut base = e.composition.take().map(|c| *c).unwrap_or_else(|| {
        let state =
            egui::text_edit::TextEditState::load(ctx, field_id(e, field)).unwrap_or_default();
        let undo = state.undoer();
        Composition {
            field,
            value: value(e, field).cloned().unwrap_or_default(),
            ruby: if field == Field::Body {
                e.settings.style.ruby.clone()
            } else {
                Vec::new()
            },
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
    field: Field,
    output: &mut egui::text_edit::TextEditOutput,
    change: &mut Option<Change>,
) -> bool {
    if change.as_ref().is_some_and(|c| c.base.field == field) {
        let Change { base, active } = change.take().unwrap();
        if active {
            if field == Field::Body {
                e.settings.style.ruby = base.ruby.clone();
                core::remap_ruby(&base.value, &e.settings.text, &mut e.settings.style.ruby);
            }
            output.state.set_undoer(base.undo.clone());
            output.state.clone().store(ctx, field_id(e, field));
            e.composition = Some(Box::new(base));
        } else {
            restore(ctx, e, base);
        }
        true
    } else if let Some(c) = &e.composition
        && c.field == field
    {
        if output.response.has_focus() {
            output.state.set_undoer(c.undo.clone());
            output.state.clone().store(ctx, field_id(e, field));
        } else {
            cancel(ctx, e);
        }
        true
    } else {
        false
    }
}

// Selection replacement must not merge into typing that happened before a long IME edit.
pub(super) fn checkpoint(ctx: &egui::Context, e: &Draft, field: Field) {
    let id = field_id(e, field);
    if !ctx.memory(|m| m.has_focus(id)) || e.composition.is_some() {
        return;
    }
    let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) else {
        return;
    };
    let Some(range) = state.cursor.char_range() else {
        return;
    };
    let selected = range.primary.index != range.secondary.index;
    let editing = ctx.input(|i| {
        !i.events.iter().any(|v| matches!(v, egui::Event::Ime(_)))
            && i.events.iter().any(|v| match v {
                egui::Event::Paste(_) | egui::Event::Cut => true,
                egui::Event::Text(t) => selected && !t.is_empty(),
                egui::Event::Key {
                    key: egui::Key::Backspace | egui::Key::Delete,
                    pressed: true,
                    ..
                } => selected,
                _ => false,
            })
    });
    if editing && let Some(text) = value(e, field) {
        let mut undo = state.undoer();
        undo.add_undo(&(range, text.clone()));
        state.set_undoer(undo);
        state.store(ctx, id);
    }
}
