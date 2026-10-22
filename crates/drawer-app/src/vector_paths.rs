use super::*;
use drawer_core::vector_tip::{self as core, Overlay};
pub(crate) fn paint(
    ui: &egui::Ui,
    d: &Document,
    rect: egui::Rect,
    selected: Option<u64>,
    settings: Overlay,
) -> (usize, usize) {
    let paths = core::outlines(d, selected, settings);
    let painter = ui.painter_at(rect);
    let project = |point| {
        let p = d.view.document_to_screen(
            point,
            [d.width as f64, d.height as f64],
            [rect.center().x as f64, rect.center().y as f64],
            ui.ctx().pixels_per_point() as f64,
        );
        egui::pos2(p[0] as f32, p[1] as f32)
    };
    let mut occupied = std::collections::BTreeSet::new();
    let mut count = 0;
    for path in &paths {
        let mut points = path.points.iter().map(|p| project(*p)).collect::<Vec<_>>();
        if path.closed {
            points.push(points[0]);
        }
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1., egui::Color32::from_rgb(60, 210, 230)),
        ));
        for control in &path.controls {
            let p = project(*control);
            if rect.expand(4.).contains(p)
                && occupied.insert(((p.x / 4.).floor() as i32, (p.y / 4.).floor() as i32))
            {
                painter.circle_stroke(p, 2.5, egui::Stroke::new(1., egui::Color32::WHITE));
                count += 1;
            }
        }
    }
    (paths.len(), count)
}
pub(crate) fn verify() -> Result<(), String> {
    let mut d = drawer_core::vector_tip_verify::fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let before = d.snapshot();
    let history = d.history_counts();
    let ctx = egui::Context::default();
    for ppp in [1., 2.5] {
        for zoom in [0.5, 3.] {
            ctx.set_pixels_per_point(ppp);
            d.view.zoom = zoom;
            let mut seen = (0, 0);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 500.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let rect = ui.max_rect();
                    seen = paint(
                        ui,
                        &d,
                        rect,
                        Some(layer),
                        Overlay {
                            enabled: true,
                            scope: core::Scope::Selected,
                            points: core::Points::All,
                        },
                    );
                    for path in core::outlines(
                        &d,
                        Some(layer),
                        Overlay {
                            enabled: true,
                            scope: core::Scope::Selected,
                            points: core::Points::All,
                        },
                    ) {
                        for point in path.controls {
                            let screen = d.view.document_to_screen(
                                point,
                                [32., 32.],
                                [rect.center().x as f64, rect.center().y as f64],
                                ppp as f64,
                            );
                            let document = d.view.screen_to_document(
                                screen,
                                [32., 32.],
                                [rect.center().x as f64, rect.center().y as f64],
                                ppp as f64,
                            );
                            if !drawer_core::vector_edit::hits(
                                &d,
                                layer,
                                document,
                                0.01,
                                drawer_core::vector_edit::Tool::Point,
                            )
                            .iter()
                            .any(|h| h.point == point)
                            {
                                seen = (0, 0);
                            }
                        }
                    }
                },
            );
            output.textures_delta.clear();
            if seen.0 != 2
                || seen.1 < 2
                || output.shapes.is_empty()
                || d.snapshot() != before
                || d.history_counts() != history
            {
                return Err("Vector overlay DPI/zoom/point picking/source isolation".into());
            }
        }
    }
    // Thousands of coincident controls are coalesced for display only; picking still reads all nodes.
    d.scene.lines.layers[0].paths[0].points = vec![[16., 16.]; 4096];
    let mut seen = (0, 0);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        seen = paint(
            ui,
            &d,
            ui.max_rect(),
            Some(layer),
            Overlay {
                enabled: true,
                scope: core::Scope::Selected,
                points: core::Points::All,
            },
        );
    });
    output.textures_delta.clear();
    if seen.0 != 2 || seen.1 > 3 {
        return Err("Dense overlay control coalescing".into());
    }
    Ok(())
}
