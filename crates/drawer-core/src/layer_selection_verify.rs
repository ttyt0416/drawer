use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_selection::{self as alpha, Source},
    scene::{Affine, Rect},
    selection::{Mask, Run},
    selection_edit::Combine,
    vector_selection::{self as vector, Control, Kind},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make(w: u32, h: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn region() -> Rect {
    Rect {
        min: [10.0, 10.0],
        max: [20.0, 12.0],
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for op in Combine::ALL {
        for (b, expected) in match op {
            Combine::Replace => [(0, 0), (128, 128), (255, 255)],
            Combine::Add => [(0, 128), (128, 192), (255, 255)],
            Combine::Subtract => [(0, 128), (128, 64), (255, 0)],
            Combine::Intersect => [(0, 0), (128, 64), (255, 128)],
        } {
            let mut d = make(3, 1)?;
            d.layers[0].pixels.fill([20, 40, 60, b]);
            d.scene.selection_mask = Some(Mask {
                runs: vec![Run {
                    x: 0,
                    y: 0,
                    length: 3,
                    alpha: 128,
                }],
            });
            let before = d.snapshot();
            let scene = alpha::prepare(&d, Source::Raster(0), op)?;
            check(
                d.snapshot() == before && scene.selection_coverage([1.5, 0.5]) == expected,
                "Layer alpha zero/half/full combine numeric result",
            )?;
            let changed = d.edit_scene(scene)?;
            check(
                d.layers == before.layers && changed == (expected != 128),
                "Layer alpha pixels/no-op semantics",
            )?;
            if changed {
                check(
                    d.history_counts().0 == 1 && d.undo() && d.snapshot() == before && d.redo(),
                    "Layer alpha one Undo/Redo",
                )?;
            } else {
                check(
                    d.history_counts() == (0, 0) && d.snapshot() == before,
                    "No-op alpha selection created history",
                )?;
            }
            let path = root.join(format!("layer-alpha-{op:?}-{b}.drawer"));
            crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            check(
                crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == d.snapshot(),
                "Layer alpha native reopen",
            )?;
        }
    }
    let mut d = make(4, 2)?;
    d.layers[0].pixels.fill([30, 40, 50, 128]);
    for (op, expected) in [
        (Combine::Replace, 128),
        (Combine::Add, 128),
        (Combine::Subtract, 127),
        (Combine::Intersect, 128),
    ] {
        check(
            alpha::prepare(&d, Source::Raster(0), op)?.selection_coverage([1.5, 0.5]) == expected,
            "No-selection alpha combine baseline",
        )?;
    }
    d.layers[0].locked = true;
    d.scene.paper.layers.entry(0).or_default().hidden = true;
    check(
        alpha::prepare(&d, Source::Raster(0), Combine::Replace)?.selection_coverage([1.5, 0.5])
            == 128,
        "Hidden/locked layer alpha was ignored",
    )?;
    d.layers[0].locked = false;
    d.scene.paper.layers.clear();
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 5,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    d.scene.selection_translation = [3.0, 0.0];
    let scene = alpha::prepare(&d, Source::Raster(0), Combine::Add)?;
    check(
        scene.selection_coverage([8.5, 0.5]) == 128 && scene.selection_translation == [0.0; 2],
        "Alpha add lost moved outside selection",
    )?;
    d.scene.selection_mask = None;
    d.scene.selection_translation = [0.0; 2];
    d.layers[0].pixels = vec![[30, 40, 50, 128]; 6];
    d.layers[0].placement = Some(PlacedRaster {
        width: 3,
        height: 2,
        offset: [-2, 1],
    });
    let s = std::f64::consts::FRAC_1_SQRT_2;
    d.scene.transform = Affine([s, s, -s, s, 0.0, 0.0]);
    check(
        alpha::prepare(&d, Source::Raster(0), Combine::Replace)?.selection_coverage([-1.9, 2.9])
            > 0,
        "Rotated grid clipped off-canvas raster alpha corners",
    )?;
    let mut v = make(16, 16)?;
    v.commit_state(crate::shapes::prepare(
        &v,
        crate::shapes::Target::Vector(None),
        &[[4.0, 4.0], [12.0, 12.0]],
        crate::shapes::Settings {
            paint: crate::shapes::Paint::CreateFill,
            fill: [200, 40, 20, 128],
            ..Default::default()
        },
    )?)?;
    let id = v.scene.lines.layers[0].id;
    v.scene.lines.layers[0].hidden = true;
    v.scene.lines.layers[0].locked = true;
    let before = v.snapshot();
    check(
        alpha::prepare(&v, Source::Vector(id), Combine::Replace)?.selection_coverage([6.5, 6.5])
            == 128
            && v.snapshot() == before,
        "Vector raw alpha altered/ignored hidden source",
    )?;
    let mut g = make(16, 16)?;
    g.scene.selection = Some(Rect {
        min: [4.0, 4.0],
        max: [12.0, 12.0],
    });
    g.commit_state(crate::gradient::prepare(
        &g,
        0,
        crate::gradient::Settings {
            first: [200, 0, 0, 128],
            last: [0, 0, 200, 128],
            ..Default::default()
        },
        true,
        Default::default(),
    )?)?;
    g.scene.selection = None;
    g.scene.gradients.layers[0].hidden = true;
    let id = g.scene.gradients.layers[0].id;
    check(
        alpha::prepare(&g, Source::Gradient(id), Combine::Replace)?.selection_coverage([6.5, 6.5])
            == 128,
        "Editable gradient alpha",
    )?;
    let mut d = make(32, 24)?;
    let layer = d.scene.lines.add_layer("Area conversion")?;
    let cross = d.scene.lines.add_path(
        layer,
        vec![[-4.0, 10.5], [36.0, 10.5]],
        false,
        1.0,
        [20, 40, 60, 128],
    )?;
    let wide = d.scene.lines.add_path(
        layer,
        vec![[11.0, 14.5], [19.0, 14.5]],
        false,
        6.0,
        [80, 40, 60, 255],
    )?;
    let boundary = d.scene.lines.add_path(
        layer,
        vec![[10.0, 10.0], [20.0, 10.0]],
        false,
        1.0,
        [20, 140, 60, 255],
    )?;
    let right = d.scene.lines.add_path(
        layer,
        vec![[20.0, 10.0], [20.0, 11.0]],
        false,
        1.0,
        [20, 40, 160, 255],
    )?;
    let corner =
        d.scene
            .lines
            .add_path(layer, vec![[21.0, 13.0]; 2], false, 2.0, [20, 40, 60, 255])?;
    d.scene.selection = Some(region());
    let before = d.snapshot();
    let scene = vector::prepare(&d, layer, Kind::Points, false, Combine::Replace)?;
    check(
        scene.lines.selection.points
            == vec![Control {
                path: boundary,
                point: 0,
            }]
            && scene.lines.selection.paths.is_empty(),
        "Control-point area/boundary selection",
    )?;
    check(
        d.snapshot() == before
            && scene.lines.layers == before.scene.lines.layers
            && scene.selection == before.scene.selection,
        "Vector conversion modified geometry/area",
    )?;
    let center = vector::prepare(&d, layer, Kind::Strokes, false, Combine::Replace)?;
    check(
        center.lines.selection.paths.contains(&cross)
            && center.lines.selection.paths.contains(&boundary)
            && !center.lines.selection.paths.contains(&wide)
            && !center.lines.selection.paths.contains(&right),
        "Crossing centerline selection with endpoints outside",
    )?;
    let thickness = vector::prepare(&d, layer, Kind::Strokes, true, Combine::Replace)?;
    check(
        thickness.lines.selection.paths.contains(&wide)
            && thickness.lines.selection.paths.contains(&right)
            && !thickness.lines.selection.paths.contains(&corner),
        "Rendered width/circular corner contact separation",
    )?;
    d.edit_scene(scene)?;
    let selected = d.scene.lines.selection.clone();
    d.scene.selection = None;
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 11,
            y: 14,
            length: 1,
            alpha: 128,
        }],
    });
    let add = vector::prepare(&d, layer, Kind::Points, false, Combine::Add)?;
    check(
        add.lines.selection.points.contains(&Control {
            path: boundary,
            point: 0,
        }) && add.lines.selection.points.contains(&Control {
            path: wide,
            point: 0,
        }),
        "Partial alpha nonzero control add",
    )?;
    d.edit_scene(add)?;
    let subtract = vector::prepare(&d, layer, Kind::Points, false, Combine::Subtract)?;
    check(
        subtract.lines.selection.points == selected.points,
        "Control subtraction",
    )?;
    let intersection = vector::prepare(&d, layer, Kind::Points, false, Combine::Intersect)?;
    check(
        intersection.lines.selection.points
            == vec![Control {
                path: wide,
                point: 0,
            }],
        "Control intersection",
    )?;
    d.scene.selection_mask = Some(Mask::default());
    let empty = vector::prepare(&d, layer, Kind::Strokes, true, Combine::Replace)?;
    check(
        empty.lines.selection.paths.is_empty() && empty.lines.selection.points.is_empty(),
        "Empty area conversion",
    )?;
    d.scene.selection_mask = None;
    d.scene.selection = Some(Rect {
        min: [10.0, 10.0],
        max: [10.0, 10.0],
    });
    check(
        vector::prepare(&d, layer, Kind::Strokes, true, Combine::Replace)?
            .lines
            .selection
            .paths
            .is_empty(),
        "Zero-area rectangle selected thick strokes",
    )?;
    d.scene.selection = None;
    let before = d.snapshot();
    check(
        vector::prepare(&d, layer, Kind::Points, false, Combine::Replace).is_err()
            && d.snapshot() == before,
        "No area silently selected all controls",
    )?;
    d.scene.selection = Some(region());
    for hidden in [false, true] {
        d.scene.lines.layers[0].hidden = hidden;
        d.scene.lines.layers[0].locked = !hidden;
        let before = d.snapshot();
        check(
            vector::prepare(&d, layer, Kind::Strokes, true, Combine::Replace).is_err()
                && d.snapshot() == before,
            "Hidden/locked vector editing selection changed",
        )?;
    }
    d.scene.lines.layers[0].hidden = false;
    d.scene.lines.layers[0].locked = false;
    d.scene.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    d.scene.selection_translation = [6.0, 0.0];
    let transformed = vector::prepare(&d, layer, Kind::Points, false, Combine::Replace)?;
    check(
        transformed.lines.selection.points.contains(&Control {
            path: boundary,
            point: 1,
        }) && !transformed.lines.selection.points.contains(&Control {
            path: boundary,
            point: 0,
        }),
        "Transformed/moved area control coordinates",
    )?;
    let before = d.snapshot();
    let history = d.history_counts();
    d.edit_scene(transformed)?;
    let path = root.join("vector-edit-selection.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.history_counts().0 == history.0 + 1
            && d.undo()
            && d.snapshot() == before
            && d.redo(),
        "Vector selection one Undo/native reopen",
    )?;
    let selected = d.snapshot();
    let area = d.scene.selection;
    d.edit_scene(vector::clear(&d))?;
    check(
        d.scene.lines.selection.is_default()
            && d.scene.selection == area
            && d.undo()
            && d.snapshot() == selected,
        "Vector selection cancellation changed area/ink",
    )?;
    d.scene.selection = None;
    d.scene.selection_mask = None;
    d.scene.selection_translation = [0.0; 2];
    d.scene.lines.selection = vector::State {
        layer: Some(layer),
        kind: Kind::Strokes,
        include_width: false,
        points: vec![],
        paths: vec![cross, boundary],
    };
    d.scene.lines.settings.erase = vec![crate::line_tools::Target::Vector(layer)];
    d.scene.transform = Affine::default();
    let before = d.snapshot();
    crate::line_tools::erase(&mut d, [15.0, 10.5], 0.1)?;
    check(
        d.scene.lines.selection.paths.is_empty()
            && d.scene.valid()
            && d.undo()
            && d.snapshot() == before,
        "Path erase did not prune/recover editing selection",
    )?;
    let mut curve = make(32, 24)?;
    curve.commit_state(crate::shapes::prepare(
        &curve,
        crate::shapes::Target::Vector(None),
        &[[5.0, 5.0], [5.0, 20.0], [25.0, 20.0], [25.0, 5.0]],
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Bezier,
            curve: crate::shapes::Curve::CubicBezier,
            ..Default::default()
        },
    )?)?;
    let layer = curve.scene.lines.layers[0].id;
    let id = curve.scene.lines.layers[0].paths[0].id;
    curve.scene.selection = Some(Rect {
        min: [13.0, 15.0],
        max: [17.0, 18.0],
    });
    check(
        vector::prepare(&curve, layer, Kind::Points, false, Combine::Replace)?
            .lines
            .selection
            .points
            .is_empty()
            && vector::prepare(&curve, layer, Kind::Strokes, false, Combine::Replace)?
                .lines
                .selection
                .paths
                == vec![id],
        "Bezier crossing missed without an internal control point",
    )?;
    curve.scene.selection = Some(Rect {
        min: [4.0, 19.0],
        max: [6.0, 21.0],
    });
    let controls = vector::prepare(&curve, layer, Kind::Points, false, Combine::Replace)?;
    check(
        controls.lines.selection.points == vec![Control { path: id, point: 1 }],
        "Authored Bezier handles confused with tessellated vertices",
    )?;
    let mut snapshot = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.selection_outline = false;
    let path = root.join("selection-outline-workspace.json");
    snapshot.export(&path).map_err(|e| e.to_string())?;
    check(
        !crate::workspaces::Snapshot::read(&path)
            .map_err(|e| e.to_string())?
            .selection_outline,
        "Selection outline preference reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn alpha_modes_sources_and_vector_area_edit_selection_persistence() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
