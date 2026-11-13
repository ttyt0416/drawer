use drawer_core::output_watermark::{Anchor, Direction, Pattern, Settings, Source};
use eframe::egui;
pub(super) fn editor(ui: &mut egui::Ui, s: &mut Settings, message: &mut String) {
    ui.checkbox(&mut s.enabled, "Watermark enabled");
    ui.add_enabled_ui(s.enabled, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.source, Source::Text, crate::i18n::text("Text"));
            ui.selectable_value(&mut s.source, Source::Image, "Local PNG");
        });
        ui.add_enabled_ui(s.source == Source::Text, |ui| {
            ui.text_edit_multiline(&mut s.text);
            ui.add(
                egui::DragValue::new(&mut s.size)
                    .range(1.0..=512.)
                    .prefix("Text px "),
            );
            ui.color_edit_button_srgba_unmultiplied(&mut s.color);
            egui::ComboBox::from_id_salt("watermark-font")
                .selected_text(s.font.label())
                .show_ui(ui, |ui| {
                    for f in drawer_core::text_layout::Font::ALL {
                        ui.selectable_value(&mut s.font, f, f.label());
                    }
                });
        });
        ui.add_enabled_ui(s.source == Source::Image, |ui| {
            let mut path = s
                .path
                .as_ref()
                .map_or(String::new(), |p| p.display().to_string());
            if ui.text_edit_singleline(&mut path).changed() {
                s.path = (!path.is_empty()).then(|| path.into());
                s.image = None;
            }
            if ui.button("Choose local PNG…").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("Static PNG", &["png"])
                    .pick_file()
            {
                s.path = Some(p);
                s.image = None;
            }
            if ui.button("Reload image").clicked() {
                *message = match s.reload(drawer_core::document::DEFAULT_DOCUMENT_BUDGET) {
                    Ok(()) => "Watermark reloaded; prepare output again".into(),
                    Err(e) => e,
                };
            }
            ui.checkbox(
                &mut s.embed,
                "Embed image pixels in local preset (otherwise reload path for each preparation)",
            );
        });
        egui::ComboBox::from_id_salt("watermark-mode")
            .selected_text(s.mode.label())
            .show_ui(ui, |ui| {
                for m in drawer_core::layer_blend::Mode::ALL {
                    ui.selectable_value(&mut s.mode, m, m.label());
                }
            });
        ui.add(egui::Slider::new(&mut s.opacity, 0.0..=1.).text("Watermark opacity"));
        ui.checkbox(&mut s.lock, crate::i18n::text("Lock aspect ratio"));
        let old = s.scale;
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut s.scale[0])
                    .speed(0.01)
                    .range(0.01..=100.)
                    .prefix("Scale X "),
            );
            ui.add_enabled(
                !s.lock,
                egui::DragValue::new(&mut s.scale[1])
                    .speed(0.01)
                    .range(0.01..=100.)
                    .prefix("Scale Y "),
            );
        });
        if s.lock && s.scale[0] != old[0] {
            s.scale[1] = (old[1] * s.scale[0] / old[0]).clamp(0.01, 100.);
        }
        ui.add(
            egui::DragValue::new(&mut s.rotation)
                .range(-360.0..=360.)
                .prefix("Rotation ° "),
        );
        egui::ComboBox::from_id_salt("watermark-anchor")
            .selected_text(format!("{:?}", s.anchor))
            .show_ui(ui, |ui| {
                for a in Anchor::ALL {
                    ui.selectable_value(&mut s.anchor, a, format!("{a:?}"));
                }
            });
        ui.horizontal(|ui| {
            for (n, v) in s.position.iter_mut().enumerate() {
                ui.add(egui::DragValue::new(v).range(-1e6..=1e6).prefix(if n == 0 {
                    "X px "
                } else {
                    "Y px "
                }));
            }
        });
        ui.horizontal(|ui| {
            for (n, v) in s.margin.iter_mut().enumerate() {
                ui.add(egui::DragValue::new(v).range(0.0..=1e6).prefix(if n == 0 {
                    "Margin X "
                } else {
                    "Margin Y "
                }));
            }
        });
        if ui.button("Reset transform only").clicked() {
            s.reset_transform();
        }
        ui.checkbox(&mut s.repeat, "Repeat watermark");
        ui.add_enabled_ui(s.repeat, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.pattern, Pattern::Grid, "Grid");
                ui.selectable_value(&mut s.pattern, Pattern::Staggered, "Staggered");
            });
            ui.horizontal(|ui| {
                for d in [Direction::Horizontal, Direction::Vertical, Direction::Both] {
                    ui.selectable_value(&mut s.direction, d, format!("{d:?}"));
                }
            });
            for (n, v) in s.spacing.iter_mut().enumerate() {
                ui.add(egui::DragValue::new(v).range(0.0..=1e6).prefix(if n == 0 {
                    "Spacing X "
                } else {
                    "Spacing Y "
                }));
            }
        });
    });
    ui.checkbox(&mut s.noise, "Independent output noise");
    ui.add_enabled_ui(s.noise, |ui| {
        ui.add(egui::Slider::new(&mut s.strength, 0.0..=0.25).text("Noise strength"));
        ui.add(egui::DragValue::new(&mut s.seed).prefix("Reproducible seed "));
    });
    ui.add_enabled_ui(s.enabled || s.noise, |ui| {
        ui.checkbox(&mut s.first_is_cover, "First output is a cover");
        ui.add_enabled(
            s.first_is_cover,
            egui::Checkbox::new(&mut s.include_cover, "Include cover"),
        );
    });
    ui.small("Output pixels only; margins/position/spacing use output pixels. Fixed seed and coordinates across pages/time-lapse still frames. Transparent alpha is preserved by noise. Save explicitly; Close discards unsaved preset changes.");
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::output_watermark::verify::run(root)?;
    crate::output_batch::watermark_smoke(root)
}
