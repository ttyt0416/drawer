//! Independent centerline intervals, pivot equations and clipboard round trips.
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, Rect},
    vector_clipboard as clip,
    vector_edit::Selection,
    vector_scope::{self as core, Picked, Transform},
    vector_selection::Control,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let layer = d.scene.lines.add_layer("Scope")?;
    d.scene.lines.add_path(
        layer,
        vec![[4., 12.], [16., 8.], [32., 12.], [48., 12.]],
        false,
        3.,
        [210, 20, 30, 150],
    )?;
    d.scene.lines.add_path(
        layer,
        vec![[10., 30.], [26., 30.], [26., 44.], [10., 44.]],
        true,
        2.,
        [10, 40, 210, 220],
    )?;
    let other = d.scene.lines.add_layer("Curves")?;
    crate::shapes::prepare(
        &d,
        crate::shapes::Target::Vector(Some(other)),
        &[[4., 4.], [12., 20.], [36., 2.], [52., 20.]],
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Bezier,
            curve: crate::shapes::Curve::CubicBezier,
            width: 3.,
            ..Default::default()
        },
    )?
    .restore(&mut d);
    for layer in &mut d.scene.lines.layers {
        for p in &mut layer.paths {
            p.pressure.device = vec![Some(0.2), None, Some(0.8)];
            p.pressure.device_positions = vec![0., 0.37, 1.];
            p.pressure.controls = vec![0.4, 1.8, 0.6];
            p.pressure.control_positions = vec![0., 0.41, 1.];
        }
    }
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let path = d.scene.lines.layers[0].paths[0].id;
    let closed = d.scene.lines.layers[0].paths[1].id;
    let curve_layer = d.scene.lines.layers[1].id;
    let curve = d.scene.lines.layers[1].paths[0].id;
    let point_scope = vec![Picked {
        layer: curve_layer,
        selection: Selection {
            controls: vec![Control {
                path: curve,
                point: 1,
            }],
            paths: vec![],
        },
    }];
    let before = d.snapshot();
    let whole = clip::copy(&d, &point_scope, false)?;
    check(
        whole.paths == vec![d.scene.lines.layers[1].paths[0].clone()] && d.snapshot() == before,
        "Single middle control must copy entire curve unchanged",
    )?;
    let spans = vec![Picked {
        layer: curve_layer,
        selection: Selection {
            controls: vec![
                Control {
                    path: curve,
                    point: 1,
                },
                Control {
                    path: curve,
                    point: 2,
                },
            ],
            paths: vec![],
        },
    }];
    check(
        clip::copy(&d, &point_scope, true).is_err(),
        "Isolated span anchor accepted silently",
    )?;
    let part = clip::copy(&d, &spans, true)?;
    let original = &d.scene.lines.layers[1].paths[0];
    let p = &part.paths[0];
    check(
        p.definition.is_none() && !p.closed && p.points.len() >= 2,
        "Span is not an editable open outline",
    )?;
    for (t, old) in [(0., 1. / 3.), (1., 2. / 3.)] {
        check(
            (p.pressure.control_at(t) - original.pressure.control_at(old)).abs() < 1e-9
                && (p.pressure.device_at(t) - original.pressure.device_at(old)).abs() < 1e-9,
            "Span endpoint pressure interpolation",
        )?;
    }
    check(
        whole.raster()?.image.width > part.raster()?.image.width,
        "Whole/span raster bounds did not differ",
    )?;
    let mut dense = whole.paths[0].clone();
    dense.points = vec![[0., 0.], [256., 256.]];
    dense.definition = None;
    dense.pressure.controls = (0..8192)
        .map(|n| if n % 2 == 0 { 0.5 } else { 1.5 })
        .collect();
    dense.pressure.control_positions.clear();
    check(
        clip::Packet {
            transform: Affine::default(),
            paths: vec![dense],
        }
        .raster()
        .is_err(),
        "Raster work bound ignored dense pressure knots",
    )?;
    let raw = whole.encode()?;
    check(
        clip::Packet::decode(&raw)? == whole,
        "Vector clipboard serialization",
    )?;
    for bad in [
        b"no packet".to_vec(),
        b"DRAWVEC1\n{}".to_vec(),
        vec![0; 4 * 1024 * 1024 + 1],
    ] {
        check(
            clip::Packet::decode(&bad).is_err(),
            "Malformed/oversize vector clipboard accepted",
        )?;
    }
    let (state, new) = clip::paste(&d, layer, &whole, [3., 4.])?;
    let pasted = state.scene.lines.layers[0].paths.last().unwrap();
    check(
        !before
            .scene
            .lines
            .layers
            .iter()
            .flat_map(|l| &l.paths)
            .any(|p| p.id == pasted.id)
            && new.selection.paths == [pasted.id],
        "Pasted IDs not fresh",
    )?;
    let mut expected = whole.paths[0].clone();
    core::whole(&mut expected, Affine([1., 0., 0., 1., 3., 4.]));
    expected.id = pasted.id;
    check(
        *pasted == expected && state.scene.lines.layers[1] == before.scene.lines.layers[1],
        "Paste geometry/handles/style or source isolation",
    )?;
    d.commit_state(state.clone())?;
    let file = root.join("vector-scope.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == state
            && d.undo()
            && d.snapshot() == before,
        "Vector paste native/one Undo",
    )?;
    let selected = vec![Picked {
        layer,
        selection: Selection {
            controls: vec![Control { path, point: 1 }, Control { path, point: 2 }],
            paths: vec![],
        },
    }];
    let cut = clip::cut(&d, &selected, true)?;
    let paths = &cut.scene.lines.layers[0].paths;
    check(
        paths.len() == 3
            && paths[0].id == path
            && paths[0].points == [[4., 12.], [16., 8.]]
            && paths[1].points == [[32., 12.], [48., 12.]]
            && paths[2].id == closed,
        "Span cut failed to retain both outside ranges",
    )?;
    d.commit_state(cut)?;
    check(
        d.undo() && d.snapshot() == before,
        "Span cut single Undo connectivity",
    )?;
    let wrap = vec![Picked {
        layer,
        selection: Selection {
            controls: vec![
                Control {
                    path: closed,
                    point: 3,
                },
                Control {
                    path: closed,
                    point: 0,
                },
            ],
            paths: vec![],
        },
    }];
    let p = clip::copy(&d, &wrap, true)?.paths.remove(0);
    check(
        p.points.first() == Some(&[10., 44.]) && p.points.last() == Some(&[10., 30.]),
        "Closed path wrap span endpoints",
    )?;
    let all = vec![
        Picked {
            layer,
            selection: Selection {
                controls: vec![Control { path, point: 1 }],
                paths: vec![closed],
            },
        },
        Picked {
            layer: curve_layer,
            selection: Selection {
                controls: vec![Control {
                    path: curve,
                    point: 2,
                }],
                paths: vec![],
            },
        },
    ];
    check(
        clip::copy(&d, &all, false)?.paths.len() == 3,
        "Mixed/noncontiguous multi-layer copy scope",
    )?;
    let whole_cut = clip::cut(&d, &all, false)?;
    check(
        whole_cut
            .scene
            .lines
            .layers
            .iter()
            .all(|l| l.paths.is_empty()),
        "Whole-stroke cut omitted a point-selected curve",
    )?;
    for duplicate in [false, true] {
        let settings = Transform {
            pivot: [16., 8.],
            translation: [3., 4.],
            scale: [2., 0.5],
            degrees: 90.,
            duplicate,
        };
        let (after, new) = core::transform(&d, &selected, settings)?;
        let p = if duplicate {
            after.scene.lines.layers[0].paths.last().unwrap()
        } else {
            &after.scene.lines.layers[0].paths[0]
        };
        // Independent rotate90(scale(point-pivot))+pivot+translation.
        check(
            p.points[0] == [4., 12.]
                && p.points[3] == [48., 12.]
                && (p.points[1][0] - 19.).abs() < 1e-9
                && (p.points[1][1] - 12.).abs() < 1e-9
                && (p.points[2][0] - 17.).abs() < 1e-9
                && (p.points[2][1] - 44.).abs() < 1e-9,
            "Partial pivot transform changed outside anchors or wrong matrix",
        )?;
        check(
            p.pressure == before.scene.lines.layers[0].paths[0].pressure
                && p.color == before.scene.lines.layers[0].paths[0].color
                && p.width == 3.,
            "Partial transform changed pressure/style",
        )?;
        if duplicate {
            check(
                after.scene.lines.layers[0].paths[..2] == before.scene.lines.layers[0].paths
                    && p.id != path
                    && new[0].selection.controls.iter().all(|c| c.path == p.id),
                "Duplicate changed originals/identity",
            )?;
        }
        check(d.snapshot() == before, "Transform preview mutated source")?;
        d.commit_state(after.clone())?;
        check(
            d.undo()
                && d.snapshot() == before
                && d.redo()
                && d.snapshot() == after
                && d.undo()
                && d.snapshot() == before,
            "Transform one Undo/Redo",
        )?;
    }
    // A single selected node rotates its handles; unrelated nodes stay fixed.
    let mut nodes = crate::vector_nodes_verify::fixture()?;
    let l = nodes.scene.lines.layers[0].id;
    let id = nodes.scene.lines.layers[0].paths[0].id;
    crate::vector_nodes::prepare(
        &nodes,
        l,
        crate::vector_edit::Hit {
            path: id,
            control: Some(1),
            parameter: None,
            point: [0.; 2],
            distance: 0.,
        },
        crate::vector_nodes::Operation::Smooth,
    )?
    .restore(&mut nodes);
    let original = nodes.scene.lines.layers[0].paths[0].clone();
    let s = [Picked {
        layer: l,
        selection: Selection {
            controls: vec![Control { path: id, point: 1 }],
            paths: vec![],
        },
    }];
    let (after, _) = core::transform(
        &nodes,
        &s,
        Transform {
            pivot: [16., 4.],
            degrees: 90.,
            ..Default::default()
        },
    )?;
    let p = &after.scene.lines.layers[0].paths[0];
    check(
        p.control_point(0) == original.control_point(0)
            && p.control_point(2) == original.control_point(2)
            && p.control_point(1) == original.control_point(1),
        "Single-node pivot moved unrelated anchors",
    )?;
    let old = original
        .definition
        .as_ref()
        .unwrap()
        .nodes
        .as_ref()
        .unwrap()
        .incoming[1];
    let new = p
        .definition
        .as_ref()
        .unwrap()
        .nodes
        .as_ref()
        .unwrap()
        .incoming[1];
    check(
        (new[0] + old[1]).abs() < 1e-9 && (new[1] - old[0]).abs() < 1e-9,
        "Selected node handles did not rotate",
    )?;
    d.scene.transform = Affine([2., 0.2, 0.4, 1.5, 3., 5.]);
    let before = d.snapshot();
    let scope = [Picked {
        layer: curve_layer,
        selection: Selection {
            controls: vec![],
            paths: vec![curve],
        },
    }];
    let settings = Transform {
        pivot: [20., 10.],
        translation: [4., -2.],
        degrees: 90.,
        ..Default::default()
    };
    let (after, _) = core::transform(&d, &scope, settings)?;
    for i in 0..4 {
        let old = d.scene.transform.map(
            before.scene.lines.layers[1].paths[0]
                .control_point(i)
                .unwrap(),
        );
        let actual = d.scene.transform.map(
            after.scene.lines.layers[1].paths[0]
                .control_point(i)
                .unwrap(),
        );
        check(
            (actual[0] - (20. - (old[1] - 10.) + 4.)).abs() < 1e-9
                && (actual[1] - (10. + (old[0] - 20.) - 2.)).abs() < 1e-9,
            "Whole curve nonuniform scene pivot transform",
        )?;
    }
    check(
        clip::paste(&d, layer, &whole, [0.; 2]).is_err(),
        "Incompatible paste frame silently distorted tips",
    )?;
    let mut native_selection = fixture()?;
    native_selection.scene.lines.selection = crate::vector_selection::State {
        layer: Some(layer),
        kind: crate::vector_selection::Kind::Points,
        points: vec![
            Control { path, point: 1 },
            Control {
                path: closed,
                point: 1,
            },
        ],
        ..Default::default()
    };
    let before = native_selection.snapshot();
    let after = clip::cut(&native_selection, &selected, true)?;
    check(
        after.scene.lines.selection.points
            == [Control {
                path: closed,
                point: 1,
            }],
        "Cut retained stale control indices or cleared unrelated selection",
    )?;
    native_selection.commit_state(after)?;
    check(
        native_selection.undo() && native_selection.snapshot() == before,
        "Cut selection cleanup did not Undo with geometry",
    )?;
    area_verify()?;
    Ok(())
}
fn area_verify() -> Result<(), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let a = d.scene.lines.add_layer("Area A")?;
    let b = d.scene.lines.add_layer("Area B")?;
    let crossing =
        d.scene
            .lines
            .add_path(a, vec![[0., 15.], [30., 15.]], false, 1., [0, 0, 0, 255])?;
    let inside =
        d.scene
            .lines
            .add_path(a, vec![[12., 12.], [18., 18.]], false, 1., [0, 0, 0, 255])?;
    let boundary =
        d.scene
            .lines
            .add_path(a, vec![[10., 20.], [20., 20.]], false, 1., [0, 0, 0, 255])?;
    let wide =
        d.scene
            .lines
            .add_path(a, vec![[10., 22.], [20., 22.]], false, 8., [0, 0, 0, 255])?;
    let looped = d.scene.lines.add_path(
        b,
        vec![[12., 12.], [18., 12.], [18., 18.]],
        true,
        1.,
        [0, 0, 0, 255],
    )?;
    d.scene.selection = Some(Rect {
        min: [10., 10.],
        max: [20., 20.],
    });
    let before = d.snapshot();
    let history = d.history_counts();
    let overlap = core::area(&d, &[a, b], false)?;
    check(
        overlap[0].selection.paths == [crossing, inside, boundary]
            && overlap[1].selection.paths == [looped]
            && !overlap[0].selection.paths.contains(&wide),
        "Area overlap used stroke width or missed boundary/crossing",
    )?;
    let contained = core::area(&d, &[a, b], true)?;
    check(
        contained[0].selection.paths == [inside, boundary]
            && contained[1].selection.paths == [looped],
        "Area containment geometry",
    )?;
    check(
        d.snapshot() == before && d.history_counts() == history,
        "Area selection changed document/Undo",
    )?;
    d.scene.lines.layers[1].locked = true;
    check(
        core::area(&d, &[a, b], false)?.len() == 1,
        "Locked layer selected",
    )?;
    d.scene.lines.layers[1].locked = false;
    d.scene.lines.layers[1].hidden = true;
    check(
        core::area(&d, &[a, b], false)?.len() == 1,
        "Hidden layer selected",
    )?;
    d.scene.lines.layers[0].paths[1].points = vec![[12., 15.5], [18., 15.5]];
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![
            crate::selection::Run {
                x: 10,
                y: 15,
                length: 4,
                alpha: 255,
            },
            crate::selection::Run {
                x: 16,
                y: 15,
                length: 4,
                alpha: 255,
            },
        ],
    });
    check(
        core::area(&d, &[a], false)?[0]
            .selection
            .paths
            .contains(&inside)
            && core::area(&d, &[a], true)?.is_empty(),
        "Mask hole missed between inside endpoints",
    )?;
    d.scene.selection_mask = Some(crate::selection::Mask { runs: vec![] });
    check(
        core::area(&d, &[a], false)?.is_empty(),
        "Explicit empty mask selected strokes",
    )?;
    d.scene.selection_mask = None;
    d.scene.selection = Some(Rect {
        min: [10., 10.],
        max: [20., 20.],
    });
    d.scene.transform = Affine([2., 0., 0., 1.5, 3., 4.]);
    d.scene.selection_translation = [4., 0.];
    d.scene.lines.layers[0].paths[1].points = vec![[12., 12.], [22., 18.]];
    check(
        core::area(&d, &[a], true)?[0]
            .selection
            .paths
            .contains(&inside),
        "Nonuniform scene/translated area coordinates",
    )?;
    d.scene.selection_translation = [0.; 2];
    check(
        !core::area(&d, &[a], true)?
            .iter()
            .any(|p| p.selection.paths.contains(&inside)),
        "Translated area selection was ignored",
    )?;
    d.scene.selection = None;
    check(core::area(&d, &[a], false).is_err(), "No area accepted")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_scope_pivot_copy_spans_cut_area_identity_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
