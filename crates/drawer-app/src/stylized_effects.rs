use drawer_core::stylized_effects::Operation;
use eframe::egui;
pub(super) fn controls(
    ui: &mut egui::Ui,
    op: &mut Operation,
    state: &mut crate::pixel_effects::Controls,
) {
    match op {
        Operation::Aberration { offset, strength } => {
            ui.add(egui::Slider::new(strength, 0. ..=1.).text("Effect strength"));
            ui.horizontal(|ui| {
                ui.label("Red / blue separation X / Y");
                for v in offset {
                    ui.add(egui::DragValue::new(v).range(-32..=32));
                }
            });
            ui.small("Red and blue move in opposite directions; green stays in place. Expands stored bounds when transparency is unlocked.");
        }
        Operation::Film {
            strength,
            contrast,
            fade,
            temperature,
            grain,
            seed,
        } => {
            ui.add(egui::Slider::new(strength, 0. ..=1.).text("Effect strength"));
            ui.add(egui::Slider::new(contrast, 0. ..=1.).text("Film curve contrast"));
            ui.add(egui::Slider::new(fade, 0. ..=1.).text("Black / white fade"));
            ui.add(egui::Slider::new(temperature, -1. ..=1.).text("Cool / warm tint"));
            ui.add(egui::Slider::new(grain, 0. ..=1.).text("Film grain"));
            state.seed(ui, seed);
        }
        Operation::Pencil {
            strength,
            outline,
            shading,
            radius,
        } => {
            ui.add(egui::Slider::new(strength, 0. ..=1.).text("Effect strength"));
            ui.add(egui::Slider::new(outline, 0. ..=2.).text("Contour darkness"));
            ui.add(egui::Slider::new(shading, 0. ..=1.).text("Pencil shading"));
            ui.add(egui::Slider::new(radius, 1..=16).text("Contour sampling radius"));
            ui.small("Contours use alpha over white to exclude hidden transparent colors. Keeps original alpha; shading and contour strength are independent.");
        }
    }
}
