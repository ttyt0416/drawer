use super::editor::{Editor, button, check};
use crate::egui;
pub(super) fn run() -> Result<serde_json::Value, String> {
    let mut cases = vec![];
    let mut maximum = 0f64;
    for ppp in [1., 1.25, 1.5, 2., 2.5] {
        let mut e = Editor::new(ppp)?;
        check(
            (e.ctx.pixels_per_point() - ppp).abs() < 1e-6,
            "Native DPI was not applied to the production egui context",
        )?;
        for zoom in [0.125, 1. / 3., 1., 1.375, 8.] {
            for angle in [0., 37., 90., 215.] {
                for flipped in [false, true] {
                    e.app.documents[0].view = drawer_core::view::View {
                        zoom,
                        angle,
                        flipped,
                        pan: [13.25, -9.75],
                    };
                    e.frame(vec![]);
                    let mut errors = vec![];
                    for target in [[10.5, 12.5], [32.5, 24.5], [51.5, 37.5]] {
                        let before = e.app.documents[0].snapshot();
                        let history = e.app.documents[0].history_counts();
                        let screen = e.point(target)?;
                        check(
                            e.app
                                .canvas_view
                                .viewport_rect()
                                .is_some_and(|r| r.contains(screen)),
                            "Chosen acceptance point falls outside the viewport",
                        )?;
                        e.frame(vec![
                            egui::Event::PointerMoved(screen),
                            button(screen, true),
                        ]);
                        let stroke = e
                            .app
                            .drawing
                            .preview(e.app.documents[0].id)
                            .ok_or("First press did not start a production stroke")?;
                        let actual = stroke
                            .trace()
                            .raw
                            .ok_or("Production stroke has no first-point trace")?;
                        let error = (actual[0] - target[0]).hypot(actual[1] - target[1]);
                        maximum = maximum.max(error);
                        check(
                            error <= 0.5,
                            "Screen-to-document first-point error exceeded 0.5 document pixels",
                        )?;
                        check(
                            drawer_core::render::Renderer::preview(
                                &e.app.documents[0],
                                Some(stroke),
                            )
                            .pixel(target)[3]
                                > 0,
                            "Expected document pixel has no pending ink",
                        )?;
                        e.frame(vec![button(screen, false)]);
                        let d = &mut e.app.documents[0];
                        check(
                            d.history_counts().0 == history.0 + 1,
                            "Projected click did not produce exactly one history entry",
                        )?;
                        check(
                            d.layers[0].pixels[target[1] as usize * 64 + target[0] as usize][3] > 0,
                            "Projected click painted away from the expected document pixel",
                        )?;
                        let after = d.snapshot();
                        check(
                            d.undo()
                                && d.snapshot() == before
                                && d.redo()
                                && d.snapshot() == after
                                && d.undo()
                                && d.snapshot() == before,
                            "Projected click Undo/Redo changed document geometry or pixels",
                        )?;
                        errors.push(serde_json::json!({"expected":target,"screenLogical":[screen.x,screen.y],"actual":actual,"errorDocumentPixels":error}));
                    }
                    cases.push(serde_json::json!({"dpiScale":ppp,"zoom":zoom,"angleDegrees":angle,"flipped":flipped,"panLogical":[13.25,-9.75],"firstPoints":errors}));
                }
            }
        }
    }
    Ok(
        serde_json::json!({"mode":"synthetic production editor with native DPI input","independentForwardProjection":true,"viewCombinations":cases.len(),"points":cases.len()*3,"maximumFirstPointErrorDocumentPixels":maximum,"limitDocumentPixels":0.5,"scenarios":cases,"osDisplayMappingValidated":false}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_dpi_fractional_zoom_rotation_flip_first_point_and_actual_pixels() {
        super::run().unwrap();
    }
}
