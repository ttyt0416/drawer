use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    vector_edit::Selection,
    vector_tip::{self as core, Overlay, Points, Preset, Scope},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn pixels(d: &crate::document::Document) -> Result<Vec<[u8; 4]>, String> {
    let renderer = crate::render::Renderer::for_export(d)?;
    if let Some(error) = renderer.error() {
        return Err(error.into());
    }
    let mut out = vec![];
    for y in 0..d.height {
        for x in 0..d.width {
            out.push(renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]));
        }
    }
    Ok(out)
}
pub fn fixture() -> Result<crate::document::Document, String> {
    let mut d = crate::vector_nodes_verify::fixture()?;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[8., 16.], [24., 16.]];
    p.definition = None;
    p.width = 12.;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let id = d.scene.lines.layers[0].paths[0].id;
    let s = Selection {
        paths: vec![id],
        ..Default::default()
    };
    let before = d.snapshot();
    check(
        core::prepare(&d, layer, &s, "Unavailable local tip").is_err() && d.snapshot() == before,
        "Missing tip partially applied",
    )?;
    let mut renders = vec![];
    for tip in Preset::ALL {
        let state = core::prepare(&d, layer, &s, tip.label())?;
        let p = &state.scene.lines.layers[0].paths[0];
        let original = &before.scene.lines.layers[0].paths[0];
        check(
            p.tip == tip
                && p.points == original.points
                && p.definition == original.definition
                && p.pressure == original.pressure
                && p.width == original.width
                && p.color == original.color
                && state.scene.lines.layers[0].paths[1] == before.scene.lines.layers[0].paths[1],
            "Tip edit changed geometry, style, pressure or unselected path",
        )?;
        let count = d.history_counts();
        d.commit_state(state)?;
        check(
            d.history_counts().0 == count.0 + 1,
            "Tip edit did not create one Undo",
        )?;
        renders.push(pixels(&d)?);
        let file = root.join(format!("vector-tip-{tip:?}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Vector tip native roundtrip",
        )?;
        if tip != Preset::Round {
            let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
            bytes[8..12].copy_from_slice(&66u32.to_le_bytes());
            std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
            check(
                crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_err(),
                "New tip accepted under old header",
            )?;
        }
        check(
            d.undo() && d.snapshot() == before,
            "Tip Undo did not restore original",
        )?;
    }
    for i in 0..renders.len() {
        for j in i + 1..renders.len() {
            check(
                renders[i] != renders[j],
                &format!(
                    "Tip presets produced identical pixels: {:?}, {:?}; visible={}",
                    Preset::ALL[i],
                    Preset::ALL[j],
                    renders[i].iter().filter(|p| p[3] > 0).count()
                ),
            )?;
        }
    }
    let mut p = d.scene.lines.layers[0].paths[0].clone();
    p.width = 6.;
    p.pressure = Default::default();
    for tip in Preset::ALL {
        p.tip = tip;
        for y in -14..=14 {
            for x in -14..=14 {
                let point = [8. + x as f64 / 4., 16. + y as f64 / 4.];
                let dx = (point[0] - 8.).min(0.);
                let dy = point[1] - 16.;
                let expected = match tip {
                    Preset::Round => dx.hypot(dy) <= 3.,
                    Preset::Flat => dx.hypot(dy * 3.) <= 3.,
                    Preset::Square => dx.abs().max(dy.abs()) <= 3.,
                    Preset::Diamond => dx.abs() + dy.abs() <= 3.,
                };
                check(
                    p.stroke_hit(point) == expected,
                    "Tip swept geometry differs from independent endpoint formula",
                )?;
            }
        }
    }
    overlay(&mut d, layer)?;
    // Erasers use the actual new tip footprint, including affine cross-sections.
    for tip in Preset::ALL {
        let mut e = fixture()?;
        let p = &mut e.scene.lines.layers[0].paths[0];
        p.width = 6.;
        p.tip = tip;
        p.pressure = Default::default();
        let point = if tip == Preset::Flat {
            [16., 18.]
        } else {
            [6., 14.]
        };
        let state = crate::vector_erase::prepare(
            &e,
            Some(crate::line_tools::Target::Vector(layer)),
            point,
            crate::vector_erase::Settings {
                mode: crate::vector_erase::Mode::Whole,
                radius: 0.1,
                ..Default::default()
            },
        )?;
        let erased = state.scene.lines.layers[0].paths.len() == 1;
        check(
            erased == matches!(tip, Preset::Round | Preset::Square),
            "Eraser ignored tip footprint",
        )?;
        for matrix in [
            crate::scene::Affine([2., 0., 0., 0.5, 0., 0.]),
            crate::scene::Affine([1., 0.3, 0.5, 1., 0., 0.]),
        ] {
            e.scene.transform = matrix;
            let state = crate::vector_erase::prepare(
                &e,
                Some(crate::line_tools::Target::Vector(layer)),
                matrix.map(point),
                crate::vector_erase::Settings {
                    mode: crate::vector_erase::Mode::Whole,
                    radius: 0.1,
                    ..Default::default()
                },
            )?;
            check(
                (state.scene.lines.layers[0].paths.len() == 1) == erased,
                "Affine eraser changed tip contact classification",
            )?;
        }
    }
    let mut pick = fixture()?;
    let path = &mut pick.scene.lines.layers[0].paths[0];
    path.tip = Preset::Square;
    path.width = 6.;
    path.pressure = Default::default();
    let (sin, cos) = std::f64::consts::FRAC_PI_4.sin_cos();
    pick.scene.transform = crate::scene::Affine([cos, sin, -sin, cos, 16., 0.]);
    check(
        crate::vector_edit::hits(
            &pick,
            layer,
            pick.scene.transform.map([5.1, 13.1]),
            0.1,
            crate::vector_edit::Tool::Select,
        )
        .iter()
        .any(|h| h.path == id),
        "Rotated square corner could not be selected",
    )?;
    Ok(())
}
fn overlay(d: &mut crate::document::Document, layer: u64) -> Result<(), String> {
    let other = d.scene.lines.add_layer("Overlay locked")?;
    d.scene.lines.add_path(
        other,
        vec![[0., 0.], [16., 16.], [0., 16.], [16., 0.]],
        false,
        2.,
        [0, 0, 0, 255],
    )?;
    crate::shapes::prepare(
        d,
        crate::shapes::Target::Vector(Some(other)),
        &[[1., 1.], [5., 10.], [10., 5.], [16., 16.]],
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Bezier,
            curve: crate::shapes::Curve::CubicBezier,
            ..Default::default()
        },
    )?
    .restore(d);
    d.scene.lines.layers[1].locked = true;
    let hidden = d.scene.lines.add_layer("Overlay hidden")?;
    d.scene
        .lines
        .add_path(hidden, vec![[0., 2.], [16., 2.]], false, 2., [0, 0, 0, 255])?;
    d.scene.lines.layers[2].hidden = true;
    let before = d.snapshot();
    let history = d.history_counts();
    let rendered = pixels(d)?;
    for scope in [Scope::Selected, Scope::All] {
        for points in [Points::Ends, Points::All, Points::None] {
            let settings = Overlay {
                enabled: true,
                scope,
                points,
            };
            let paths = core::outlines(d, Some(layer), settings);
            check(
                paths.len() == if scope == Scope::All { 4 } else { 2 },
                "Overlay layer scope/hidden/locked handling",
            )?;
            check(
                paths.iter().all(|p| p.layer != hidden)
                    && paths.iter().any(|p| p.layer == other) == (scope == Scope::All),
                "Overlay hid a locked layer or included a hidden layer",
            )?;
            for path in paths {
                let p = d
                    .scene
                    .lines
                    .layers
                    .iter()
                    .find(|l| l.id == path.layer)
                    .unwrap()
                    .paths
                    .iter()
                    .find(|p| p.id == path.path)
                    .unwrap();
                check(
                    path.points
                        == p.points
                            .iter()
                            .map(|p| d.scene.transform.map(*p))
                            .collect::<Vec<_>>(),
                    "Overlay path transform",
                )?;
                check(
                    path.controls.len()
                        == match points {
                            Points::None => 0,
                            Points::Ends => {
                                if p.closed {
                                    0
                                } else {
                                    2
                                }
                            }
                            Points::All => p.control_count(),
                        },
                    "Overlay point scope",
                )?;
            }
            check(
                d.snapshot() == before && d.history_counts() == history && pixels(d)? == rendered,
                "View overlay changed document, selection, Undo or export pixels",
            )?;
        }
    }
    check(
        core::outlines(d, Some(layer), Overlay::default()).is_empty(),
        "Hidden overlay emitted paths",
    )?;
    let original = d.scene.lines.layers[0].paths[0].clone();
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points.push([16., 24.]);
    p.closed = true;
    let closed = core::outlines(
        d,
        Some(layer),
        Overlay {
            enabled: true,
            scope: Scope::Selected,
            points: Points::Ends,
        },
    );
    check(
        closed[0].closed && closed[0].controls.is_empty(),
        "Closed path displayed false open endpoints",
    )?;
    d.scene.lines.layers[0].paths[0] = original;
    check(
        d.snapshot() == before,
        "Overlay closed fixture was not restored",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_tip_presets_native_geometry_eraser_overlay_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
