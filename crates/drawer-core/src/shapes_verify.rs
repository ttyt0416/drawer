use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    render::Renderer,
    scene::{Affine, Point},
    selection::{Mask, Run},
    shapes::{self as s, Constraint, Curve, Kind, Paint, Settings, Target},
};
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 40,
            height: 32,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-8 && (a[1] - b[1]).abs() < 1e-8
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let controls = [[4.0, 4.0], [12.0, 20.0], [20.0, 20.0], [28.0, 4.0]];
    let mut settings = Settings {
        kind: Kind::Bezier,
        ..Default::default()
    };
    for curve in Curve::ALL {
        settings.curve = curve;
        let g = s::geometry(settings, &controls)?;
        check(
            near(g.points[0], controls[0]) && near(*g.points.last().unwrap(), controls[3]),
            "Curve endpoint changed",
        )?;
        match curve {
            Curve::StraightLine => check(g.points == controls, "Straight control polyline")?,
            Curve::QuadraticBezier => check(
                near(g.points[32], [16.0, 12.0]),
                "Quadratic independent midpoint",
            )?,
            Curve::CubicBezier => check(
                near(g.points[48], [16.0, 16.0])
                    && (g.points[1][1] - 4.0) / (g.points[1][0] - 4.0) > 1.95,
                "Cubic midpoint/endpoint tangent",
            )?,
            Curve::Spline => check(
                near(g.points[32], controls[1]) && near(g.points[64], controls[2]),
                "Spline does not interpolate controls",
            )?,
        }
    }
    let mut d = make()?;
    for kind in Kind::ALL {
        for curve in Curve::ALL {
            for paint in Paint::ALL {
                let settings = Settings {
                    kind,
                    curve,
                    paint,
                    width: 1.0,
                    line: [10, 20, 30, 255],
                    fill: [200, 40, 20, 128],
                    ..Default::default()
                };
                let points = if kind.is_box() {
                    vec![[8.0, 8.0], [24.0, 24.0]]
                } else {
                    controls.to_vec()
                };
                for target in [Target::Raster(0), Target::Vector(None)] {
                    let before = d.snapshot();
                    let state = s::prepare(&d, target, &points, settings)?;
                    check(d.snapshot() == before, "Shape prepare mutated source")?;
                    d.commit_state(state)?;
                    check(d.history_counts().0 == 1, "Shape does not use one Undo")?;
                    if let Target::Vector(_) = target {
                        let p = &d.scene.lines.layers[0].paths[0];
                        check(
                            p.stroke == paint.line()
                                && p.fill.is_some() == paint.fill()
                                && p.definition.as_ref().is_some_and(|def| {
                                    def.settings == settings && def.controls == points
                                }),
                            "Editable shape recipe/fill not preserved",
                        )?;
                    }
                    check(
                        d.undo() && d.snapshot() == before && d.redo(),
                        "Shape Undo/Redo",
                    )?;
                    d.undo();
                }
            }
        }
    }
    for center in [false, true] {
        for rounded in [false, true] {
            for snap in [false, true] {
                let settings = Settings {
                    kind: Kind::Rectangle,
                    center,
                    corner: if rounded { 2.0 } else { 0.0 },
                    snap,
                    snap_step: 2.0,
                    paint: Paint::CreateBoth,
                    ..Default::default()
                };
                let controls = [[10.4, 11.6], [20.4, 19.6]];
                let before = d.snapshot();
                d.commit_state(s::prepare(&d, Target::Vector(None), &controls, settings)?)?;
                let file = root.join(format!("shape-{center}-{rounded}-{snap}.drawer"));
                crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
                check(
                    crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                        .map_err(|e| e.to_string())?
                        .snapshot()
                        == d.snapshot(),
                    "Shape recipe native reopen",
                )?;
                check(d.undo() && d.snapshot() == before, "Shape options Undo")?;
            }
        }
    }
    let ratio = s::geometry(
        Settings {
            constraint: Constraint::SpecifiedRatio,
            ratio: 1.0,
            ..Default::default()
        },
        &[[10.0, 10.0], [14.0, 12.0]],
    )?;
    check(near(ratio.points[2], [14.0, 14.0]), "Fixed ratio")?;
    let length = s::geometry(
        Settings {
            kind: Kind::Polyline,
            constraint: Constraint::SpecifyLength,
            length: 5.0,
            ..Default::default()
        },
        &[[0.0, 0.0], [6.0, 8.0]],
    )?;
    check(near(length.points[1], [3.0, 4.0]), "Fixed length")?;
    let centered = s::geometry(
        Settings {
            center: true,
            ..Default::default()
        },
        &[[10.0, 10.0], [14.0, 12.0]],
    )?;
    check(
        near(centered.points[0], [6.0, 8.0]) && near(centered.points[2], [14.0, 12.0]),
        "Center-start bounds",
    )?;
    for kind in Kind::ALL {
        check(
            s::geometry(
                Settings {
                    kind,
                    ..Default::default()
                },
                &[[2.0, 2.0]; 2],
            )
            .is_err(),
            "Zero-length shape accepted",
        )?;
    }
    check(
        s::geometry(
            Settings {
                width: f64::NAN,
                ..Default::default()
            },
            &controls,
        )
        .is_err(),
        "Invalid width accepted",
    )?;
    let before = d.snapshot();
    let state = s::prepare(
        &d,
        Target::Raster(0),
        &[[-20.0, -20.0], [8.0, 8.0]],
        Settings {
            paint: Paint::CreateFill,
            fill: [200, 40, 20, 255],
            ..Default::default()
        },
    )?;
    check(
        state.layers[0].pixels[0] == [200, 40, 20, 255]
            && state.layers[0].pixels[9 * 40 + 9] == [0; 4]
            && state.scene == before.scene,
        "Canvas-outside shape changed other pixels/scene",
    )?;
    let bowtie = [[4.0, 4.0], [20.0, 20.0], [4.0, 20.0], [20.0, 4.0]];
    check(
        s::contains(&bowtie, [12.5, 6.5]) && !s::contains(&bowtie, [6.5, 12.5]),
        "Even-odd self-crossing fill",
    )?;
    d.commit_state(s::prepare(
        &d,
        Target::Vector(None),
        &bowtie,
        Settings {
            kind: Kind::Polygon,
            paint: Paint::CreateFill,
            fill: [200, 40, 20, 255],
            ..Default::default()
        },
    )?)?;
    check(
        Renderer::new(&d).pixel([12.5, 6.5]) == [200, 40, 20, 255]
            && Renderer::new(&d).pixel([6.5, 12.5]) == [0; 4],
        "Self-crossing vector fill renderer",
    )?;
    d.undo();
    d.scene.selection_mask = Some(Mask {
        runs: vec![
            Run {
                x: 8,
                y: 8,
                length: 1,
                alpha: 128,
            },
            Run {
                x: 10,
                y: 8,
                length: 1,
                alpha: 255,
            },
        ],
    });
    let before = d.snapshot();
    let state = s::prepare(
        &d,
        Target::Raster(0),
        &[[4.0, 4.0], [20.0, 20.0]],
        Settings {
            paint: Paint::CreateFill,
            fill: [200, 40, 20, 255],
            ..Default::default()
        },
    )?;
    check(
        state.scene == before.scene
            && state.layers[0].pixels[8 * 40 + 8] == [200, 40, 20, 128]
            && state.layers[0].pixels[8 * 40 + 9] == [0; 4],
        "Shape selection strength/hole",
    )?;
    check(
        s::prepare(&d, Target::Vector(None), &controls, Settings::default()).is_err()
            && d.snapshot() == before,
        "Selected editable vector shape changed page",
    )?;
    d.scene.selection_mask = None;
    d.layers[0].locked = true;
    check(
        s::prepare(&d, Target::Raster(0), &controls, Settings::default()).is_err(),
        "Locked shape raster accepted",
    )?;
    d.layers[0].locked = false;
    let layer = d.scene.lines.add_layer("Locked shapes")?;
    d.scene.lines.layers[0].locked = true;
    let before = d.snapshot();
    check(
        s::prepare(
            &d,
            Target::Vector(Some(layer)),
            &controls,
            Settings::default(),
        )
        .is_err()
            && d.snapshot() == before,
        "Locked shape vector accepted",
    )?;
    d.scene.lines.layers.clear();
    d.scene.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let settings = Settings {
        paint: Paint::CreateFill,
        fill: [100, 200, 40, 255],
        ..Default::default()
    };
    d.commit_state(s::prepare(
        &d,
        Target::Vector(None),
        &[[8.0, 8.0], [24.0, 24.0]],
        settings,
    )?)?;
    check(
        Renderer::new(&d).pixel([12.5, 12.5]) == [100, 200, 40, 255]
            && Renderer::new(&d).pixel([30.5, 12.5]) == [0; 4],
        "Shape page coordinate conversion",
    )?;
    let mut refs = make()?;
    refs.commit_state(s::prepare(
        &refs,
        Target::Vector(None),
        &[[8.0, 8.0], [24.0, 24.0]],
        Settings {
            paint: Paint::CreateFill,
            line: [0; 4],
            fill: [0, 0, 0, 255],
            ..Default::default()
        },
    )?)?;
    let id = refs.scene.lines.layers[0].id;
    refs.scene.lines.settings.references = vec![crate::line_tools::Target::Vector(id)];
    check(
        crate::line_regions::Region::build(&refs, [12.5, 12.5])
            .err()
            .is_some_and(|e| e.contains("starting point is on")),
        "Fill-only shape ignored by line references",
    )?;
    let mut snapshot = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.shapes = Settings {
        kind: Kind::Bezier,
        curve: Curve::CubicBezier,
        input: s::Input::ByClick,
        corner: 2.0,
        snap: true,
        ..Default::default()
    };
    snapshot.shapes_vector = true;
    let file = root.join("shape-settings.json");
    snapshot.export(&file).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Snapshot::read(&file).map_err(|e| e.to_string())?;
    check(
        loaded.shapes == snapshot.shapes && loaded.shapes_vector,
        "Shape settings reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn shapes_curves_constraints_fill_raster_vector_history_persistence() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
