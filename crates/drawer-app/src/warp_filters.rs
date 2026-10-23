use drawer_core::warp_filters::Operation;
use eframe::egui;
pub(super) fn controls(
    ui: &mut egui::Ui,
    op: &mut Operation,
    options: &drawer_core::filter_options::Options,
) {
    match op {
        Operation::Fisheye {
            strength,
            radius,
            center,
        } => {
            ui.add(egui::Slider::new(strength, -1. ..=1.).text("Fisheye strength"));
            if options.area == drawer_core::filter_options::Area::Specify {
                ui.add(
                    egui::Slider::new(radius, 0. ..=32768.)
                        .logarithmic(true)
                        .text("Radius (pixels)"),
                );
                center_controls(ui, center);
            }
        }
        Operation::Panorama {
            field_of_view,
            center,
        } => {
            ui.add(egui::Slider::new(field_of_view, 0. ..=179.).text("Horizontal field of view °"));
            center_controls(ui, center);
        }
        Operation::Wave {
            amplitude,
            period,
            phase,
        } => {
            for k in 0..2 {
                ui.add(
                    egui::Slider::new(&mut amplitude[k], 0. ..=1024.).text(format!(
                        "{} displacement amplitude",
                        ["Horizontal", "Vertical"][k]
                    )),
                );
                ui.add(
                    egui::Slider::new(&mut period[k], 0.25..=32768.)
                        .logarithmic(true)
                        .text(format!(
                            "{} wave period (pixels)",
                            ["Horizontal", "Vertical"][k]
                        )),
                );
            }
            ui.add(egui::Slider::new(phase, -360. ..=360.).text("Phase °"));
        }
        Operation::Swirl {
            angle,
            radius,
            center,
        } => {
            ui.add(
                egui::Slider::new(angle, -720. ..=720.).text("Swirl angle ° (clockwise positive)"),
            );
            if options.area == drawer_core::filter_options::Area::Specify {
                ui.add(
                    egui::Slider::new(radius, 0. ..=32768.)
                        .logarithmic(true)
                        .text("Radius (pixels)"),
                );
                center_controls(ui, center);
            }
        }
    }
    ui.small("Inverse mapping; bilinear samples use the selected document-edge policy. Output keeps its stored bounds. Selection limits output only. Alpha lock preserves original alpha.");
}
fn center_controls(ui: &mut egui::Ui, center: &mut Option<[f64; 2]>) {
    let mut auto = center.is_none();
    if ui
        .checkbox(&mut auto, "Use source center automatically")
        .changed()
    {
        *center = if auto { None } else { Some([0.; 2]) };
    }
    if let Some(point) = center {
        ui.horizontal(|ui| {
            ui.label("Center X / Y (document pixels)");
            for v in point {
                ui.add(egui::DragValue::new(v).range(-1e6..=1e6));
            }
        });
    }
}
