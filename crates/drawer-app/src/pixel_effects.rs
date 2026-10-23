use drawer_core::pixel_effects::{DustTarget, Operation};
use eframe::egui;
pub(super) struct Controls {
    seed: String,
    source: Option<u64>,
    pub valid: bool,
    pub seed_rect: Option<egui::Rect>,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            seed: String::new(),
            source: None,
            valid: true,
            seed_rect: None,
        }
    }
}
impl Controls {
    pub(super) fn seed(&mut self, ui: &mut egui::Ui, seed: &mut u64) {
        if self.source != Some(*seed) {
            self.seed = seed.to_string();
            self.source = Some(*seed);
        }
        ui.horizontal(|ui| {
            ui.label("Seed");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.seed)
                    .char_limit(20)
                    .desired_width(185.),
            );
            self.seed_rect = Some(r.rect);
        });
        match self.seed.parse::<u64>() {
            Ok(value) => {
                *seed = value;
                self.source = Some(value);
            }
            Err(_) => {
                self.valid = false;
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Seed must be an integer from0 to18446744073709551615",
                );
            }
        }
    }
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    op: &mut Operation,
    state: &mut Controls,
    options: &drawer_core::filter_options::Options,
) {
    state.valid = true;
    match op {
        Operation::Cleanup(op) => crate::cleanup_filters::controls(ui, op),
        Operation::Extended(op) => crate::extended_filters::controls(ui, op, options),
        Operation::Warp(op) => crate::warp_filters::controls(ui, op, options),
        Operation::Style(op) => crate::stylized_effects::controls(ui, op, state),
        Operation::Mosaic { cell, origin } => {
            ui.add(egui::Slider::new(cell, 1..=256).text("Cell size (pixels)"));
            ui.horizontal(|ui| {
                ui.label("Grid origin X / Y");
                for v in origin {
                    ui.add(egui::DragValue::new(v).range(-1_000_000..=1_000_000));
                }
            });
        }
        Operation::Noise {
            amount,
            seed,
            monochrome,
        } => {
            ui.add(egui::Slider::new(amount, 0. ..=1.).text("Noise amount"));
            state.seed(ui, seed);
            ui.horizontal(|ui| {
                ui.radio_value(monochrome, false, "Color");
                ui.radio_value(monochrome, true, "Gray");
            });
        }
        Operation::Dust {
            max_area,
            threshold,
            target,
        } => {
            egui::ComboBox::from_id_salt("speck-target")
                .selected_text(target.label())
                .show_ui(ui, |ui| {
                    for t in DustTarget::ALL {
                        ui.selectable_value(target, t, t.label());
                    }
                });
            ui.add(egui::Slider::new(max_area, 0..=4096).text("Maximum connected area (pixels)"));
            ui.add(egui::Slider::new(threshold, 0. ..=1.).text("Luminance / alpha threshold"));
            ui.small("The red candidate map marks affected pixels. Intentional dots can also be candidates: restrict the selection or lower the area limit. Components touching stored edges are protected.");
        }
        Operation::LineWidth { delta } => {
            ui.horizontal(|ui| {
                let mut thicken = *delta >= 0;
                if ui.radio_value(&mut thicken, true, "Thicken").changed() {
                    *delta = delta.abs();
                }
                if ui.radio_value(&mut thicken, false, "Narrow").changed() {
                    *delta = -delta.abs().max(1);
                }
            });
            ui.add(egui::Slider::new(delta, -16..=16).text("Width change (pixels)"));
            ui.small("Positive expands alpha with a square neighborhood; negative shrinks it. Existing ink colors remain; expansion inherits the nearest highest-alpha sample. Unlock transparency first.");
        }
    }
}
pub(super) fn candidate_image(
    d: &drawer_core::document::Document,
    state: &drawer_core::sizing::DocumentState,
    layer: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<egui::ColorImage, String> {
    let src = &d.layers[layer];
    let extent = src.extent(d.width, d.height);
    let size = [
        extent.width.min(64) as usize,
        extent.height.min(64) as usize,
    ];
    let mut image =
        egui::ColorImage::new(size, vec![egui::Color32::from_gray(48); size[0] * size[1]]);
    for i in 0..src.pixels.len() {
        if i.is_multiple_of(128) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Filter cancelled".into());
        }
        let x = i % extent.width as usize;
        let y = i / extent.width as usize;
        let after = state.layers[layer].sample(
            d.width,
            d.height,
            extent.offset[0] + x as i64,
            extent.offset[1] + y as i64,
        );
        if after != src.sample_at(i) {
            let px = x * size[0] / extent.width as usize;
            let py = y * size[1] / extent.height as usize;
            image.pixels[py * size[0] + px] = egui::Color32::RED;
        }
    }
    Ok(image)
}
