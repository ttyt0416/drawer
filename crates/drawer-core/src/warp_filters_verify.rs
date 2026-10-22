use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    pixel_effects::{self as core, Operation as P},
    sizing::DocumentState,
    warp_filters::Operation as O,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 7,
            height: 7,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for y in 0..7 {
        for x in 0..7 {
            d.layers[0].pixels[y * 7 + x] = [
                (x * 36) as u8,
                (y * 35) as u8,
                ((x + y) * 17) as u8,
                if x % 3 == 0 { 120 } else { 255 },
            ];
        }
    }
    Ok(d)
}
pub fn zero(op: O) -> O {
    match op {
        O::Fisheye { radius, center, .. } => O::Fisheye {
            strength: 0.,
            radius,
            center,
        },
        O::Panorama { center, .. } => O::Panorama {
            field_of_view: 0.,
            center,
        },
        O::Wave { period, phase, .. } => O::Wave {
            amplitude: [0.; 2],
            period,
            phase,
        },
        O::Swirl { radius, center, .. } => O::Swirl {
            angle: 0.,
            radius,
            center,
        },
    }
}
fn run(d: &Document, op: O, selection: bool) -> Result<DocumentState, String> {
    core::prepare(
        d,
        0,
        P::Warp(op),
        selection,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let extent = d.layers[0].extent(7, 7);
    for op in O::defaults() {
        check(
            run(&d, zero(op), false)? == original,
            "Distortion zero preserves dirty/redo/last recipe",
        )?;
        let state = run(&d, op, false)?;
        check(
            state != original,
            "Each distortion must affect asymmetric source",
        )?;
        if !matches!(op, O::Wave { .. }) {
            check(
                op.map([3.5, 3.5], extent) == [3.5, 3.5],
                "Distortion center fixed",
            )?;
        }
        d.commit_state(state)?;
        let p = root.join("distortion.drawer");
        crate::files::save(&mut d, &p).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&p, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Distortion native parameter/result reopen",
        )?;
        let mut bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&74u32.to_le_bytes());
        let old = root.join("distortion-old.drawer");
        std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Older header must reject distortion recipe",
        )?;
        check(
            d.undo() && d.snapshot() == original,
            "Distortion one Undo / source restoration",
        )?;
    }
    let wave = O::Wave {
        amplitude: [1024., 0.],
        period: [2.; 2],
        phase: 0.,
    };
    let state = run(&d, wave, false)?;
    check(
        state.layers[0].pixels.iter().all(|p| p[3] == 0),
        "Outside source samples transparent, no edge repetition",
    )?;
    let a = run(
        &d,
        O::Wave {
            amplitude: [1., 2.],
            period: [3., 4.],
            phase: 0.,
        },
        false,
    )?;
    let b = run(
        &d,
        O::Wave {
            amplitude: [1., 2.],
            period: [3., 4.],
            phase: 360.,
        },
        false,
    )?;
    check(a == b || a.layers == b.layers, "Wave phase period")?;
    d.scene.selection = Some(crate::scene::Rect {
        min: [2.; 2],
        max: [5.; 2],
    });
    let before = d.snapshot();
    let selected = run(&d, O::defaults()[0], true)?;
    for y in 0..7 {
        for x in 0..7 {
            if !(2..5).contains(&x) || !(2..5).contains(&y) {
                check(
                    selected.layers[0].pixels[y * 7 + x] == d.layers[0].pixels[y * 7 + x],
                    "Selection is output scope / outside original preserved",
                )?;
            }
        }
    }
    let invalid = O::Wave {
        amplitude: [1.; 2],
        period: [0., 1.],
        phase: 0.,
    };
    check(
        run(&d, invalid, false).is_err() && d.snapshot() == before,
        "Invalid period fails atomically",
    )?;
    check(
        run(
            &d,
            O::Panorama {
                field_of_view: 180.,
                center: None,
            },
            false,
        )
        .is_err(),
        "Singular panorama rejected",
    )?;
    let mut ticks = 0;
    check(
        core::prepare(
            &d,
            0,
            P::Warp(O::defaults()[3]),
            false,
            DEFAULT_DOCUMENT_BUDGET,
            |_, _| {
                ticks += 1;
                if ticks > 1 {
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && d.snapshot() == before,
        "Distortion cancellation atomic",
    )?;
    let off = O::Fisheye {
        strength: 1.,
        radius: 2.,
        center: Some([100., 100.]),
    };
    check(
        run(&d, off, false)? == before,
        "Outside radius remains unchanged",
    )?;
    let mut red = fixture()?;
    red.layers[0].pixels.fill([0, 255, 255, 0]);
    red.layers[0].pixels[0] = [255, 0, 0, 255];
    let half = crate::warp_filters::sample(
        &red,
        0,
        [0, 0],
        O::Wave {
            amplitude: [0.5, 0.],
            period: [2.; 2],
            phase: 0.,
        },
    )?
    .rgba8();
    check(
        half == [255, 0, 0, 128],
        "Bilinear alpha weighting must not leak hidden transparent cyan",
    )?;
    let mut high = fixture()?;
    high.color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        ..Default::default()
    };
    let mut samples = high.layers[0]
        .pixels
        .iter()
        .map(|p| crate::color_precision::Sample::from8(*p))
        .collect::<Vec<_>>();
    samples[10] = crate::color_precision::Sample::from16([12345, 45678, 23456, 1]);
    crate::color_storage::set_samples(&mut high.layers[0], &high.color, extent, samples)?;
    let state = run(&high, O::defaults()[3], false)?;
    check(
        state.color == high.color && state.layers[0].precise.is_some(),
        "Distortion retains native16/profile",
    )?;
    high.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&high, 0));
    let locked = run(&high, O::defaults()[0], false)?;
    check(
        locked.layers[0]
            .precise
            .as_ref()
            .unwrap()
            .iter()
            .zip(high.layers[0].precise.as_ref().unwrap())
            .all(|(a, b)| a[3] == b[3]),
        "Distortion alpha lock preserves exact16-bit alpha",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn geometry_alpha_scope_native_precision_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
