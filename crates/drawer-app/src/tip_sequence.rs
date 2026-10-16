use super::*;
use drawer_core::tip_sequence::{Order, Settings};
pub(super) fn settings_ui(
    ui: &mut egui::Ui,
    s: &mut Settings,
    library: &drawer_core::material_library::Library,
) {
    crate::help::checkbox(
        ui,
        &mut s.enabled,
        "Use multiple local tip masks",
        "tip-sequence",
    );
    egui::ComboBox::from_id_salt("tip-order")
        .selected_text(s.order.label())
        .show_ui(ui, |ui| {
            for order in Order::ALL {
                ui.selectable_value(&mut s.order, order, order.label());
            }
        });
    super::edit::number(
        ui,
        &mut s.seed,
        0.0..=u32::MAX as f64,
        1.0,
        " order seed",
        "tip-sequence",
    );
    let mut remove = None;
    let mut movement = None;
    for n in 0..s.count as usize {
        ui.push_id(("tip-slot", n), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Tip {}", n + 1));
                egui::ComboBox::from_id_salt("mask")
                    .selected_text(
                        library
                            .label(s.tips[n], drawer_core::material_library::Kind::Tip)
                            .unwrap_or_else(|| Settings::label(s.tips[n])),
                    )
                    .show_ui(ui, |ui| {
                        for source in 0..=BrushTip::ALL.len() as u32 {
                            ui.selectable_value(&mut s.tips[n], source, Settings::label(source));
                        }
                        for a in library
                            .assets()
                            .iter()
                            .filter(|a| a.kind == drawer_core::material_library::Kind::Tip)
                        {
                            ui.selectable_value(
                                &mut s.tips[n],
                                a.id,
                                format!("{} (#{})", a.name, a.id),
                            );
                        }
                    });
                if ui.add_enabled(n > 0, egui::Button::new("Up")).clicked() {
                    movement = Some((n, n - 1));
                }
                if ui
                    .add_enabled(n + 1 < s.count as usize, egui::Button::new("Down"))
                    .clicked()
                {
                    movement = Some((n, n + 1));
                }
                if ui
                    .add_enabled(s.count > 1, egui::Button::new("Remove tip"))
                    .clicked()
                {
                    remove = Some(n);
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.slots[n].flip[0], "Flip tip left/right");
                ui.checkbox(&mut s.slots[n].flip[1], "Flip tip up/down");
            });
            super::tip_texture::option_ui(ui, &mut s.slots[n].texture, library);
        });
    }
    if let Some((a, b)) = movement {
        s.tips.swap(a, b);
        s.slots.swap(a, b);
    }
    if let Some(n) = remove {
        for k in n..s.count as usize - 1 {
            s.tips[k] = s.tips[k + 1];
            s.slots[k] = s.slots[k + 1];
        }
        s.count -= 1;
        s.tips[s.count as usize] = 0;
        s.slots[s.count as usize] = Default::default();
    }
    if ui
        .add_enabled(s.count < 8, egui::Button::new("Add local tip"))
        .clicked()
    {
        s.tips[s.count as usize] = 0;
        s.slots[s.count as usize] = Default::default();
        s.count += 1;
    }
    for source in s.missing().into_iter().filter(|id| {
        library
            .get(*id, drawer_core::material_library::Kind::Tip)
            .is_none()
    }) {
        ui.colored_label(egui::Color32::YELLOW, Settings::label(source));
    }
    ui.label("Slot 1 controls the current tip when multiple masks are off. Texture overrides apply before combining tips; inherited brush textures use Multiply in this mode.");
    ui.label("Do not repeat keeps the last mask. One time only / one random cycle stop after one pass. Mask choice does not turn a color brush into an eraser.");
}
pub(super) fn ends_ui(ui: &mut egui::Ui, s: &mut drawer_core::stroke_ends::Settings) {
    use drawer_core::stroke_ends::Method;
    let old = s.method;
    egui::ComboBox::from_id_salt("taper-method")
        .selected_text(s.method.label())
        .show_ui(ui, |ui| {
            for method in Method::ALL {
                ui.selectable_value(&mut s.method, method, method.label());
            }
        });
    let max = if s.method == Method::Percentage {
        100.0
    } else {
        1e6
    };
    if old != s.method {
        s.start = s.start.min(max);
        s.end = s.end.min(max);
        super::edit::discard_layer(ui.ctx(), ui.layer_id());
    }
    ui.add_enabled_ui(s.method != Method::Fade, |ui| {
        super::edit::number(
            ui,
            &mut s.start,
            0.0..=max,
            1.0,
            if s.method == Method::Percentage {
                "% start"
            } else {
                " px start"
            },
            "tip-sequence",
        );
    });
    super::edit::number(
        ui,
        &mut s.end,
        0.0..=max,
        1.0,
        if s.method == Method::Percentage {
            "% end"
        } else if s.method == Method::Fade {
            " px fade distance"
        } else {
            " px end"
        },
        "tip-sequence",
    );
    if s.method == Method::Fade {
        super::edit::slider(
            ui,
            &mut s.fade_minimum,
            0.0..=1.0,
            "Hold this minimum after fading",
            "tip-sequence",
        );
        ui.label("Fade starts at full size and reaches the chosen minimum from the beginning; it stays there until release.");
    }
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::tip_sequence::verify()?;
    drawer_core::brush_material::verify_mix_modes()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence {
        size: 8.0,
        ..Default::default()
    };
    s.sequences[0] = Settings {
        enabled: true,
        count: 3,
        tips: [7, 2, 5, 0, 0, 0, 0, 0],
        seed: 825,
        ..Default::default()
    };
    for order in Order::ALL {
        s.sequences[0].order = order;
        let a = super::input_replay::run(&s, &cancel, &progress)?;
        let b = super::input_replay::run(&s, &cancel, &progress)?;
        if !a.record.passed || !b.record.passed || a.pixels != b.pixels {
            return Err(format!(
                "Actual input {} is not reproducible",
                order.label()
            ));
        }
    }
    s.sequences[0].count = 1;
    s.sequences[0].tips[0] = 999;
    s.sequences[0].order = Order::Repeat;
    let missing = super::input_replay::run(&s, &cancel, &progress)?;
    s.sequences[0].tips[0] = 0;
    let plain = super::input_replay::run(&s, &cancel, &progress)?;
    if !missing.record.passed || !plain.record.passed || missing.pixels != plain.pixels {
        return Err("Deleted tip fallback changed parent recipe".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_replay_all_orders_seed_and_deleted_asset() {
        super::smoke().unwrap();
    }
}
