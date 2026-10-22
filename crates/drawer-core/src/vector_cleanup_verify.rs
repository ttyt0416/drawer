use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    vector_cleanup::{self as core, Trace},
    vector_edit::Selection,
    vector_scope::Picked,
    vector_selection::Control,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn blank(w: u32, h: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn saved(d: &mut Document, root: &std::path::Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    crate::files::save(d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot(),
        "Vector cleanup native roundtrip",
    )
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    tracing(root)?;
    short_lengths(root)?;
    joins(root)
}
pub fn fixture() -> Result<Document, String> {
    let mut d = blank(32, 24)?;
    for y in 2..22 {
        for x in 2..30 {
            if (7..14).contains(&x) && (7..17).contains(&y) {
                continue;
            }
            d.layers[0].pixels[y * 32 + x] = if x < 18 {
                [200, 20, 30, 128]
            } else {
                [20, 50, 210, 220]
            };
        }
    }
    Ok(d)
}
fn tracing(root: &std::path::Path) -> Result<(), String> {
    // Exhaust every 3x3 binary mask, including diagonal contacts and isolated holes.
    let mut masks = blank(3, 3)?;
    for bits in 1u16..512 {
        for i in 0..9 {
            masks.layers[0].pixels[i] = if bits & (1 << i) != 0 {
                [10, 20, 30, 130]
            } else {
                [0; 4]
            };
        }
        let p = core::trace(&masks, 0, Trace::default())?;
        for i in 0..9 {
            let actual = p.state.scene.lines.layers[0].paths[0]
                .pixel([(i % 3) as f64 + 0.5, (i / 3) as f64 + 0.5]);
            check(
                actual == masks.layers[0].pixels[i],
                "Exhaustive pixel-mask topology oracle",
            )?;
        }
    }
    let mut d = fixture()?;
    d.scene.transform = crate::scene::Affine([2., 0.2, 0.3, 0.75, 4., -2.]);
    let original = d.snapshot();
    let counts = d.history_counts();
    for error in [0., 0.6, 3., 100.] {
        let p = core::trace(
            &d,
            0,
            Trace {
                colors: 2,
                error,
                ..Default::default()
            },
        )?;
        check(
            p.changed == 2
                && p.state.layers == original.layers
                && d.snapshot() == original
                && d.history_counts() == counts,
            "Trace mutated source / lost color groups",
        )?;
        let layer = p.state.scene.lines.layers.last().unwrap();
        for y in 0..24 {
            for x in 0..32 {
                let point = d
                    .scene
                    .transform
                    .inverse()
                    .map([x as f64 + 0.5, y as f64 + 0.5]);
                let actual = layer.paths.iter().fold([0; 4], |color, path| {
                    crate::render::over(color, path.pixel(point))
                });
                let expected = d.layers[0].pixels[y * 32 + x];
                if error == 0. {
                    check(
                        actual == expected,
                        "Zero-error RGBA/hole contour differs from pixel oracle",
                    )?;
                }
            }
        }
        // The explicit central hole stays empty even at very large simplification errors.
        check(
            layer
                .paths
                .iter()
                .all(|p| !p.fill_hit(d.scene.transform.inverse().map([10.5, 10.5]))),
            "Trace simplification filled a hole",
        )?;
    }
    let p = core::trace(
        &d,
        0,
        Trace {
            colors: 2,
            ..Default::default()
        },
    )?;
    d.commit_state(p.state)?;
    check(
        d.history_counts().0 == counts.0 + 1,
        "Trace was not one Undo",
    )?;
    saved(&mut d, root, "raster-trace.drawer")?;
    check(
        d.undo() && d.snapshot() == original && d.redo(),
        "Trace Undo/Redo lost source",
    )?;
    let one = core::trace(
        &fixture()?,
        0,
        Trace {
            colors: 1,
            ..Default::default()
        },
    )?;
    check(
        one.changed == 1
            && one.state.scene.lines.layers.last().unwrap().paths[0]
                .fill
                .unwrap()[3]
                > 128,
        "RGBA color clustering lost alpha",
    )?;
    let mut limits = blank(2, 1)?;
    limits.layers[0].pixels = vec![[100, 100, 100, 128], [100, 100, 100, 127]];
    let p = core::trace(
        &limits,
        0,
        Trace {
            alpha: 128,
            brightness: 100,
            ..Default::default()
        },
    )?;
    let p = &p.state.scene.lines.layers[0].paths[0];
    check(
        p.fill_hit([0.5, 0.5]) && !p.fill_hit([1.5, 0.5]),
        "Inclusive alpha/brightness thresholds",
    )?;
    check(
        core::trace(
            &limits,
            0,
            Trace {
                brightness: 99,
                ..Default::default()
            },
        )
        .is_err(),
        "Brightness threshold ignored",
    )?;
    for settings in [
        Trace {
            colors: 0,
            ..Default::default()
        },
        Trace {
            error: f64::NAN,
            ..Default::default()
        },
        Trace {
            error: -1.,
            ..Default::default()
        },
    ] {
        check(
            core::trace(&limits, 0, settings).is_err(),
            "Invalid trace options accepted",
        )?;
    }
    let mut noise = blank(129, 129)?;
    for y in 0..129 {
        for x in 0..129 {
            if (x + y) % 2 == 0 {
                noise.layers[0].pixels[y * 129 + x] = [0, 0, 0, 255];
            }
        }
    }
    let before = noise.snapshot();
    check(
        core::trace(&noise, 0, Trace::default()).is_err() && noise.snapshot() == before,
        "Excess trace edges/nodes left partial geometry",
    )?;
    let mut nodes = blank(48, 48)?;
    for y in 0..48 {
        for x in 0..48 {
            if (x + y) % 2 == 0 {
                nodes.layers[0].pixels[y * 48 + x] = [10, 20, 30, 128];
            }
        }
    }
    let before = nodes.snapshot();
    check(
        core::trace(&nodes, 0, Trace::default()).is_err() && nodes.snapshot() == before,
        "Stored contour node limit left partial geometry",
    )?;
    // A large simple outline is reduced losslessly before the stored-node limit.
    let mut wide = blank(2048, 2)?;
    wide.layers[0].pixels.fill([20, 30, 40, 255]);
    check(
        core::trace(&wide, 0, Trace::default())?
            .state
            .scene
            .lines
            .layers[0]
            .paths[0]
            .points
            .len()
            == 5,
        "Collinear exact trace reduction",
    )?;
    Ok(())
}
fn short_lengths(root: &std::path::Path) -> Result<(), String> {
    let mut d = blank(64, 48)?;
    let layer = d.scene.lines.add_layer("Lengths")?;
    let a = d.scene.lines.add_path(
        layer,
        vec![[2., 2.], [5., 6.]],
        false,
        2.,
        [100, 20, 30, 150],
    )?;
    let b = d.scene.lines.add_path(
        layer,
        vec![[20., 20.], [21., 20.]],
        false,
        3.,
        [20, 200, 30, 200],
    )?;
    let scope = vec![Picked {
        layer,
        selection: Selection {
            paths: vec![a],
            controls: vec![],
        },
    }];
    let original = d.snapshot();
    check(
        core::remove_short(&d, &scope, 0.)?.state == original,
        "Zero length threshold changed source",
    )?;
    let p = core::remove_short(&d, &scope, 5.)?;
    check(
        p.changed == 0 && p.state == original,
        "Equal threshold length was removed",
    )?;
    let p = core::remove_short(&d, &scope, 5.0001)?;
    check(
        p.changed == 1
            && p.state.scene.lines.layers[0]
                .paths
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
                == [b]
            && d.snapshot() == original,
        "Short selected removal touched outside selection",
    )?;
    d.commit_state(p.state)?;
    saved(&mut d, root, "short-vector.drawer")?;
    check(d.undo() && d.snapshot() == original, "Short removal Undo")?;
    d.scene.transform = crate::scene::Affine([2., 0., 0., 2., 0., 0.]);
    check(
        core::remove_short(&d, &scope, 9.)?.changed == 0,
        "Short removal measured view/local coordinates",
    )?;
    d.scene.transform = Default::default();
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[2., 2.], [4., 2.], [24., 2.]];
    p.pressure.device = vec![Some(0.2), Some(0.7), Some(1.)];
    p.pressure.controls = vec![0.5, 1., 1.5];
    let old = p.clone();
    let spans = vec![Picked {
        layer,
        selection: Selection {
            paths: vec![],
            controls: vec![Control { path: a, point: 0 }, Control { path: a, point: 1 }],
        },
    }];
    let p = core::remove_short(&d, &spans, 3.)?;
    let after = &p.state.scene.lines.layers[0].paths[0];
    check(
        after.points == [[4., 2.], [24., 2.]]
            && p.state.scene.lines.layers[0].paths[1] == d.scene.lines.layers[0].paths[1],
        "Short span changed outside geometry",
    )?;
    for n in 0..101 {
        let t = n as f64 / 100.;
        check(
            (after.pressure.factor(t) - old.pressure.factor(0.5 + t * 0.5)).abs() < 1e-9,
            "Short span lost unselected pressure",
        )?;
    }
    let hidden = d.scene.lines.add_layer("Excluded")?;
    let id = d
        .scene
        .lines
        .add_path(hidden, vec![[0., 0.], [1., 0.]], false, 1., [0, 0, 0, 255])?;
    d.scene.lines.layers[1].hidden = true;
    let scope = core::all(&d);
    check(scope.len() == 1, "Hidden layer included in cleanup")?;
    let p = core::remove_short(&d, &scope, 100.)?;
    check(
        p.state.scene.lines.layers[1].paths[0].id == id
            && p.state.scene.lines.layers[0].paths.is_empty(),
        "All eligible layer cleanup",
    )?;
    d.scene.lines.layers[0].locked = true;
    check(
        core::all(&d).is_empty() && core::remove_short(&d, &spans, 3.).is_err(),
        "Locked layer cleanup accepted",
    )?;
    // Across the closed seam, adjacent selected spans are one connected interval.
    let mut closed = blank(64, 48)?;
    let l = closed.scene.lines.add_layer("Closed")?;
    let id = closed.scene.lines.add_path(
        l,
        vec![[0., 0.], [2., 0.], [2., 2.], [0., 2.]],
        true,
        1.,
        [0, 0, 0, 255],
    )?;
    let s = [Picked {
        layer: l,
        selection: Selection {
            paths: vec![],
            controls: [3, 0, 1]
                .into_iter()
                .map(|point| Control { path: id, point })
                .collect(),
        },
    }];
    check(
        core::remove_short(&closed, &s, 3.)?.changed == 0,
        "Closed seam measured as separate shorter spans",
    )?;
    let p = core::remove_short(&closed, &s, 4.01)?;
    check(
        p.changed == 1
            && p.state.scene.lines.layers[0].paths[0].points == [[2., 0.], [2., 2.], [0., 2.]],
        &format!(
            "Closed seam cut / outside arc: count={} points={:?}",
            p.changed, p.state.scene.lines.layers[0].paths[0].points
        ),
    )?;
    let mut dense = blank(64, 48)?;
    let l = dense.scene.lines.add_layer("Dense closed")?;
    dense.scene.lines.add_path(
        l,
        (0..4096)
            .map(|n| {
                let a = n as f64 / 4096. * std::f64::consts::TAU;
                [24. + a.cos() * 10., 24. + a.sin() * 10.]
            })
            .collect(),
        true,
        1.,
        [0, 0, 0, 255],
    )?;
    check(
        core::remove_short(&dense, &core::all(&dense), 1000.)?.changed == 1,
        "Maximum-node closed length allocated an oversized temporary fragment",
    )?;
    Ok(())
}
fn joins(root: &std::path::Path) -> Result<(), String> {
    let mut d = blank(64, 48)?;
    let layer = d.scene.lines.add_layer("Mixed styles")?;
    let a = d.scene.lines.add_path(
        layer,
        vec![[4., 10.], [20., 10.]],
        false,
        2.,
        [220, 20, 30, 128],
    )?;
    let b = d.scene.lines.add_path(
        layer,
        vec![[40., 12.], [24., 12.]],
        false,
        12.,
        [20, 50, 220, 210],
    )?;
    d.scene.lines.layers[0].paths[0].pressure.device = vec![Some(0.2), Some(0.5)];
    d.scene.lines.layers[0].paths[1].pressure.controls = vec![1.5, 0.75];
    let original = d.snapshot();
    let selection = Selection {
        paths: vec![a, b],
        controls: vec![],
    };
    let p = crate::vector_refine::prepare(
        &d,
        layer,
        &selection,
        crate::vector_refine::Operation::Join(5.),
    )?;
    let paths = &p.scene.lines.layers[0].paths;
    check(
        paths.len() == 3
            && paths[0] == original.scene.lines.layers[0].paths[0]
            && paths[2] == original.scene.lines.layers[0].paths[1]
            && d.snapshot() == original,
        "Mixed-style join changed source segments",
    )?;
    let bridge = &paths[1];
    check(
        bridge.points == [[20., 10.], [24., 12.]]
            && bridge.color == [220, 20, 30, 128]
            && p.scene.lines.selection.paths.len() == 3,
        "Mixed-style connector geometry/color/selection",
    )?;
    for n in 0..101 {
        let t = n as f64 / 100.;
        check(
            (bridge.width_at(t) - (1. * (1. - t) + 9. * t)).abs() < 1e-9,
            "Bridge width was not linear between source endpoint widths",
        )?;
    }
    d.commit_state(p)?;
    saved(&mut d, root, "mixed-join.drawer")?;
    check(
        d.undo() && d.snapshot() == original,
        "Mixed-style join one Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn raster_color_alpha_holes_limits_short_spans_and_style_preserving_joins() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
