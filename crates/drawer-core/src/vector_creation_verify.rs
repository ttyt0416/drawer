use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    shapes::{self as core, Curve, Input, Kind, Paint, Settings, Target},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for curve in Curve::ALL {
        for (closed, controls) in [
            (false, vec![[0.5, 0.5], [0.500001, 0.5]]),
            (false, vec![[-10., -10.], [5., 10.], [30., 40.], [70., 70.]]),
            (true, vec![[2., 2.], [8., 2.], [8., 8.], [2., 8.]]),
        ] {
            let mut d = crate::search_verify::page()?;
            let before = d.snapshot();
            let settings = Settings {
                kind: Kind::Bezier,
                curve,
                input: Input::ByClick,
                paint: Paint::CreateLine,
                width: 0.1,
                closed,
                line: [10, 30, 210, 255],
                ..Default::default()
            };
            let prepared = core::prepare(&d, Target::Vector(None), &controls, settings)?;
            check(d.snapshot() == before, "Vector preparation mutated page")?;
            d.commit_state(prepared)?;
            let layer = &d.scene.lines.layers[0];
            let path = &layer.paths[0];
            check(
                path.definition
                    .as_ref()
                    .is_some_and(|p| p.controls == controls && p.settings == settings)
                    && path.width == 0.1
                    && path.closed == closed,
                "Editable vector controls/style/closure lost",
            )?;
            check(
                path.points.first() == controls.first() && (!closed || !path.points.is_empty()),
                "Vector curve start changed",
            )?;
            if controls[0][0] < 0. {
                check(
                    path.points[0][0] < 0. && path.points.last().unwrap()[0] > d.width as f64,
                    "Off-page vector coordinates cropped",
                )?;
            }
            let id = layer.id;
            let old_path = path.clone();
            let after = d.snapshot();
            let file = root.join("vector-creation.drawer");
            crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
            let reopened =
                crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
            check(
                reopened.snapshot() == after,
                "Vector controls/style native roundtrip",
            )?;
            check(
                d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
                "Vector layer/path creation not one Undo",
            )?;
            d.commit_state(core::prepare(
                &d,
                Target::Vector(Some(id)),
                &[[3., 4.], [7., 9.]],
                Settings {
                    kind: Kind::Polyline,
                    curve: Curve::StraightLine,
                    closed: false,
                    ..settings
                },
            )?)?;
            check(
                d.scene.lines.layers.len() == 1
                    && d.scene.lines.layers[0].paths.len() == 2
                    && d.scene.lines.layers[0].paths[0] == old_path,
                "Existing vector target not retained",
            )?;
            check(d.undo() && d.snapshot() == after, "Second vector path Undo")?;
            let before = d.snapshot();
            d.scene.lines.layers[0].locked = true;
            let locked = d.snapshot();
            check(
                core::prepare(&d, Target::Vector(Some(id)), &controls, settings).is_err()
                    && d.snapshot() == locked,
                "Locked vector changed",
            )?;
            before.restore(&mut d);
        }
    }
    let mut d = crate::search_verify::page()?;
    let before = d.snapshot();
    check(
        core::prepare(&d, Target::Vector(None), &[[2., 2.]], Settings::default()).is_err()
            && d.snapshot() == before,
        "Incomplete clicked path changed page",
    )?;
    let id = d.scene.lines.add_layer("Dot")?;
    let before = d.snapshot();
    d.commit_state(crate::auto_shape::freehand_vector(
        &d,
        id,
        &[[2.5, 2.5]],
        0.5,
        [20, 40, 60, 255],
    )?)?;
    check(
        d.scene.lines.layers[0].paths[0].points == vec![[2.5, 2.5]; 2]
            && crate::render::Renderer::new(&d).pixel([2.5, 2.5]) == [20, 40, 60, 255],
        "Single-click freehand vector dot missing",
    )?;
    check(d.undo() && d.snapshot() == before, "Vector dot Undo")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn editable_curves_tiny_closed_off_page_existing_layer_native_dot_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
