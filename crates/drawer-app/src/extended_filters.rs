use drawer_core::extended_filters::Operation as O;
use eframe::egui;
fn center_radius(ui: &mut egui::Ui, center: &mut Option<[f64; 2]>, radius: &mut f64) {
    let mut custom = center.is_some();
    if ui
        .checkbox(&mut custom, "Custom center (document pixels)")
        .changed()
    {
        *center = custom.then_some([0.; 2]);
    }
    if let Some(p) = center {
        ui.horizontal(|ui| {
            for (v, label) in p.iter_mut().zip(["Center X ", "Center Y "]) {
                ui.add(egui::DragValue::new(v).range(-1e6..=1e6).prefix(label));
            }
        });
    }
    ui.add(
        egui::DragValue::new(radius)
            .range(0. ..=32768.)
            .prefix("Radius (pixels) "),
    );
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    op: &mut O,
    options: &drawer_core::filter_options::Options,
) {
    match op {
        O::Normal { strength, y_up } => {
            ui.add(egui::Slider::new(strength, 0. ..=32.).text("Height gradient strength"));
            ui.horizontal(|ui| {
                ui.radio_value(y_up, true, "Y up");
                ui.radio_value(y_up, false, "Y down");
            });
        }
        O::Abstract { outline, levels } => {
            if options.abstraction != drawer_core::filter_options::Abstraction::ColorOnly {
                ui.add(egui::Slider::new(outline, 0. ..=4.).text("Outline strength"));
            }
            if options.abstraction != drawer_core::filter_options::Abstraction::LinesOnly {
                ui.add(egui::Slider::new(levels, 2..=32).text("Color levels"));
            }
        }
        O::Jpeg { block, ring } => {
            ui.add(egui::Slider::new(block, 0. ..=1.).text("8×8 block boundary strength"));
            ui.add(egui::Slider::new(ring, 0. ..=1.).text("Ringing reduction"));
        }
        O::Polar {
            inverse,
            center,
            radius,
        } => {
            if !options.spherize {
                ui.horizontal(|ui| {
                    ui.radio_value(inverse, false, "Rectangular to polar");
                    ui.radio_value(inverse, true, "Polar to rectangular");
                });
            }
            center_radius(ui, center, radius);
        }
        O::Smoothing {
            radius,
            edge,
            amount,
        } => {
            ui.add(egui::Slider::new(radius, 0..=8).text("Radius (pixels)"));
            ui.add(egui::Slider::new(edge, 0. ..=1.).text("Edge preservation"));
            ui.add(egui::Slider::new(amount, 0. ..=1.).text("Smoothing amount"));
        }
        O::Pinch {
            strength,
            center,
            radius,
        }
        | O::Curved {
            strength,
            center,
            radius,
            ..
        } => {
            ui.add(egui::Slider::new(strength, -1. ..=1.).text("Strength"));
            if options.area == drawer_core::filter_options::Area::Specify {
                center_radius(ui, center, radius);
            }
        }
        O::Zigzag {
            amplitude,
            period,
            center,
            radius,
        }
        | O::Ripple {
            amplitude,
            period,
            center,
            radius,
        } => {
            ui.add(
                egui::DragValue::new(amplitude)
                    .range(0. ..=1024.)
                    .prefix("Amplitude (pixels) "),
            );
            ui.add(
                egui::DragValue::new(period)
                    .range(0.25..=32768.)
                    .prefix("Wavelength (pixels) "),
            );
            if options.area == drawer_core::filter_options::Area::Specify {
                center_radius(ui, center, radius);
            }
        }
        O::Blur => {
            ui.small("Alpha-preserving 3×3 box blur.");
        }
        O::BlurStrong => {
            ui.small("Alpha-preserving 7×7 box blur.");
        }
        O::SharpenMore => {
            ui.small("Alpha-preserving 3×3 unsharp residual at strength3.");
        }
    }
    if let O::Curved { cylinder, .. } = op {
        ui.horizontal(|ui| {
            ui.radio_value(cylinder, false, "Sphere");
            ui.radio_value(cylinder, true, "Cylinder");
        });
    }
}
