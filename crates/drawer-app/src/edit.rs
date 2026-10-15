//! Local text editing: draft values, field undo and clipboard failure preservation.
use eframe::egui;
#[derive(Clone, Copy)]
pub(super) enum Rule {
    Text,
    Number { min: f64, max: f64, integer: bool },
}
impl Rule {
    fn valid(self, text: &str) -> bool {
        match self {
            Self::Text => text.chars().count() <= 1024,
            Self::Number { min, max, integer } => text.trim().parse::<f64>().is_ok_and(|n| {
                n.is_finite() && n >= min && n <= max && (!integer || n.fract() == 0.0)
            }),
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    All,
}
#[derive(Clone)]
struct Draft {
    source: String,
    text: String,
    error: Option<String>,
    composing: bool,
    before_composition: Option<String>,
    action: Option<Action>,
}
#[derive(Clone, Copy)]
struct Clipboard {
    write: fn(&str) -> Result<(), String>,
    read: fn() -> Result<String, String>,
}
fn clipboard(ctx: &egui::Context) -> Clipboard {
    ctx.data(|d| d.get_temp(egui::Id::new("drawer-text-clipboard")))
        .unwrap_or(Clipboard {
            write: |s| {
                arboard::Clipboard::new()
                    .and_then(|mut c| c.set_text(s))
                    .map_err(|e| e.to_string())
            },
            read: || {
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .map_err(|e| e.to_string())
            },
        })
}
fn key(key: egui::Key, command: bool, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: command,
            command,
            shift,
            ..Default::default()
        },
    }
}
fn selected(ctx: &egui::Context, id: egui::Id, text: &str) -> Option<String> {
    let range = egui::TextEdit::load_state(ctx, id)?.cursor.char_range()?;
    if range.is_empty() {
        return None;
    }
    let [start, end] = range.sorted_cursors();
    Some(
        text.chars()
            .skip(usize::from(start.index))
            .take(usize::from(end.index - start.index))
            .collect(),
    )
}
fn events(ctx: &egui::Context, id: egui::Id, draft: &mut Draft, clip: Clipboard) {
    if ctx.memory(|m| m.focused()) != Some(id) {
        return;
    }
    let mut input = ctx.input_mut(|i| std::mem::take(&mut i.events));
    if let Some(action) = draft.action.take() {
        input.push(match action {
            Action::Undo => key(egui::Key::Z, true, false),
            Action::Redo => key(egui::Key::Z, true, true),
            Action::Cut => egui::Event::Cut,
            Action::Copy => egui::Event::Copy,
            Action::Delete => {
                if selected(ctx, id, &draft.text).is_some() {
                    key(egui::Key::Delete, false, false)
                } else {
                    egui::Event::Text(String::new())
                }
            }
            Action::All => key(egui::Key::A, true, false),
            Action::Paste => match (clip.read)() {
                Ok(s) if s.len() <= 64 * 1024 => egui::Event::Paste(s),
                Ok(_) => {
                    draft.error = Some("Clipboard text exceeds 64 KiB".into());
                    egui::Event::Text(String::new())
                }
                Err(e) => {
                    draft.error = Some(format!("Clipboard unavailable: {e}"));
                    egui::Event::Text(String::new())
                }
            },
        });
    }
    let has_cut = input.iter().any(|e| matches!(e, egui::Event::Cut));
    let has_copy = input.iter().any(|e| matches!(e, egui::Event::Copy));
    let mut result = Vec::new();
    for event in input {
        let cut = matches!(event, egui::Event::Cut)
            || matches!(event,egui::Event::Key{key:egui::Key::X,pressed:true,modifiers,..} if modifiers.command&&!has_cut);
        let copy = matches!(event, egui::Event::Copy)
            || matches!(event,egui::Event::Key{key:egui::Key::C,pressed:true,modifiers,..} if modifiers.command&&!has_copy);
        if cut || copy {
            if let Some(text) = selected(ctx, id, &draft.text) {
                match (clip.write)(&text) {
                    Ok(()) => {
                        draft.error = None;
                        if cut {
                            result.push(key(egui::Key::Delete, false, false));
                        }
                    }
                    Err(e) => draft.error = Some(format!("Clipboard write failed; text kept: {e}")),
                }
            }
        } else {
            result.push(event);
        }
    }
    ctx.input_mut(|i| i.events = result);
}
pub(super) fn text(ui: &mut egui::Ui, value: &mut String, topic: &'static str) -> egui::Response {
    field(ui, value, Rule::Text, topic)
}
pub(super) fn finite(
    ui: &mut egui::Ui,
    value: &mut String,
    min: f64,
    max: f64,
    integer: bool,
    topic: &'static str,
) -> egui::Response {
    field(ui, value, Rule::Number { min, max, integer }, topic)
}
fn field(ui: &mut egui::Ui, value: &mut String, rule: Rule, topic: &'static str) -> egui::Response {
    let id = ui.next_auto_id();
    field_id(ui, value, rule, topic, id)
}
fn field_id(
    ui: &mut egui::Ui,
    value: &mut String,
    rule: Rule,
    topic: &'static str,
    id: egui::Id,
) -> egui::Response {
    let state_id = id.with("drawer-edit-draft");
    let mut draft = ui
        .ctx()
        .data_mut(|d| d.get_temp::<Draft>(state_id))
        .filter(|d| d.source == *value)
        .unwrap_or_else(|| Draft {
            source: value.clone(),
            text: value.clone(),
            error: None,
            composing: false,
            before_composition: None,
            action: None,
        });
    let focused = ui.memory(|m| m.has_focus(id) || m.had_focus_last_frame(id));
    events(ui.ctx(), id, &mut draft, clipboard(ui.ctx()));
    let mut cancel = false;
    let mut cancel_composition = false;
    if focused {
        ui.input_mut(|i| {
            for e in &i.events {
                if let egui::Event::Ime(ime) = e {
                    match ime {
                        egui::ImeEvent::Preedit { text, .. } => {
                            if !text.is_empty() && !draft.composing {
                                draft.before_composition = Some(draft.text.clone());
                            }
                            draft.composing = !text.is_empty();
                        }
                        egui::ImeEvent::Commit(_) => {
                            draft.composing = false;
                            draft.before_composition = None;
                        }
                        _ => {}
                    }
                }
            }
            if draft.composing {
                cancel_composition = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                    || !i.focused
                    || i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::WindowFocused(false)));
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            } else {
                cancel = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            }
        });
    }
    if cancel_composition {
        draft.text = draft
            .before_composition
            .take()
            .unwrap_or_else(|| draft.source.clone());
        draft.composing = false;
        if let Some(previous) = egui::TextEdit::load_state(ui.ctx(), id) {
            let mut state = egui::text_edit::TextEditState::default();
            state.set_undoer(previous.undoer());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(draft.text.chars().count()),
                )));
            state.store(ui.ctx(), id);
        }
    }
    let enter = focused && !draft.composing && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if cancel {
        draft.text = draft.source.clone();
        draft.error = None;
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    let mut out = egui::TextEdit::singleline(&mut draft.text)
        .id(id)
        .char_limit(1024)
        .desired_width(120.0)
        .show(ui);
    out.response.flags.remove(egui::response::Flags::CHANGED);
    if !cancel && !cancel_composition && !draft.composing && (enter || out.response.lost_focus()) {
        if rule.valid(&draft.text) {
            if *value != draft.text {
                *value = draft.text.clone();
                out.response.mark_changed();
            }
            draft.source = value.clone();
            draft.error = None;
        } else {
            draft.error = Some(match rule {
                Rule::Text => "Text is too long".into(),
                Rule::Number { min, max, integer } => format!(
                    "Enter a finite {} from {min} to {max}",
                    if integer { "whole number" } else { "number" }
                ),
            });
            out.response.request_focus();
        }
    }
    if cancel_composition && ui.input(|i| i.focused) {
        out.response.request_focus();
    }
    out.response.context_menu(|ui| {
        for (label, action) in [
            ("Undo", Action::Undo),
            ("Redo", Action::Redo),
            ("Cut", Action::Cut),
            ("Copy", Action::Copy),
            ("Paste", Action::Paste),
            ("Delete selection", Action::Delete),
            ("Select all", Action::All),
        ] {
            if ui.button(label).clicked() {
                draft.action = Some(action);
                out.response.request_focus();
                ui.close();
            }
        }
    });
    if let Some(error) = &draft.error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    super::preferences::observe(
        ui.ctx(),
        &out.response,
        !matches!(rule, Rule::Text),
        draft.composing,
    );
    super::help::tag(ui.ctx(), &out.response, topic);
    ui.ctx().data_mut(|d| d.insert_temp(state_id, draft));
    out.response.response
}
/// Dragging applies bounded values directly; text editing commits on Enter/focus loss.
pub(super) fn number<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
    suffix: &str,
    topic: &'static str,
) -> egui::Response {
    let id = ui.next_auto_id();
    let editing_id = id.with("drawer-number-editing");
    let editing = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(editing_id))
        .unwrap_or(false)
        || (ui.memory(|m| m.has_focus(id)) && !ui.input(|i| i.pointer.primary_down()));
    if editing {
        let mut text = value.to_f64().to_string();
        let response = field_id(
            ui,
            &mut text,
            Rule::Number {
                min: *range.start(),
                max: *range.end(),
                integer: T::INTEGRAL,
            },
            topic,
            id,
        );
        if response.changed() {
            *value = T::from_f64(text.trim().parse::<f64>().unwrap());
        }
        let focused = response.has_focus();
        ui.ctx().data_mut(|d| d.insert_temp(editing_id, focused));
        response
    } else {
        let mut response = ui.add(
            egui::Button::new(format!("{}{suffix}", value.to_f64()))
                .sense(egui::Sense::click_and_drag()),
        );
        if response.clicked() {
            response.request_focus();
            ui.ctx().data_mut(|d| d.insert_temp(editing_id, true));
        }
        if response.drag_stopped() {
            response.surrender_focus();
        }
        if response.dragged() {
            let speed = precision_step(speed, ui.input(|i| i.modifiers.ctrl), T::INTEGRAL);
            let n = (value.to_f64() + response.drag_delta().x as f64 * speed)
                .clamp(*range.start(), *range.end());
            let next = T::from_f64(if T::INTEGRAL { n.round() } else { n });
            if next.to_f64() != value.to_f64() {
                *value = next;
                response.mark_changed();
            }
        }
        super::help::tag(ui.ctx(), &response, topic);
        response
    }
}

