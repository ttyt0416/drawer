use drawer_core::{
    detail_filter::Operation as D, extended_filters::Operation as E, filter_options::*,
    pixel_effects::Operation as P, stylized_effects::Operation as S, warp_filters::Operation as W,
};
use eframe::egui;
fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, values: &[(T, &str)]) {
    let text = values
        .iter()
        .find(|(v, _)| v == value)
        .map_or("", |(_, s)| *s);
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (v, label) in values {
                ui.selectable_value(value, *v, *label);
            }
        });
}
pub(super) fn boundary(ui: &mut egui::Ui, value: &mut Boundary) {
    ui.label("Outside document");
    choice(
        ui,
        "filter-boundary",
        value,
        &Boundary::ALL.map(|v| (v, v.label())),
    );
}
pub(super) fn controls(ui: &mut egui::Ui, op: D, o: &mut Options) {
    ui.collapsing("Filter-specific options", |ui| {
        if !matches!(
            op,
            D::Pixels(
                P::Noise { .. } | P::Mosaic { .. } | P::Dust { .. } | P::Style(S::Film { .. })
            )
        ) {
            boundary(ui, &mut o.boundary);
        }
        match op {
            D::Pixels(P::Warp(W::Wave { .. })) => choice(
                ui,
                "wave-shape",
                &mut o.waveform,
                &[
                    (Waveform::Sine, "Sine"),
                    (Waveform::Triangle, "Triangle"),
                    (Waveform::Rectangular, "Rectangular"),
                ],
            ),
            D::Pixels(P::Extended(E::Abstract { .. })) => choice(
                ui,
                "abstract-mode",
                &mut o.abstraction,
                &[
                    (Abstraction::ColorAndLines, "Color and lines"),
                    (Abstraction::ColorOnly, "Color only"),
                    (Abstraction::LinesOnly, "Lines only"),
                ],
            ),
            D::Pixels(P::Extended(E::Polar { .. })) => {
                ui.checkbox(&mut o.spherize, "Spherize");
            }
            D::Pixels(P::Style(S::Aberration { .. })) => {
                choice(
                    ui,
                    "aberration-mode",
                    &mut o.aberration,
                    &[
                        (Aberration::Offset, "Offset X / Y"),
                        (Aberration::Radial, "Radial (use center point)"),
                        (Aberration::Lateral, "Lateral (use Angle)"),
                    ],
                );
                if o.aberration == Aberration::Lateral {
                    ui.add(
                        egui::DragValue::new(&mut o.angle)
                            .range(-36000. ..=36000.)
                            .prefix("Angle ° "),
                    );
                }
                if o.aberration == Aberration::Radial {
                    let mut custom = o.center.is_some();
                    if ui.checkbox(&mut custom, "Custom center").changed() {
                        o.center = custom.then_some([0.; 2]);
                    }
                    if let Some(c) = &mut o.center {
                        ui.horizontal(|ui| {
                            for (v, label) in c.iter_mut().zip(["Center X ", "Center Y "]) {
                                ui.add(egui::DragValue::new(v).range(-1e6..=1e6).prefix(label));
                            }
                        });
                    }
                }
            }
            D::Pixels(P::Style(S::Film { .. })) => choice(
                ui,
                "film-mode",
                &mut o.film,
                &[
                    (Film::None, "None"),
                    (Film::Sepia, "Sepia"),
                    (Film::Slant, "Light leak (slant)"),
                    (Film::All, "Light leak (all)"),
                    (Film::SepiaSlant, "Sepia + Light leak (slant)"),
                    (Film::SepiaAll, "Sepia + Light leak (all)"),
                ],
            ),
            D::Pixels(P::LineWidth { delta }) if delta < 0 => {
                ui.checkbox(&mut o.minimum_line, "At least 1 pixel");
            }
            _ => {}
        }
        if matches!(
            op,
            D::Pixels(
                P::Warp(W::Fisheye { .. } | W::Swirl { .. })
                    | P::Extended(
                        E::Curved { .. } | E::Pinch { .. } | E::Ripple { .. } | E::Zigzag { .. }
                    )
            )
        ) {
            choice(
                ui,
                "filter-area",
                &mut o.area,
                &[
                    (Area::EntireSelection, "Entire selection"),
                    (Area::Specify, "Specify area (use Radius and Shape)"),
                ],
            );
            if o.area == Area::Specify {
                ui.add(
                    egui::Slider::new(&mut o.shape, 0.1..=10.)
                        .logarithmic(true)
                        .text("Shape (ellipse width / height)"),
                );
            }
        }
    });
}
