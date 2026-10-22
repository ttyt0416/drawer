use crate::{
    color_precision::{Depth, Sample},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    local_color::{self as core, Correction},
    references::LocalImage,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for (i, p) in d.layers[0].pixels.iter_mut().enumerate() {
        *p = [
            if i & 1 == 0 { 64 } else { 192 },
            if i & 2 == 0 { 64 } else { 192 },
            if i & 4 == 0 { 64 } else { 192 },
            255,
        ];
    }
    Ok(d)
}
pub fn reference() -> Result<LocalImage, String> {
    let d = fixture()?;
    let pixels = d.layers[0]
        .pixels
        .iter()
        .cycle()
        .take(16)
        .map(|p| [p[0] / 2 + 32, p[1] / 2 + 64, p[2] / 2 + 96, 255])
        .collect();
    Ok(LocalImage {
        width: 4,
        height: 4,
        pixels,
    })
}
pub fn write_reference(root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let image = reference()?;
    let bytes: Vec<u8> = image.pixels.iter().flatten().copied().collect();
    let path = root.join("local-reference.png");
    image::save_buffer(
        &path,
        &bytes,
        image.width,
        image.height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let reference = reference()?;
    let value = core::distribution(&d, &reference, 1., |_, _| Ok(()))?;
    for p in &d.layers[0].pixels {
        let mapped = value.rgb(
            p[..3]
                .try_into()
                .map(|a: [u8; 3]| a.map(|v| v as f64 / 255.))
                .unwrap(),
        );
        let expected = [p[0] / 2 + 32, p[1] / 2 + 64, p[2] / 2 + 96];
        check(
            (0..3).all(|i| (mapped[i] * 255. - expected[i] as f64).abs() < 1e-6),
            "Covariance transfer matches independent diagonal mapping",
        )?;
    }
    for value in [
        value,
        Correction::Lighting {
            light: [1., 0.5, 0.25],
            shadow: [0.25, 0.5, 1.],
            strength: 0.8,
        },
    ] {
        let counts = d.history_counts();
        let state = core::prepare(&d, value)?;
        check(
            state.layers[..original.layers.len()] == original.layers,
            "Correction preserves original raster bytes",
        )?;
        let after = crate::layer_merge::preview(&state, d.current_frame)?;
        let renderer = crate::float_render::Renderer::new(&after)?;
        let expected = value.rgb([64. / 255.; 3]);
        let p = renderer.display_sample([0.5, 0.5])?;
        check(
            p.alpha() == 1. && (0..3).all(|i| (p.rgb()[i] - expected[i]).abs() < 1e-6),
            "Stored correction renders without alpha change",
        )?;
        d.commit_state(state)?;
        check(
            d.history_counts().0 == counts.0 + 1,
            "One correction is one Undo",
        )?;
        let path = root.join("local-color.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Local color native roundtrip",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&76u32.to_le_bytes());
        let old = root.join("local-color-old.drawer");
        std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Old native header rejects local color",
        )?;
        check(
            d.undo() && d.snapshot() == original,
            "Undo restores source and correction tree",
        )?;
    }
    let gray = LocalImage {
        width: 1,
        height: 2,
        pixels: vec![[40, 40, 40, 255], [210, 210, 210, 255]],
    };
    let value = core::distribution(&d, &gray, 1., |_, _| Ok(()))?;
    for p in &d.layers[0].pixels {
        let rgb = value.rgb(std::array::from_fn(|i| p[i] as f64 / 255.));
        check(
            (rgb[0] - rgb[1]).abs() < 1e-8 && (rgb[1] - rgb[2]).abs() < 1e-8,
            "Singular grayscale reference removes chroma without NaN",
        )?;
    }
    let mut transparent = gray.clone();
    transparent.pixels.push([255, 0, 255, 0]);
    transparent.height = 3;
    check(
        core::distribution(&d, &transparent, 1., |_, _| Ok(()))? == value,
        "Hidden transparent colors excluded",
    )?;
    transparent.pixels.fill([255, 0, 0, 0]);
    check(
        core::distribution(&d, &transparent, 1., |_, _| Ok(())).is_err(),
        "Fully transparent reference has actionable error",
    )?;
    let constant = LocalImage {
        width: 1,
        height: 1,
        pixels: vec![[100, 150, 200, 128]],
    };
    let flat = core::distribution(&d, &constant, 1., |_, _| Ok(()))?;
    check(
        flat.rgb([0.3, 0.4, 0.8])
            .iter()
            .zip([100., 150., 200.])
            .all(|(a, b)| (a * 255. - b).abs() < 1e-6),
        "Constant reference maps to its mean",
    )?;
    let half = Correction::Lighting {
        light: [1., 0.5, 0.25],
        shadow: [0.25, 0.5, 1.],
        strength: 0.5,
    };
    let rgb = [0.25, 0.5, 0.75];
    let l = 0.25 * 0.2126 + 0.5 * 0.7152 + 0.75 * 0.0722;
    let expected: [f64; 3] = [
        0.25 + 0.5 * (0.25 * 2. * (0.25 * (1. - l) + l) - 0.25),
        0.5,
        0.75 + 0.5 * ((0.75_f64 * 2. * (1. - l + 0.25 * l)).min(1.) - 0.75),
    ];
    check(
        half.rgb(rgb)
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() < 1e-10),
        "Lighting uses independent shadow/highlight equation",
    )?;
    let sample = Sample::new([rgb[0], rgb[1], rgb[2], 1. / 65535.])?;
    let corrected = crate::color_profile::WorkingSpace::new(&d.color.profile)?.adjusted(
        sample,
        crate::layer_effects::Adjustment {
            local: Some(half),
            ..Default::default()
        },
    )?;
    check(
        corrected.alpha() == sample.alpha() && corrected.rgba16()[3] == 1,
        "16bit low alpha is unchanged",
    )?;
    for mut value in [flat, half, Correction::lighting()] {
        *value.strength().unwrap() = 0.;
        let counts = d.history_counts();
        let dirty = d.dirty;
        d.commit_state(core::prepare(&d, value)?)?;
        check(
            d.snapshot() == original && d.history_counts() == counts && d.dirty == dirty,
            "Strength zero preserves dirty/Redo/history",
        )?;
    }
    check(
        core::distribution(&d, &reference, 1., |_, _| Err("cancel".into())).is_err()
            && d.snapshot() == original,
        "Cancelled statistics never modify the document",
    )?;
    let mut constant_source = fixture()?;
    constant_source.layers[0].pixels.fill([90, 100, 110, 255]);
    let weighted = LocalImage {
        width: 1,
        height: 2,
        pixels: vec![[255, 0, 0, 255], [0, 0, 255, 85]],
    };
    let transferred = core::distribution(&constant_source, &weighted, 1., |_, _| Ok(()))?.rgb([
        90. / 255.,
        100. / 255.,
        110. / 255.,
    ]);
    check(
        transferred
            .iter()
            .zip([0.75, 0., 0.25])
            .all(|(a, b)| (a - b).abs() < 1e-8),
        "Alpha-weighted reference and constant source use finite mean",
    )?;
    let mut empty = fixture()?;
    empty.layers[0].pixels.fill([0; 4]);
    check(
        core::distribution(&empty, &reference, 1., |_, _| Ok(())).is_err(),
        "Transparent source backdrop is rejected",
    )?;
    check(
        core::distribution(&d, &reference, f64::NAN, |_, _| Ok(())).is_err()
            && core::prepare(
                &d,
                Correction::Lighting {
                    light: [2.; 3],
                    shadow: [0.5; 3],
                    strength: 1.,
                },
            )
            .is_err(),
        "Invalid parameters rejected",
    )?;
    // Same correction takes the floating path for legacy documents and native 16-bit documents.
    let state = core::prepare(&d, half)?;
    let legacy = crate::layer_merge::preview(&state, d.current_frame)?;
    let p = crate::render::Renderer::new(&legacy).pixel([0.5, 0.5]);
    let f = crate::float_render::Renderer::new(&legacy)?
        .display_sample([0.5, 0.5])?
        .rgba8();
    check(p == f, "Legacy and float correction composition agree")?;
    let mut high = Document::create_with_color(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        crate::color_storage::Config {
            depth: Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    high.layers[0].precise.as_mut().unwrap()[0] = sample.rgba16();
    high.layers[0].pixels[0] = sample.rgba8();
    let old = high.layers[0].clone();
    let state = core::prepare(&high, half)?;
    check(state.layers[0] == old, "16bit native source preserved")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn offline_covariance_lighting_alpha_native_and_undo() {
        let dir = crate::test_support::TempDir::new();
        super::verify(&dir.0).unwrap();
    }
}
