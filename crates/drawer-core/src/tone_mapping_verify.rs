use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    gradient_ramp::Ramp,
    tone_mapping::{self as core, Curve, Operation},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 256,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for n in 0..256 {
        d.layers[0].pixels[n] = [
            n as u8,
            (255 - n) as u8,
            (n * 7 % 256) as u8,
            if n == 0 { 0 } else { (n * 11 % 255 + 1) as u8 },
        ];
    }
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    check(
        core::prepare(&d, 0, false, &Operation::identity_curves())? == original,
        "Identity tone curve changed source/precision",
    )?;
    let mut c = std::array::from_fn::<_, 4, _>(|_| Curve::default());
    c[0].points = vec![[0., 1.], [1., 0.]];
    let inverse = Operation::Curves(c.clone());
    let p = core::prepare(&d, 0, false, &inverse)?;
    for (a, b) in d.layers[0].pixels.iter().zip(&p.layers[0].pixels) {
        check(
            a[3] == b[3]
                && (if a[3] == 0 {
                    a == b
                } else {
                    (0..3).all(|k| b[k] == 255 - a[k])
                }),
            "Inverse curve pixel/alpha oracle",
        )?;
    }
    check(
        p.scene == original.scene && d.snapshot() == original && d.history_counts() == counts,
        "Tone curve changed geometry/source/history",
    )?;
    d.commit_state(p)?;
    let path = root.join("tone-curves.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Tone mapping native/one Undo/Redo",
    )?;
    d.undo();
    c[1].points = vec![[0., 0.], [1., 0.5]];
    let p = core::prepare(&d, 0, false, &Operation::Curves(c.clone()))?;
    for (a, b) in d.layers[0].pixels.iter().zip(&p.layers[0].pixels) {
        if a[3] > 0 {
            check(
                // Either nearest byte is valid at an exact half-step after floating interpolation.
                (b[0] as f64 - (255 - a[0]) as f64 * 0.5).abs() <= 0.500000001
                    && b[1] == 255 - a[1]
                    && b[2] == 255 - a[2]
                    && b[3] == a[3],
                &format!("Master then channel order: {a:?} -> {b:?}"),
            )?;
        }
    }
    let curved = Curve {
        points: vec![[0., 0.], [0.2, 0.8], [0.6, 0.25], [1., 1.]],
        smooth: true,
    };
    for n in 0..=1024 {
        let t = n as f64 / 1024.;
        let i = curved
            .points
            .partition_point(|p| p[0] <= t)
            .saturating_sub(1)
            .min(2);
        let y = curved.sample(t).unwrap();
        check(
            y >= curved.points[i][1].min(curved.points[i + 1][1]) - 1e-12
                && y <= curved.points[i][1].max(curved.points[i + 1][1]) + 1e-12,
            "Smooth curve overshot neighboring outputs",
        )?;
    }
    for points in [
        vec![[0., 0.], [0.5, 1.], [0.5, 0.2], [1., 1.]],
        vec![[0., 0.], [f64::NAN, 0.5], [1., 1.]],
        vec![[0., 0.], [1e-320, 1.], [1., 1.]],
    ] {
        c[2].points = points;
        check(
            core::prepare(&d, 0, false, &Operation::Curves(c.clone())).is_err()
                && d.snapshot() == original,
            "Overlapping/nonfinite/nearly coincident controls accepted",
        )?;
    }
    let ramp = Ramp::two([0, 0, 0, 0], [255, 0, 0, 0]);
    let mapped = core::prepare(&d, 0, false, &Operation::Gradient(ramp.clone()))?;
    for (a, b) in d.layers[0].pixels.iter().zip(&mapped.layers[0].pixels) {
        if a[3] > 0 {
            let expected =
                (0.2126 * a[0] as f64 + 0.7152 * a[1] as f64 + 0.0722 * a[2] as f64).round() as u8;
            check(
                b[0].abs_diff(expected) <= 1 && b[1] == 0 && b[2] == 0 && b[3] == a[3],
                "Gradient map luminance/alpha oracle",
            )?;
        } else {
            check(a == b, "Transparent hidden RGB changed")?;
        }
    }
    let mut opaque = ramp;
    for s in &mut opaque.stops {
        s.color[3] = 255;
    }
    check(
        core::prepare(&d, 0, false, &Operation::Gradient(opaque))? == mapped,
        "Gradient stop alpha changed mapping or original alpha",
    )?;
    let mut selected = fixture()?;
    selected.scene.selection = Some(crate::scene::Rect {
        min: [50., 0.],
        max: [100., 1.],
    });
    let p = core::prepare(&selected, 0, true, &inverse)?;
    for n in 0..256 {
        if !(50..100).contains(&n) {
            check(
                p.layers[0].pixels[n] == selected.layers[0].pixels[n],
                "Tone mapping changed pixels outside selection",
            )?;
        }
    }
    selected.scene.selection_mask = Some(Default::default());
    check(
        core::prepare(&selected, 0, true, &inverse)? == selected.snapshot(),
        "Empty selection created a tone mapping edit",
    )?;
    let mut precise = fixture()?;
    precise.layers[0].pixels[0] = [0; 4];
    precise.color = Default::default();
    let mut high = precise.layers[0]
        .pixels
        .iter()
        .map(|p| crate::color_precision::Sample::from8(*p).rgba16())
        .collect::<Vec<_>>();
    high[50] = [12345, 30000, 50000, 23456];
    high[51] = [65535, 0, 0, 1];
    precise.layers[0].pixels[50] = crate::color_precision::Sample::from16(high[50]).rgba8();
    precise.layers[0].pixels[51] = crate::color_precision::Sample::from16(high[51]).rgba8();
    precise.layers[0].precise = Some(high.clone());
    let p = core::prepare(&precise, 0, false, &inverse)?;
    let after = p.layers[0].precise.as_ref().unwrap();
    check(
        after.iter().zip(&high).all(|(a, b)| a[3] == b[3])
            && after[50][..3] == [65535 - 12345, 65535 - 30000, 65535 - 50000]
            && after[51] == [0, 65535, 65535, 1],
        "Native16 curve precision/alpha oracle",
    )?;
    d.layers[0].locked = true;
    check(
        core::prepare(&d, 0, false, &inverse).is_err(),
        "Locked tone mapping target accepted",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn curves_gradient_mapping_identity_inverse_overlap_channels_precision_alpha_selection_native_and_undo()
     {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
