use drawer_core::cleanup_filters::{Dust, Mode, Operation as O, Selection, SelectionKind};
use eframe::egui;
fn size(ui: &mut egui::Ui, area: &mut u32, threshold: &mut f64, hole: bool, white: bool) {
    ui.add(egui::Slider::new(area, 0..=4096).text("Maximum connected area (pixels)"));
    if hole {
        let mut cutoff = 1. - *threshold;
        if ui
            .add(egui::Slider::new(&mut cutoff, 0. ..=1.).text("Maximum gap opacity"))
            .changed()
        {
            *threshold = 1. - cutoff;
        }
    } else {
        ui.add(egui::Slider::new(threshold, 0. ..=1.).text(if white {
            "Minimum contrast on white"
        } else {
            "Minimum dust opacity"
        }));
    }
}
fn dust(ui: &mut egui::Ui, s: &mut Dust) {
    egui::ComboBox::from_id_salt("cleanup-mode")
        .selected_text(s.mode.label())
        .show_ui(ui, |ui| {
            for mode in Mode::ALL {
                ui.selectable_value(&mut s.mode, mode, mode.label());
            }
        });
    size(
        ui,
        &mut s.max_area,
        &mut s.threshold,
        matches!(s.mode, Mode::Surrounding | Mode::Foreground),
        s.mode == Mode::WhiteBackground,
    );
    if s.mode == Mode::Foreground {
        let mut rgba = s.foreground.map(|v| v as f32);
        ui.label("Fill foreground RGBA");
        if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
            s.foreground = rgba.map(|v| v as f64);
        }
    }
    ui.small("Full input components are classified before clipping to the existing selection and document. Components touching source/document edges are protected. Size0 changes no pixels. Mode/size changes refresh the preview automatically.");
}
pub(super) fn controls(ui: &mut egui::Ui, op: &mut O) {
    match op {
        O::Dust(s) => dust(ui, s),
        O::Gray => {
            ui.label("Convert native RGB luminance to equal gray channels; preserve alpha and hidden transparent colors.");
        }
        O::Descreen {
            radius,
            amount,
            line_threshold,
            coverage,
        } => {
            ui.checkbox(coverage, "Also average transparent tone coverage");
            ui.add(egui::Slider::new(radius, 0..=8).text("Tone averaging radius"));
            ui.add(egui::Slider::new(amount, 0. ..=1.).text("Tone removal amount"));
            ui.add(egui::Slider::new(line_threshold, 0. ..=1.).text("Continuous line tolerance"));
            ui.small("Independent spatial descreening. Averages periodic dots and protects isolated continuous dark lines; this does not recover an original pre-tone image.");
        }
    }
}
pub(super) fn selection_controls(ui: &mut egui::Ui, s: &mut Selection) {
    ui.label("Selection only — pixel layers remain unchanged");
    egui::ComboBox::from_id_salt("cleanup-selection-mode")
        .selected_text(s.kind.label())
        .show_ui(ui, |ui| {
            for kind in SelectionKind::ALL {
                ui.selectable_value(&mut s.kind, kind, kind.label());
            }
        });
    size(
        ui,
        &mut s.max_area,
        &mut s.threshold,
        s.kind == SelectionKind::Hole,
        s.kind == SelectionKind::WhiteBackground,
    );
    ui.small("Creates a replacement selection of the candidates inside the current selection/document. Use a pixel repair command to remove or fill candidates.");
}
pub(super) fn selection_image(
    d: &drawer_core::document::Document,
    state: &drawer_core::sizing::DocumentState,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<egui::ColorImage, String> {
    let size = [d.width.min(128) as usize, d.height.min(128) as usize];
    let mut image =
        egui::ColorImage::new(size, vec![egui::Color32::from_gray(48); size[0] * size[1]]);
    if let Some(mask) = &state.scene.selection_mask {
        let mut done = 0usize;
        for r in &mask.runs {
            for x in r.x as i64..r.x as i64 + r.length as i64 {
                if done.is_multiple_of(128) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Cleanup selection cancelled".into());
                }
                done += 1;
                let p = state
                    .scene
                    .selection_point([x as f64 + 0.5, r.y as f64 + 0.5]);
                if p[0] >= 0. && p[1] >= 0. && p[0] < d.width as f64 && p[1] < d.height as f64 {
                    let x = (p[0] * size[0] as f64 / d.width as f64).floor() as usize;
                    let y = (p[1] * size[1] as f64 / d.height as f64).floor() as usize;
                    image.pixels[y * size[0] + x] = egui::Color32::RED;
                }
            }
        }
    }
    Ok(image)
}
