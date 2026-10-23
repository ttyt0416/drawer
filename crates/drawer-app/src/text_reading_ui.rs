use super::*;
pub(super) fn remember(e: &mut Draft, text: String, ruby: Vec<core::Ruby>) {
    if e.reading_history
        .last()
        .is_some_and(|(t, r)| *t == text && *r == ruby)
    {
        return;
    }
    e.reading_history.push((text, ruby));
    while e.reading_history.len() > 32
        || e.reading_history
            .iter()
            .map(|(t, r)| t.len() + r.iter().map(|r| r.text.len() + 128).sum::<usize>())
            .sum::<usize>()
            > 512 * 1024
    {
        e.reading_history.remove(0);
    }
}
pub(super) fn remap_field(e: &mut Draft, before: &Settings) {
    remember(e, before.text.clone(), before.style.ruby.clone());
    if let Some((_, r)) = e
        .reading_history
        .iter()
        .rev()
        .find(|(t, _)| *t == e.settings.text)
    {
        e.settings.style.ruby = r.clone();
    } else {
        core::remap_ruby(&before.text, &e.settings.text, &mut e.settings.style.ruby);
    }
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    e: &mut Draft,
    panel: &mut Panel,
    change: &mut Option<composition::Change>,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut e.settings.style.flow,
            core::Flow::Horizontal,
            "Horizontal",
        );
        let r = ui.selectable_value(
            &mut e.settings.style.flow,
            core::Flow::Vertical,
            "Vertical (columns right to left)",
        );
        panel.vertical = Some(r.rect);
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut e.settings.style.tracking)
                .range(-0.5..=4.)
                .speed(0.01)
                .prefix("Letter spacing × "),
        );
        ui.checkbox(&mut e.settings.style.ligatures, "Standard ligatures");
    });
    let count = e.settings.text.chars().count();
    let composing = e.composition.is_some() || change.is_some();
    let selection = egui::text_edit::TextEditState::load(ui.ctx(), composition::widget_id(e))
        .and_then(|s| s.cursor.char_range());
    let (mut start, mut end) = selection.map_or((0, 1), |s| {
        (
            usize::from(s.primary.index).min(usize::from(s.secondary.index)),
            usize::from(s.primary.index).max(usize::from(s.secondary.index)),
        )
    });
    start = start.min(count.saturating_sub(1));
    if end <= start {
        end = start + 1;
    }
    end = end.min(count);
    let chars: Vec<_> = e.settings.text.chars().collect();
    let can_add = !composing
        && start < end
        && e.settings.style.ruby.len() < 64
        && !chars[start..end].contains(&'\n')
        && !e
            .settings
            .style
            .ruby
            .iter()
            .any(|r| r.start < end && r.end > start);
    let r = ui.add_enabled(
        can_add,
        egui::Button::new("Add reading to selected characters / caret"),
    );
    panel.add_reading = Some(r.rect);
    if r.clicked() {
        e.settings.style.ruby.push(core::Ruby {
            start,
            end,
            ..Default::default()
        });
        e.settings.style.ruby.sort_by_key(|r| r.start);
    }
    if !e.settings.style.ruby.is_empty() {
        let mut remove = None;
        egui::CollapsingHeader::new("Reading annotations (ruby)")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("ruby-annotations-scroll")
                    .max_height(180.)
                    .show(ui, |ui| {
                        for n in 0..e.settings.style.ruby.len() {
                            ui.push_id(n, |ui| {
                                let id = composition::field_id(e, composition::Field::Reading(n));
                                composition::checkpoint(
                                    ui.ctx(),
                                    e,
                                    composition::Field::Reading(n),
                                );
                                let mut output =
                                    egui::TextEdit::singleline(&mut e.settings.style.ruby[n].text)
                                        .id(id)
                                        .char_limit(1024)
                                        .hint_text("Reading text")
                                        .desired_width(400.)
                                        .show(ui);
                                if n == 0 {
                                    panel.reading_text = Some(output.response.rect);
                                }
                                composition::after(
                                    ui.ctx(),
                                    e,
                                    composition::Field::Reading(n),
                                    &mut output,
                                    change,
                                );
                                let composing = e.composition.is_some() || change.is_some();
                                ui.add_enabled_ui(!composing, |ui| {
                                    let r = &mut e.settings.style.ruby[n];
                                    ui.horizontal(|ui| {
                                        let mut a = r.start + 1;
                                        ui.add(
                                            egui::DragValue::new(&mut a)
                                                .range(1..=count.max(1))
                                                .prefix("From character "),
                                        );
                                        r.start = a - 1;
                                        ui.add(
                                            egui::DragValue::new(&mut r.end)
                                                .range(1..=count.max(1))
                                                .prefix("Through "),
                                        );
                                        if ui.button("Remove reading").clicked() {
                                            remove = Some(n);
                                        }
                                    });
                                    egui::ComboBox::from_id_salt("reading-font")
                                        .selected_text(r.font.label())
                                        .show_ui(ui, |ui| {
                                            for font in Font::ALL {
                                                ui.selectable_value(
                                                    &mut r.font,
                                                    font,
                                                    font.label(),
                                                );
                                            }
                                        });
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut r.ratio)
                                                .range(0.1..=2.)
                                                .speed(0.01)
                                                .prefix("Size ratio "),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut r.gap)
                                                .range(0. ..=4.)
                                                .speed(0.01)
                                                .prefix("Distance × "),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut r.tracking)
                                                .range(-0.5..=4.)
                                                .speed(0.01)
                                                .prefix("Spacing × "),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        for (value, label) in [
                                            (Align::Left, "Start"),
                                            (Align::Center, "Center"),
                                            (Align::Right, "End"),
                                        ] {
                                            ui.selectable_value(&mut r.align, value, label);
                                        }
                                        ui.selectable_value(
                                            &mut r.side,
                                            core::Side::Before,
                                            "Above / right",
                                        );
                                        ui.selectable_value(
                                            &mut r.side,
                                            core::Side::After,
                                            "Below / left",
                                        );
                                    });
                                });
                                ui.separator();
                            });
                        }
                    });
            });
        if let Some(n) = remove {
            e.settings.style.ruby.remove(n);
        }
        if !composing {
            e.settings.style.ruby.sort_by_key(|r| r.start);
        }
        if !e.settings.style.text_valid(&e.settings.text) {
            ui.colored_label(egui::Color32::YELLOW,"Reading ranges must be separate, within the body, and cannot cross a hard newline.");
        }
        ui.small("Each reading moves and wraps with its base range. Long readings reserve their own space. Deleting the entire base removes its reading; document Undo restores both.");
    }
}

#[path = "text_reading_ui_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
