use crate::{
    deform as core,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    material_mapping::{UNIT, Warp},
    scene::Rect,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([0, 255, 0, 0]);
    for y in 4..12 {
        for x in 4..12 {
            if x == 8 || y == 8 {
                d.layers[0].pixels[y * 16 + x] = [255, 0, 0, 128];
            }
        }
    }
    Ok(d)
}
fn dab(mode: core::Mode) -> core::Dab {
    core::Dab {
        mode,
        center: [8.5, 8.5],
        delta: [2., 0.],
        radius: 6.,
        strength: 1.,
    }
}
pub fn directions() -> Result<(), String> {
    // Independent expectations in screen coordinates, including the inverse sampling sign.
    let p = [10.5, 8.5];
    let expected = 2. * (1. - 4f64 / 36.).powi(2);
    check(
        (dab(core::Mode::Push).inverse(p)[0] - (p[0] - expected * 0.75)).abs() < 1e-12,
        "Push did not move ink along the drag",
    )?;
    check(
        dab(core::Mode::Left).inverse(p)[1] > p[1] && dab(core::Mode::Right).inverse(p)[1] < p[1],
        "Side push is not left/right relative to travel",
    )?;
    check(
        dab(core::Mode::Expand).inverse(p)[0] < p[0] && dab(core::Mode::Pinch).inverse(p)[0] > p[0],
        "Expand/pinch sampling reversed",
    )?;
    check(
        dab(core::Mode::Clockwise).inverse(p)[1] < p[1]
            && dab(core::Mode::Anticlockwise).inverse(p)[1] > p[1],
        "Screen clockwise/anticlockwise sampling reversed",
    )?;
    for m in core::Mode::ALL {
        let mut b = dab(m);
        check(
            b.inverse([15., 8.5]) == [15., 8.5],
            "Brush changed pixels outside its support",
        )?;
        b.strength = 0.;
        check(b.inverse(p) == p, "Zero strength changes coordinates")?;
    }
    Ok(())
}
pub fn quads() -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let identity = Warp::Distort(UNIT);
    check(
        core::prepare(&d, &[0], false, core::Change::Quad(&identity))? == original,
        "Identity changed storage",
    )?;
    let q = UNIT.map(|p| [p[0] + 0.125, p[1]]);
    for w in [Warp::Distort(q), Warp::Perspective(q)] {
        let s = core::prepare(&d, &[0], false, core::Change::Quad(&w))?;
        check(
            s.layers[0].pixel(16, 16, 10, 8) == [255, 0, 0, 128]
                && s.layers[0].pixel(16, 16, 8, 4) == [0; 4],
            "Translated quad differs from exact2-pixel reference",
        )?;
        check(d.snapshot() == original, "Quad preview changed source")?;
    }
    for q in [
        [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
        [[0., 0.]; 4],
        [[0., 0.], [1., 1.], [1., 0.], [0., 1.]],
    ] {
        for w in [Warp::Distort(q), Warp::Perspective(q)] {
            check(
                core::prepare(&d, &[0], false, core::Change::Quad(&w)).is_err(),
                "Reversed/folded/degenerate quad accepted",
            )?;
        }
    }
    let perspective = Warp::Perspective([[0.2, 0.], [0.8, 0.], [1., 1.], [0., 1.]]);
    let area = core::area(&d, &[0], false)?.unwrap();
    let p = core::inverse_quad(&perspective, area, [8., 8.]).ok_or("Perspective inverse failed")?;
    check(
        (p[0] - 8.).abs() < 1e-8 && (p[1] - 10.).abs() < 1e-8,
        "Perspective was treated as bilinear free deformation",
    )?;
    d.scene.selection = Some(Rect {
        min: [7., 4.],
        max: [10., 12.],
    });
    let before = d.layers[0].pixel(16, 16, 4, 8);
    let w = Warp::Distort(UNIT.map(|p| [p[0] + 1., p[1]]));
    let s = core::prepare(&d, &[0], true, core::Change::Quad(&w))?;
    check(
        s.layers[0].pixel(16, 16, 4, 8) == before
            && s.layers[0].pixel(16, 16, 11, 4) == [255, 0, 0, 128],
        "Selection cut/move changed unselected pixels or lost selected ink",
    )?;
    check(
        s.scene.selection_coverage([11.5, 4.5]) > 0 && s.scene.selection_coverage([8.5, 4.5]) == 0,
        "Area selection did not follow quad",
    )?;
    Ok(())
}
pub fn liquify() -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    for mode in core::Mode::ALL {
        let b = dab(mode);
        let s = core::prepare(
            &d,
            &[0],
            false,
            core::Change::Liquify {
                dabs: &[b],
                only_refer: false,
            },
        )?;
        check(
            s.layers[0].pixels != original.layers[0].pixels,
            "Liquify mode did not alter markers",
        )?;
        check(
            s.layers[0].pixel(16, 16, 0, 0) == d.layers[0].pixels[0],
            "Liquify changed hidden color outside brush",
        )?;
        for i in 0..s.layers[0].pixels.len() {
            let p = s.layers[0].sample_at(i);
            if p.alpha() > 0. {
                check(
                    p.rgb()[0] > 0.99 && p.rgb()[1] < 0.01 && p.rgb()[2] < 0.01,
                    "Transparent RGB caused a colored/black fringe",
                )?;
            }
        }
        let mut zero = b;
        zero.strength = 0.;
        check(
            core::prepare(
                &d,
                &[0],
                false,
                core::Change::Liquify {
                    dabs: &[zero],
                    only_refer: true,
                },
            )? == original,
            "Zero strength changed native storage",
        )?;
    }
    d.scene.selection = Some(Rect {
        min: [4., 4.],
        max: [12., 12.],
    });
    d.layers[0].pixels.fill([0; 4]);
    for y in 0..16 {
        d.layers[0].pixels[y * 16 + 3] = [255, 0, 0, 255];
    }
    let mut b = dab(core::Mode::Push);
    b.center = [5.5, 8.5];
    b.delta = [1.5, 0.];
    let loose = core::prepare(
        &d,
        &[0],
        true,
        core::Change::Liquify {
            dabs: &[b],
            only_refer: false,
        },
    )?;
    let strict = core::prepare(
        &d,
        &[0],
        true,
        core::Change::Liquify {
            dabs: &[b],
            only_refer: true,
        },
    )?;
    check(
        loose.layers[0].pixel(16, 16, 4, 8)[3] > 0 && strict.layers[0].pixel(16, 16, 4, 8)[3] == 0,
        "Editing-area-only reference did not exclude outside ink",
    )?;
    check(
        strict.layers[0].pixel(16, 16, 3, 8) == [255, 0, 0, 255],
        "Liquify changed pixels outside selection",
    )?;
    let empty = crate::selection::Mask { runs: vec![] };
    d.scene.selection_mask = Some(empty);
    check(
        core::prepare(
            &d,
            &[0],
            true,
            core::Change::Liquify {
                dabs: &[b],
                only_refer: false,
            },
        )? == d.snapshot(),
        "Empty selection was treated as unrestricted",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    directions()?;
    quads()?;
    liquify()?;
    let mut faint = fixture()?;
    faint.color = Default::default();
    faint.layers[0].pixels.fill([0; 4]);
    let mut high = vec![[0; 4]; 256];
    high[136] = [23456, 12345, 45678, 10];
    faint.layers[0].pixels[136] = crate::color_precision::Sample::from16(high[136]).rgba8();
    faint.layers[0].precise = Some(high);
    let expanded = core::prepare(
        &faint,
        &[0],
        false,
        core::Change::Liquify {
            dabs: &[dab(core::Mode::Expand)],
            only_refer: false,
        },
    )?;
    let at = expanded.layers[0].pixel_index(16, 16, 9, 8).unwrap();
    let value = expanded.layers[0].precise.as_ref().unwrap()[at];
    check(
        value[3] > 0
            && value[3] < 10
            && expanded.layers[0].pixels[at][3] == 0
            && value[..3]
                .iter()
                .zip([23456, 12345, 45678])
                .all(|(a, b)| a.abs_diff(b) < 2),
        "Liquify lost native16 ink whose8-bit alpha preview is zero",
    )?;
    let mut d = fixture()?;
    d.color = Default::default();
    let high = d.layers[0]
        .pixels
        .iter()
        .map(|p| {
            if p[3] == 0 {
                [0; 4]
            } else {
                crate::color_precision::Sample::from8(*p).rgba16()
            }
        })
        .collect::<Vec<_>>();
    d.layers[0].pixels = high
        .iter()
        .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
        .collect();
    d.layers[0].precise = Some(high);
    d.layers[0].precise.as_mut().unwrap()[8 * 16 + 8] = [23456, 12345, 45678, 10];
    d.layers[0].pixels[8 * 16 + 8] =
        crate::color_precision::Sample::from16([23456, 12345, 45678, 10]).rgba8();
    d.layers.push(d.layers[0].clone());
    d.layers[1].name = "Second raster".into();
    let original = d.snapshot();
    let counts = d.history_counts();
    let b = dab(core::Mode::Expand);
    let state = core::prepare(
        &d,
        &[0, 1],
        false,
        core::Change::Liquify {
            dabs: &[b, dab(core::Mode::Clockwise)],
            only_refer: false,
        },
    )?;
    check(
        state.layers[0].same_color(&state.layers[1]) && state.layers[0].precise.is_some(),
        "Multi-layer/native16 processing diverged",
    )?;
    let one = core::prepare(
        &d,
        &[0, 1],
        false,
        core::Change::Liquify {
            dabs: &[b],
            only_refer: false,
        },
    )?;
    let intermediate = crate::layer_merge::preview(&one, d.current_frame)?;
    let two = core::prepare(
        &intermediate,
        &[0, 1],
        false,
        core::Change::Liquify {
            dabs: &[dab(core::Mode::Clockwise)],
            only_refer: false,
        },
    )?;
    check(
        state.layers == two.layers && d.snapshot() == original && d.history_counts() == counts,
        "Dabs were not sequential or changed original",
    )?;
    d.commit_state(state)?;
    let saved = d.snapshot();
    let file = root.join("raster-deformation.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved
            && d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.snapshot() == saved,
        "Native/atomic Undo/Redo failed",
    )?;
    d.layers[1].locked = true;
    check(
        core::prepare(
            &d,
            &[0, 1],
            false,
            core::Change::Liquify {
                dabs: &[b],
                only_refer: false,
            },
        )
        .is_err(),
        "Locked multi-layer member accepted",
    )?;
    d.layers[1].locked = false;
    d.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&d, 0));
    check(
        core::prepare(
            &d,
            &[0],
            false,
            core::Change::Liquify {
                dabs: &[b],
                only_refer: false,
            },
        )
        .is_err(),
        "Protected alpha moved",
    )?;
    d.scene.clipping.alpha.clear();
    let mut bad = b;
    bad.strength = f64::NAN;
    check(
        core::prepare(
            &d,
            &[0],
            false,
            core::Change::Liquify {
                dabs: &[bad],
                only_refer: false,
            },
        )
        .is_err(),
        "Nonfinite brush accepted",
    )?;
    let mut huge = b;
    huge.radius = 1024.;
    check(
        core::prepare(
            &d,
            &[0],
            false,
            core::Change::Liquify {
                dabs: &[huge],
                only_refer: false,
            },
        )
        .is_err(),
        "Deformation exceeded output/memory budget",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn analytic_liquify_directions() {
        super::directions().unwrap();
    }
    #[test]
    fn exact_quad_selection_and_reject_folds() {
        super::quads().unwrap();
    }
    #[test]
    fn liquify_markers_alpha_selection_reference() {
        super::liquify().unwrap();
    }
    #[test]
    fn multi_native_sequential_history_limits() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
