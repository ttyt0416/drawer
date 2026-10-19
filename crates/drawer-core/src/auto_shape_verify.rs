use crate::{
    auto_shape::{self as core, Capture, Kind, Plan, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    shapes::Target,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let s = Settings {
        enabled: true,
        hold_ms: 500,
        motion_pixels: 3.0,
    };
    let mut c = Capture::new([8.0, 8.0], 0.0)?;
    check(!c.ready(9.0, s), "A dot became a shape")?;
    c.push([40.0, 40.0], 0.1, 3.0)?;
    check(
        !c.ready(0.599999, s) && c.ready(0.6, s) && c.ready(0.600001, s),
        "Hold before/equal/after threshold",
    )?;
    let points = c.points.clone();
    c.consume();
    check(
        !c.ready(1.0, s) && c.points == points,
        "Cancelling correction lost raw stroke/retriggered",
    )?;
    c.push([41.0, 40.0], 1.0, 3.0)?;
    check(
        !c.ready(2.0, s),
        "Within-radius jitter retriggered correction",
    )?;
    c.push([44.0, 40.0], 2.0, 3.0)?;
    check(
        !c.ready(2.499999, s) && c.ready(2.5, s),
        "Movement did not reset hold",
    )?;
    c.push([48.0, 40.0], 1.0, 3.0)?;
    check(!c.ready(0.9, s), "Backward clock triggered hold")?;
    check(
        c.push([f64::NAN, 0.0], 2.0, 3.0).is_err(),
        "Invalid shape coordinate accepted",
    )?;
    let square = [
        [12.0, 12.0],
        [48.0, 12.0],
        [48.0, 48.0],
        [12.0, 48.0],
        [12.0, 12.0],
    ];
    let triangle = [[30.0, 10.0], [50.0, 48.0], [10.0, 48.0], [30.0, 10.0]];
    check(
        core::recognize(&square)? == Kind::Rectangle
            && core::recognize(&triangle)? == Kind::Triangle
            && core::recognize(&[[10.0, 10.0], [30.0, 30.2], [50.0, 50.0]])? == Kind::Line
            && core::recognize(&[[10.0, 10.0], [30.0, 48.0], [50.0, 10.0]])? == Kind::Curve,
        "Independent shape recognition fixtures",
    )?;
    for (rx, ry, wanted) in [(20.0, 20.0, Kind::Circle), (22.0, 10.0, Kind::Ellipse)] {
        let points = (0..=64)
            .map(|i| {
                let (s, c) = (std::f64::consts::TAU * i as f64 / 64.0).sin_cos();
                [32.0 + rx * c, 32.0 + ry * s]
            })
            .collect::<Vec<_>>();
        check(
            core::recognize(&points)? == wanted,
            "Circle/ellipse recognition",
        )?;
    }
    for kind in Kind::ALL {
        for vector in [false, true] {
            let mut d = make()?;
            let target = if vector {
                Target::Vector(Some(d.scene.lines.add_layer("Converted")?))
            } else {
                Target::Raster(0)
            };
            let before = d.snapshot();
            let raw = square.to_vec();
            let mut p = Plan::from_points(&raw, kind, 2.0, [40, 80, 160, 255])?;
            check(
                !p.geometry(&d)?.points.is_empty() && raw == square,
                "Candidate generation changed raw trace",
            )?;
            if kind == Kind::Rectangle {
                p.corner = 3.0;
                p.controls[1] = [52.0, 50.0];
                check(
                    p.geometry(&d)?.points.len() > 4,
                    "Corner/control edit did not affect preview",
                )?;
            }
            let state = p.prepare(&d, target)?;
            check(
                d.snapshot() == before,
                "Shape conversion preview changed document",
            )?;
            d.commit_state(state)?;
            check(d.history_counts().0 == 1, "Conversion did not use one Undo")?;
            if vector {
                let path = &d.scene.lines.layers[0].paths[0];
                check(
                    path.definition
                        .as_ref()
                        .is_some_and(|def| def.controls == p.controls)
                        && d.layers == before.layers,
                    "Editable vector controls missing / raster changed",
                )?;
            } else {
                check(
                    d.layers[0].pixels.iter().any(|p| p[3] > 0),
                    "Raster conversion produced no pixels",
                )?;
            }
            let file = root.join(format!("auto-shape-{kind:?}-{vector}.drawer"));
            crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
            check(
                crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == d.snapshot(),
                "Converted shape native reopen",
            )?;
            check(
                d.undo() && d.snapshot() == before && d.redo(),
                "Auto-shape one Undo/Redo",
            )?;
        }
    }
    let mut d = make()?;
    let layer = d.scene.lines.add_layer("Freehand")?;
    let raw = [[10.0, 11.0], [20.0, 25.0], [40.0, 12.0]];
    let before = d.snapshot();
    d.commit_state(core::freehand_vector(
        &d,
        layer,
        &raw,
        3.0,
        [20, 30, 40, 128],
    )?)?;
    check(
        d.scene.lines.layers[0].paths[0].points == raw && d.undo() && d.snapshot() == before,
        "Vector freehand fallback / Undo",
    )?;
    d.scene.lines.layers[0].locked = true;
    check(
        core::freehand_vector(&d, layer, &raw, 3.0, [0; 4]).is_err(),
        "Freehand fallback bypassed vector lock",
    )?;
    let workspace = crate::workspaces::Snapshot {
        auto_shape: s,
        ..Default::default()
    };
    let file = root.join("auto-shape-settings.json");
    workspace.export(&file).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&file)
            .map_err(|e| e.to_string())?
            .auto_shape
            == s,
        "Auto shape settings reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn hold_candidates_controls_outputs_fallback_undo_and_storage() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
