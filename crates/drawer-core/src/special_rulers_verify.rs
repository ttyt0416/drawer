use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::Target,
    ruler_scope::{self, Display, Scope},
    rulers::{self, Kind, Runtime},
    scene::{Affine, Point},
    special_rulers::{Marks, Perspective, Spec},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6
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
fn runtime(spec: Spec) -> Result<(Document, Runtime), String> {
    let mut d = make()?;
    d.scene.rulers.add("Special", Kind::Special(spec))?;
    let mut r = Runtime::for_target(&d, Target::Raster(0))?;
    r.configure_snapping(
        &d,
        crate::snapping::Settings {
            screen_distance: 2.0,
            ..Default::default()
        },
        1.0,
    )?;
    Ok((d, r))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let (_, mut p) = runtime(Spec::ParallelLine {
        a: [0.0, 0.0],
        b: [64.0, 0.0],
    })?;
    check(
        near(p.constrain([8.0, 8.0]), [8.0, 8.0]) && near(p.constrain([40.0, 30.0]), [40.0, 8.0]),
        "Parallel line anchor constraint",
    )?;
    let (_, mut g) = runtime(Spec::Guide {
        a: [0.0, 0.0],
        b: [64.0, 0.0],
    })?;
    check(
        near(g.constrain([8.0, 8.0]), [8.0, 0.0]) && near(g.constrain([40.0, 30.0]), [40.0, 0.0]),
        "Fixed guide constraint",
    )?;
    let (_, mut p) = runtime(Spec::ParallelCurve {
        points: vec![[0.0, 0.0], [32.0, 32.0], [64.0, 0.0]],
    })?;
    check(
        near(p.constrain([32.0, 40.0]), [32.0, 40.0]),
        "Parallel curve lost signed normal offset at knot",
    )?;
    let q = p.constrain([48.0, 40.0]);
    check(
        q[0] > 32.0 && q[1] < 40.0,
        "Parallel curve flattened to a straight line",
    )?;
    let (_, mut p) = runtime(Spec::MultipleCurve {
        curves: vec![
            vec![[0.0, 0.0], [64.0, 0.0]],
            vec![[0.0, 32.0], [64.0, 32.0]],
        ],
    })?;
    check(
        near(p.constrain([8.0, 28.0]), [8.0, 32.0]) && near(p.constrain([40.0, 3.0]), [40.0, 32.0]),
        "Multiple curve did not lock nearest curve",
    )?;
    let (_, mut p) = runtime(Spec::RadialLine {
        center: [32.0, 32.0],
        guide: [64.0, 32.0],
    })?;
    check(
        near(p.constrain([16.0, 16.0]), [16.0, 16.0])
            && near(p.constrain([48.0, 20.0]), [34.0, 34.0]),
        "Radial line did not keep initial ray",
    )?;
    let (_, mut p) = runtime(Spec::RadialCurve {
        center: [0.0, 0.0],
        points: vec![[0.0, 0.0], [64.0, 0.0]],
    })?;
    check(
        near(p.constrain([0.0, 16.0]), [0.0, 16.0]) && near(p.constrain([20.0, 32.0]), [0.0, 32.0]),
        "Radial curve rotation/radius anchor",
    )?;
    let circle = Spec::ConcentricCircle {
        center: [32.0, 32.0],
        radius: 20.0,
    };
    let (_, mut p) = runtime(circle.clone())?;
    check(
        near(p.constrain([48.0, 32.0]), [48.0, 32.0])
            && near(p.constrain([32.0, 60.0]), [32.0, 48.0])
            && near(p.constrain([32.0, 32.0]), [32.0, 48.0]),
        "Concentric circle radius/zero-vector handling",
    )?;
    let equal = Spec::Division {
        a: [0.0, 0.0],
        b: [64.0, 0.0],
        marks: Marks::Equal(4),
        snap_ticks: true,
    };
    check(
        equal.tick_points()
            == vec![
                [0.0, 0.0],
                [16.0, 0.0],
                [32.0, 0.0],
                [48.0, 0.0],
                [64.0, 0.0],
            ],
        "Equal division positions",
    )?;
    let (_, mut p) = runtime(equal)?;
    check(
        near(p.constrain([17.0, 0.5]), [16.0, 0.0]) && near(p.constrain([20.0, 3.0]), [20.0, 0.0]),
        "Scale tick distance/continuous baseline",
    )?;
    let golden = Spec::Division {
        a: [0.0, 0.0],
        b: [64.0, 0.0],
        marks: Marks::Golden(1),
        snap_ticks: false,
    };
    let ticks = golden.tick_points();
    check(
        ticks.len() == 4
            && (ticks[1][0] - 24.4458247200067).abs() < 1e-8
            && (ticks[2][0] - 39.5541752799933).abs() < 1e-8,
        "Golden ratio marks",
    )?;
    let mut perspective = Perspective::default();
    perspective.points[0].fixed = true;
    let old = perspective.clone();
    check(
        perspective.move_point(1, [2.0, 40.0]).is_err() && perspective == old,
        "Fixed vanishing point moved",
    )?;
    perspective.lock_horizon(true)?;
    perspective.add_point([20.0, 60.0])?;
    check(
        perspective.points.last().unwrap().point == [20.0, 32.0],
        "Eye-height lock did not constrain new point",
    )?;
    perspective.add_guide(1, [32.0, 48.0])?;
    let gid = perspective.guides[0].id;
    let old = perspective.clone();
    check(
        perspective.add_guide(999, [0.0, 0.0]).is_err() && perspective == old,
        "Invalid linked guide mutated perspective",
    )?;
    perspective.remove_guide(gid);
    check(perspective.guides.is_empty(), "Delete guide failed")?;
    perspective.add_guide(1, [32.0, 48.0])?;
    perspective.remove_point(1)?;
    check(
        perspective.guides.is_empty(),
        "Deleted VP left dangling guides",
    )?;
    let mut infinite = Perspective::default();
    infinite.infinitize(2, true)?;
    let (_, mut p) = runtime(Spec::Perspective(infinite))?;
    p.constrain([32.0, 48.0]);
    check(
        near(p.constrain([48.0, 47.0]), [48.0, 48.0]),
        "Infinite VP did not preserve parallel direction",
    )?;
    let (_, mut p) = runtime(Spec::Perspective(Perspective::default()))?;
    p.constrain([32.0, 48.0]);
    let q = p.constrain([48.0, 38.0]);
    check(
        ((q[0] - 32.0) * -16.0 - (q[1] - 48.0) * 32.0).abs() < 1e-6,
        "Perspective ray did not pass through chosen VP",
    )?;
    let fish = Perspective {
        fisheye: true,
        ..Default::default()
    };
    let (_, mut p) = runtime(Spec::Perspective(fish.clone()))?;
    p.constrain([32.0, 16.0]);
    let q = p.constrain([48.0, 20.0]);
    check(
        ((q[0] - 32.0).powi(2) + (q[1] - 56.0).powi(2) - 1600.0).abs() < 1e-5
            && near(p.constrain([100.0, 32.0]), [64.0, 32.0]),
        "Fisheye circle/lens boundary geometry",
    )?;
    let lines = Spec::Perspective(fish.clone()).lines(128.0)?;
    check(
        lines.len() > 96
            && lines
                .iter()
                .flatten()
                .all(|q| q.iter().all(|v| v.is_finite())),
        "Fisheye curved construction guides missing",
    )?;
    let mut fish = fish;
    fish.add_guide(1, [32.0, 16.0])?;
    let (path, closed) = Spec::Perspective(fish).follow()?;
    check(
        !closed
            && path.len() > 2
            && path
                .iter()
                .all(|p| ((p[0] - 32.0).powi(2) + (p[1] - 32.0).powi(2)).sqrt() <= 32.0 + 1e-5),
        "Follow fisheye guide drew a straight line/outside lens",
    )?;
    let mut near_horizon = Perspective {
        fisheye: true,
        ..Default::default()
    };
    near_horizon.add_guide(1, [32.0, 31.999])?;
    let (path, closed) = Spec::Perspective(near_horizon).follow()?;
    check(
        !closed
            && path.len() > 2
            && path.iter().all(|p| {
                p.iter().all(|v| v.is_finite())
                    && ((p[0] - 32.0).powi(2) + (p[1] - 32.0).powi(2)).sqrt() <= 32.0 + 1e-5
            }),
        "Nearly straight fisheye guide lost its narrow arc",
    )?;
    let mut horizon = Perspective {
        fisheye: true,
        ..Default::default()
    };
    horizon.add_guide(1, [32.0, 32.0])?;
    let (path, closed) = Spec::Perspective(horizon).follow()?;
    check(
        !closed && path == vec![[0.0, 32.0], [64.0, 32.0]],
        "Horizon fisheye line failed",
    )?;
    for marks in [
        Marks::Equal(1),
        Marks::Equal(512),
        Marks::Golden(1),
        Marks::Golden(32),
    ] {
        let spec = Spec::Division {
            a: [0.0, 0.0],
            b: [64.0, 0.0],
            marks,
            snap_ticks: true,
        };
        check(
            spec.valid()
                && spec
                    .tick_points()
                    .iter()
                    .all(|p| p[0] >= 0.0 && p[0] <= 64.0),
            "Division limits",
        )?;
    }
    let mut d = make()?;
    d.layers.push(d.layers[0].clone());
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 4,
        tracks: vec![],
        audio: vec![],
    });
    d.scene.rulers.add(
        "Scoped",
        Kind::Special(Spec::Guide {
            a: [0.0, 8.0],
            b: [64.0, 8.0],
        }),
    )?;
    let guide = &mut d.scene.rulers.guides[0];
    guide.scope = Scope::Layer(Target::Raster(0));
    guide.display = Display::AppliedLayers;
    guide.frames = Some([1, 2]);
    d.current_frame = 1;
    let mut own = Runtime::for_target(&d, Target::Raster(0))?;
    let mut other = Runtime::for_target(&d, Target::Raster(1))?;
    check(
        near(own.constrain([20.0, 30.0]), [20.0, 8.0])
            && near(other.constrain([20.0, 30.0]), [20.0, 30.0])
            && rulers::lines_for(&d, &d.scene.rulers.guides[0], Target::Raster(1)).is_empty(),
        "Layer application/display scope",
    )?;
    d.current_frame = 3;
    check(
        near(
            Runtime::for_target(&d, Target::Raster(0))?.constrain([20.0, 30.0]),
            [20.0, 30.0],
        ),
        "Frame range did not disable ruler",
    )?;
    d.current_frame = 1;
    d.scene.rulers.guides[0].display = Display::Hidden;
    check(
        rulers::lines_for(&d, &d.scene.rulers.guides[0], Target::Raster(0)).is_empty()
            && near(
                Runtime::for_target(&d, Target::Raster(0))?.constrain([20.0, 30.0]),
                [20.0, 8.0],
            ),
        "Hidden guide changed snap behavior",
    )?;
    d.scene.rulers.guides[0].display = Display::All;
    d.scene.rulers.guides[0].link = Some(Target::Raster(0));
    let before = d.snapshot();
    let state = ruler_scope::translate_layer(&d, Target::Raster(0), [3.0, 4.0])?;
    check(
        state.scene.rulers.guides[0].offset == [3.0, 4.0]
            && state.layers[0].pixels == before.layers[0].pixels,
        "Linked layer move changed pixels/missed ruler",
    )?;
    d.commit_state(state)?;
    check(
        near(
            Runtime::for_target(&d, Target::Raster(0))?.constrain([20.0, 30.0]),
            [20.0, 12.0],
        ) && d.undo()
            && d.snapshot() == before,
        "Linked ruler move/Undo",
    )?;
    d.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 64.0, 0.0]);
    let mut r = Runtime::for_target(&d, Target::Raster(0))?;
    check(
        near(r.constrain([24.0, 16.0]), [48.0, 16.0]),
        "Ruler page rotation/scale",
    )?;
    let mut folders = make()?;
    folders.layers.push(folders.layers[0].clone());
    folders.layers.push(folders.layers[0].clone());
    folders.scene.paper.folders.push(crate::paper::Folder {
        through: false,
        name: "Cut group".into(),
        children: vec![crate::paper::Node::Raster(0), crate::paper::Node::Raster(1)],
        paper: crate::paper::Paper {
            space: Default::default(),
            fill: Default::default(),
            hidden: false,
            blend: Default::default(),
            seed: 0,
            pattern: Default::default(),
            origin: [0.0; 2],
        },
    });
    folders.scene.paper.roots = vec![crate::paper::Node::Folder(0), crate::paper::Node::Raster(2)];
    let id = folders.scene.rulers.add(
        "Cut scope",
        Kind::Special(Spec::Guide {
            a: [0.0, 8.0],
            b: [64.0, 8.0],
        }),
    )?;
    folders.scene.rulers.guides[0].scope = Scope::Folder(0);
    folders.scene.rulers.guides[0].link = Some(Target::Raster(1));
    check(
        ruler_scope::applies(&folders, &folders.scene.rulers.guides[0], Target::Raster(0))
            && !ruler_scope::applies(&folders, &folders.scene.rulers.guides[0], Target::Raster(2)),
        "Cut/folder scope escaped its layers",
    )?;
    let outside = folders.scene.rulers.add(
        "Outside owner",
        Kind::Straight {
            a: [0.0, 16.0],
            b: [64.0, 16.0],
        },
    )?;
    folders.scene.rulers.guides[1].link = Some(Target::Raster(2));
    folders.scene.rulers.guides[1].scope = Scope::Layer(Target::Raster(2));
    let before = folders.snapshot();
    crate::paper::bake(&mut folders, crate::paper::Node::Folder(0))?;
    check(
        folders
            .scene
            .rulers
            .guides
            .iter()
            .find(|g| g.id == id)
            .unwrap()
            .link
            == Some(Target::Raster(0))
            && folders
                .scene
                .rulers
                .guides
                .iter()
                .find(|g| g.id == outside)
                .unwrap()
                .link
                == Some(Target::Raster(1))
            && folders.undo()
            && folders.snapshot() == before,
        "Folder bake did not preserve linked ruler owners/Undo",
    )?;
    let mut vector = make()?;
    let layer = vector.scene.lines.add_layer("Owned")?;
    let state = crate::shapes::prepare(
        &vector,
        crate::shapes::Target::Vector(Some(layer)),
        &[[8.0, 8.0], [24.0, 24.0]],
        Default::default(),
    )?;
    vector.commit_state(state)?;
    vector.scene.rulers.add(
        "Linked vector",
        Kind::Special(Spec::Guide {
            a: [0.0, 8.0],
            b: [64.0, 8.0],
        }),
    )?;
    vector.scene.rulers.guides[0].link = Some(Target::Vector(layer));
    let before = vector.snapshot();
    vector.commit_state(ruler_scope::translate_layer(
        &vector,
        Target::Vector(layer),
        [5.0, -2.0],
    )?)?;
    check(
        vector.scene.rulers.guides[0].offset == [5.0, -2.0]
            && vector.scene.lines.layers[0].paths[0].points[0] == [13.0, 6.0]
            && vector.undo()
            && vector.snapshot() == before,
        "Vector layer link/definition/Undo",
    )?;
    for vector in [false, true] {
        let mut d = make()?;
        let target = if vector {
            crate::shapes::Target::Vector(Some(d.scene.lines.add_layer("Follow target")?))
        } else {
            crate::shapes::Target::Raster(0)
        };
        let id = d.scene.rulers.add(
            "Circle",
            Kind::Special(Spec::ConcentricCircle {
                center: [32.0, 32.0],
                radius: 8.0,
            }),
        )?;
        let before = d.snapshot();
        let selected = crate::selection_shortcuts::prepare(
            &d,
            &crate::selection_shortcuts::Source::Rulers(vec![id]),
            crate::selection_edit::Combine::Replace,
            false,
        )?;
        check(
            selected.selection_coverage([32.5, 32.5]) == 255
                && selected.selection_coverage([0.5, 0.5]) == 0,
            "Closed circle ruler to selection",
        )?;
        d.commit_state(ruler_scope::follow(
            &d,
            id,
            target,
            2.0,
            [200, 40, 20, 255],
        )?)?;
        check(
            d.history_counts().0 == 1,
            "Follow ruler did not use one Undo",
        )?;
        if vector {
            check(
                d.scene.lines.layers[0].paths[0].closed && d.layers == before.layers,
                "Follow ruler editable vector result",
            )?;
        } else {
            check(
                d.layers[0].pixel(64, 64, 32, 32)[3] == 0
                    && d.layers[0].pixel(64, 64, 40, 32)[3] > 0,
                "Follow ruler raster ring result",
            )?;
        }
        let file = root.join(format!("special-ruler-{vector}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot()
                && d.undo()
                && d.snapshot() == before,
            "Special ruler native/Undo",
        )?;
    }
    check(
        !Spec::ParallelCurve { points: vec![] }.valid()
            && !Spec::Division {
                a: [0.0, 0.0],
                b: [1.0, 0.0],
                marks: Marks::Equal(0),
                snap_ticks: false,
            }
            .valid(),
        "Invalid curve/division accepted",
    )?;
    let (_, mut bad) = runtime(Spec::Guide {
        a: [0.0, 0.0],
        b: [64.0, 0.0],
    })?;
    bad.constrain([f64::NAN, 0.0]);
    check(
        bad.check_snapping().is_err(),
        "Invalid ruler sample did not cancel",
    )?;
    let _ = id;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn families_scope_links_vanishing_fisheye_follow_undo_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