pub(super) fn slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    label: impl Into<String>,
    topic: &'static str,
) -> egui::Response {
    let min = range.start().to_f64();
    let max = range.end().to_f64();
    let step = precision_step(
        if T::INTEGRAL { 1.0 } else { 0.01 },
        ui.input(|i| i.modifiers.ctrl),
        T::INTEGRAL,
    );
    ui.horizontal(|ui| {
        let before = value.to_f64();
        let a = ui.add(
            egui::Slider::new(value, range)
                .show_value(false)
                .text(label.into()),
        );
        if ui.input(|i| i.modifiers.ctrl) && a.is_pointer_button_down_on() {
            let delta = ui.input(|i| i.pointer.delta().x) as f64;
            let movement = delta * (max - min) / ui.spacing().slider_width.max(1.0) as f64 * 0.1;
            *value = T::from_f64((before + (movement / step).round() * step).clamp(min, max));
        } else if ui.input(|i| i.modifiers.ctrl) && a.has_focus() {
            let direction = ui.input(|i| {
                i.num_presses(egui::Key::ArrowRight) as i32
                    - i.num_presses(egui::Key::ArrowLeft) as i32
            });
            if direction != 0 {
                *value = T::from_f64((before + direction as f64 * step).clamp(min, max));
            }
        }
        super::help::tag(ui.ctx(), &a, topic);
        let b = number(
            ui,
            value,
            min..=max,
            if T::INTEGRAL { 1.0 } else { 0.01 },
            "",
            topic,
        );
        a | b
    })
    .inner
}
fn precision_step(base: f64, fine: bool, integer: bool) -> f64 {
    let step = base * if fine { 0.1 } else { 1.0 };
    if integer { step.max(1.0) } else { step }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fine_increment_is_one_tenth_and_unfocused_slider_does_not_round_saved_values() {
        assert_eq!(precision_step(1.0, true, false), 0.1);
        assert_eq!(precision_step(0.01, true, false), 0.001);
        assert_eq!(precision_step(1.0, true, true), 1.0);
        let ctx = egui::Context::default();
        let mut value = 0.703123;
        for fine in [false, true] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::ModifiersChanged(if fine {
                        egui::Modifiers::CTRL
                    } else {
                        egui::Modifiers::NONE
                    })],
                    ..Default::default()
                },
                |ui| {
                    slider(ui, &mut value, 0.0..=0.95, "Precision", "tool-properties");
                },
            );
            output.textures_delta.clear();
            assert_eq!(value, 0.703123);
        }
    }
    #[test]
    fn ctrl_slider_drag_changes_value_more_slowly_through_the_real_widget() {
        fn draw(ctx: &egui::Context, value: &mut f64, events: Vec<egui::Event>) -> egui::Rect {
            let mut rect = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect = slider(ui, value, 0.0..=100.0, "", "tool-properties").rect;
                },
            );
            output.textures_delta.clear();
            rect
        }
        let mut movement = Vec::new();
        for fine in [false, true] {
            let ctx = egui::Context::default();
            let mut value = 50.0;
            let rect = draw(&ctx, &mut value, vec![]);
            let pos = egui::pos2(rect.left() + 50.0, rect.center().y);
            let modifiers = if fine {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            draw(
                &ctx,
                &mut value,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ],
            );
            let before = value;
            draw(
                &ctx,
                &mut value,
                vec![egui::Event::PointerMoved(pos + egui::vec2(20.0, 0.0))],
            );
            movement.push(value - before);
        }
        assert!(
            movement[0] > 10.0 && movement[1] > 0.0 && movement[1] < movement[0] * 0.15,
            "{movement:?}"
        );
    }
    use egui::text::{CCursor, CCursorRange};
    fn run(
        ctx: &egui::Context,
        value: &mut String,
        rule: Rule,
        events: Vec<egui::Event>,
        time: f64,
    ) -> bool {
        let mut changed = false;
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                changed = field_id(ui, value, rule, "size", egui::Id::new("test-field")).changed();
            },
        );
        out.textures_delta.clear();
        changed
    }
    fn focus(ctx: &egui::Context, count: usize) {
        let id = egui::Id::new("test-field");
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(0_usize),
            CCursor::new(count),
        )));
        state.store(ctx, id);
    }
    fn draft(ctx: &egui::Context) -> Draft {
        ctx.data(|d| d.get_temp(egui::Id::new("test-field").with("drawer-edit-draft")))
            .unwrap()
    }
    fn action(ctx: &egui::Context, a: Action) {
        let mut v = draft(ctx);
        v.action = Some(a);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-field").with("drawer-edit-draft"), v));
    }
    #[test]
    fn clipboard_failure_unicode_cut_copy_paste_delete_and_field_undo_redo() {
        let ctx = egui::Context::default();
        let mut value = "한글abc".to_string();
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("drawer-text-clipboard"),
                Clipboard {
                    write: |_| Err("busy".into()),
                    read: || Ok("붙여넣기".into()),
                },
            )
        });
        run(&ctx, &mut value, Rule::Text, vec![], 0.0);
        focus(&ctx, 5);
        run(&ctx, &mut value, Rule::Text, vec![egui::Event::Cut], 1.0);
        assert_eq!(draft(&ctx).text, "한글abc");
        assert!(draft(&ctx).error.unwrap().contains("text kept"));
        assert_eq!(value, "한글abc");
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("drawer-text-clipboard"),
                Clipboard {
                    write: |s| {
                        assert_eq!(s, "한글abc");
                        Ok(())
                    },
                    read: || Ok("붙여넣기".into()),
                },
            )
        });
        run(&ctx, &mut value, Rule::Text, vec![egui::Event::Copy], 2.0);
        run(&ctx, &mut value, Rule::Text, vec![egui::Event::Cut], 3.0);
        assert_eq!(draft(&ctx).text, "");
        assert_eq!(value, "한글abc");
        run(&ctx, &mut value, Rule::Text, vec![], 4.0);
        action(&ctx, Action::Undo);
        run(&ctx, &mut value, Rule::Text, vec![], 5.0);
        assert_eq!(draft(&ctx).text, "한글abc");
        action(&ctx, Action::Redo);
        run(&ctx, &mut value, Rule::Text, vec![], 6.0);
        assert_eq!(draft(&ctx).text, "");
        action(&ctx, Action::Paste);
        run(&ctx, &mut value, Rule::Text, vec![], 7.0);
        assert_eq!(draft(&ctx).text, "붙여넣기");
        action(&ctx, Action::All);
        run(&ctx, &mut value, Rule::Text, vec![], 8.0);
        action(&ctx, Action::Delete);
        run(&ctx, &mut value, Rule::Text, vec![], 9.0);
        assert_eq!(draft(&ctx).text, "");
        // Empty selection is a no-op and must not call the failing/wrong-text clipboard.
        run(
            &ctx,
            &mut value,
            Rule::Text,
            vec![egui::Event::Cut, egui::Event::Copy],
            10.0,
        );
        assert_eq!(draft(&ctx).text, "");
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("test-field"))
        );
        let canceled = run(
            &ctx,
            &mut value,
            Rule::Text,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                key(egui::Key::Escape, false, false),
            ],
            11.0,
        );
        assert!(
            !canceled,
            "value={value:?}, source={:?}, focus={:?}",
            draft(&ctx).source,
            ctx.memory(|m| m.focused())
        );
        assert_eq!(draft(&ctx).text, "한글abc");
        assert_eq!(value, "한글abc");
    }
    #[test]
    fn numeric_validation_commit_focus_cancel_and_korean_composition() {
        let ctx = egui::Context::default();
        let mut value = "5".to_string();
        let rule = Rule::Number {
            min: 1.0,
            max: 10.0,
            integer: true,
        };
        run(&ctx, &mut value, rule, vec![], 0.0);
        for (n, invalid) in ["sentence", "NaN", "inf", "-1", "11", "1.5"]
            .into_iter()
            .enumerate()
        {
            focus(&ctx, 1024);
            run(
                &ctx,
                &mut value,
                rule,
                vec![egui::Event::Paste(invalid.into())],
                n as f64 * 3.0 + 1.0,
            );
            assert!(!run(
                &ctx,
                &mut value,
                rule,
                vec![key(egui::Key::Enter, false, false)],
                n as f64 * 3.0 + 2.0
            ));
            assert_eq!(value, "5");
            assert!(draft(&ctx).error.is_some());
        }
        focus(&ctx, 1024);
        run(
            &ctx,
            &mut value,
            rule,
            vec![egui::Event::Paste("7".into())],
            21.0,
        );
        assert_eq!(value, "5");
        assert!(run(
            &ctx,
            &mut value,
            rule,
            vec![key(egui::Key::Enter, false, false)],
            22.0
        ));
        assert_eq!(value, "7");
        focus(&ctx, 1024);
        run(
            &ctx,
            &mut value,
            rule,
            vec![egui::Event::Paste("8".into())],
            23.0,
        );
        ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("test-field")));
        assert!(run(&ctx, &mut value, rule, vec![], 24.0));
        assert_eq!(value, "8");
        let ctx = egui::Context::default();
        let mut text = String::new();
        run(&ctx, &mut text, Rule::Text, vec![], 0.0);
        focus(&ctx, 0);
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "ㅎ".into(),
                active_range_chars: None,
            })],
            1.0,
        );
        assert_eq!(text, "");
        assert!(draft(&ctx).composing);
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("한".into()))],
            2.0,
        );
        assert_eq!(text, "");
        assert!(run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![key(egui::Key::Enter, false, false)],
            3.0
        ));
        assert_eq!(text, "한");
    }
    #[test]
    fn numeric_button_commits_on_focus_change_and_escape_restores_the_edit_start() {
        let ctx = egui::Context::default();
        let mut value = 5.0;
        let mut id = egui::Id::NULL;
        let mut frame = |events| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    id = number(ui, &mut value, 1.0..=10.0, 0.1, "", "tools").id;
                },
            );
            out.textures_delta.clear();
            (id, value)
        };
        let (id, _) = frame(vec![]);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(vec![]);
        let select = || {
            let mut s = egui::TextEdit::load_state(&ctx, id).unwrap();
            s.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(0_usize),
                CCursor::new(100_usize),
            )));
            s.store(&ctx, id);
        };
        select();
        assert_eq!(frame(vec![egui::Event::Paste("7".into())]).1, 5.0);
        ctx.memory_mut(|m| m.surrender_focus(id));
        assert_eq!(frame(vec![]).1, 7.0);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(vec![]);
        select();
        assert_eq!(frame(vec![egui::Event::Paste("9".into())]).1, 7.0);
        assert_eq!(frame(vec![key(egui::Key::Escape, false, false)]).1, 7.0);
    }
    #[test]
    fn ime_enter_escape_and_window_switch_never_force_number_or_parent_cancel() {
        let ctx = egui::Context::default();
        let mut value = "5".to_string();
        let rule = Rule::Number {
            min: 1.0,
            max: 10.0,
            integer: true,
        };
        run(&ctx, &mut value, rule, vec![], 0.0);
        focus(&ctx, 1);
        let preedit = || {
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "가".into(),
                active_range_chars: None,
            })
        };
        run(&ctx, &mut value, rule, vec![preedit()], 1.0);
        assert_eq!(value, "5");
        assert!(draft(&ctx).composing);
        assert!(!run(
            &ctx,
            &mut value,
            rule,
            vec![key(egui::Key::Enter, false, false)],
            2.0
        ));
        assert!(draft(&ctx).composing);
        assert!(!run(
            &ctx,
            &mut value,
            rule,
            vec![key(egui::Key::Escape, false, false)],
            3.0
        ));
        assert_eq!(value, "5");
        assert_eq!(draft(&ctx).text, "5");
        assert!(!draft(&ctx).composing);
        assert!(!ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        focus(&ctx, 1);
        run(&ctx, &mut value, rule, vec![preedit()], 4.0);
        run(
            &ctx,
            &mut value,
            rule,
            vec![egui::Event::WindowFocused(false)],
            5.0,
        );
        assert_eq!(value, "5");
        assert!(!draft(&ctx).composing);
        let ctx = egui::Context::default();
        let mut text = "base".to_string();
        run(&ctx, &mut text, Rule::Text, vec![], 0.0);
        focus(&ctx, 4);
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![egui::Event::Text("draft".into())],
            1.0,
        );
        run(&ctx, &mut text, Rule::Text, vec![preedit()], 2.0);
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![key(egui::Key::Escape, false, false)],
            3.0,
        );
        assert_eq!(text, "base");
        assert_eq!(draft(&ctx).text, "draft");
    }
    #[test]
    fn numeric_pointer_drag_and_cancel_do_not_change_drawing_history() {
        use crate::{Drawer, tests::tiny_spec};
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let mut state = app.documents[0].snapshot();
        state.layers[0].pixels[0] = [17, 28, 39, 255];
        app.documents[0].commit_state(state).unwrap();
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let mut value = 5.0;
        let mut rect = egui::Rect::NOTHING;
        let mut frame = |events| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.basic_shortcuts(ui.ctx());
                    rect = number(ui, &mut value, 1.0..=10.0, 1.0, "", "tools").rect;
                },
            );
            out.textures_delta.clear();
            rect
        };
        let start = frame(vec![]).center();
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        frame(vec![egui::Event::PointerMoved(
            start + egui::vec2(30.0, 0.0),
        )]);
        frame(vec![egui::Event::PointerButton {
            pos: start + egui::vec2(30.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(value, 10.0);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        let mut text = "name".to_string();
        run(&ctx, &mut text, Rule::Text, vec![], 10.0);
        focus(&ctx, 4);
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![egui::Event::Text("new".into())],
            11.0,
        );
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![key(egui::Key::Z, true, false)],
                ..Default::default()
            },
            |ui| {
                app.basic_shortcuts(ui.ctx());
                field_id(
                    ui,
                    &mut text,
                    Rule::Text,
                    "files",
                    egui::Id::new("test-field"),
                );
            },
        );
        out.textures_delta.clear();
        run(
            &ctx,
            &mut text,
            Rule::Text,
            vec![key(egui::Key::Escape, false, false)],
            12.0,
        );
        assert_eq!(text, "name");
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
}
