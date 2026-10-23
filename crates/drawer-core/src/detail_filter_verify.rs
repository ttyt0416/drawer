use crate::{
    color_precision::Sample,
    detail_filter::{self as core, Operation as O, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
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
            background: Background::Color([80; 3]),
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for (n, v) in [
        (16, 50),
        (17, 100),
        (18, 180),
        (23, 100),
        (24, 120),
        (25, 160),
        (30, 50),
        (31, 160),
        (32, 230),
    ] {
        d.layers[0].pixels[n] = [v, v, v, 255];
    }
    Ok(d)
}
fn settings(operation: O) -> Settings {
    Settings {
        options: Default::default(),
        operation,
        selection: false,
    }
}
fn run(d: &Document, op: O) -> Result<crate::sizing::DocumentState, String> {
    core::prepare(d, 0, settings(op), |_, _| Ok(()))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    for op in O::defaults() {
        let p = run(&d, op)?;
        check(
            p != original
                && p.scene.last_detail_filter == Some(settings(op))
                && p.layers[0].placement == d.layers[0].placement
                && p.layers[0]
                    .pixels
                    .iter()
                    .zip(&d.layers[0].pixels)
                    .all(|(a, b)| a[3] == b[3])
                && d.snapshot() == original
                && d.history_counts() == counts,
            "Detail source isolation/record/alpha/extent",
        )?;
    }
    for op in [
        O::Sharpen { strength: 0. },
        O::Unsharp {
            radius: 0,
            amount: 1.,
            threshold: 0.,
        },
        O::Unsharp {
            radius: 3,
            amount: 0.,
            threshold: 0.,
        },
        O::Denoise {
            luminance: 0.,
            chroma: 0.,
            detail: 0.,
        },
    ] {
        check(run(&d, op)? == original, "Zero detail changed storage")?;
    }
    let mut uniform = fixture()?;
    uniform.layers[0].pixels.fill([37, 123, 210, 128]);
    for op in O::defaults() {
        check(
            run(&uniform, op)? == uniform.snapshot(),
            "Uniform field changed at a stored boundary",
        )?;
    }
    uniform.layers[0].pixels.fill([80, 80, 80, 255]);
    uniform.layers[0].pixels[24] = [200, 200, 200, 255];
    let noise = O::Denoise {
        luminance: 1.,
        chroma: 1.,
        detail: 0.01,
    };
    check(
        run(&uniform, noise)?.layers[0].pixels[24] == [80, 80, 80, 255],
        "Isolated point luminance noise was not removed",
    )?;
    uniform.layers[0].pixels[24] = [160, 64, 32, 255];
    let chroma = run(
        &uniform,
        O::Denoise {
            luminance: 0.,
            chroma: 1.,
            detail: 0.01,
        },
    )?;
    check(
        chroma.layers[0].pixels[24] == [80, 80, 80, 255],
        "Chroma reduction did not retain point luminance",
    )?;
    let luma = run(
        &uniform,
        O::Denoise {
            luminance: 1.,
            chroma: 0.,
            detail: 0.01,
        },
    )?;
    check(
        luma.layers[0].pixels[24] == [160, 64, 32, 255],
        "Luma reduction unexpectedly changed a point with matching luminance",
    )?;
    uniform.layers[0].pixels[24] = [192, 96, 64, 255];
    let luma = run(
        &uniform,
        O::Denoise {
            luminance: 1.,
            chroma: 0.,
            detail: 0.01,
        },
    )?;
    check(
        luma.layers[0].pixels[24] == [160, 64, 32, 255],
        "Luma reduction changed chroma components",
    )?;
    for y in 0..7 {
        uniform.layers[0].pixels[y * 7 + 3] = [0, 0, 0, 255];
    }
    check(
        run(&uniform, noise)?.layers == uniform.layers,
        "One-pixel continuous line erased by noise reduction",
    )?;
    // Independent four-neighbor Laplacian: center120, neighbors100/160/100/160 => -40.
    let sharp = run(&d, O::Sharpen { strength: 0.5 })?;
    check(
        sharp.layers[0].pixels[24] == [100, 100, 100, 255],
        "Sharpen Laplacian differs from reference",
    )?;
    let over = run(&d, O::Sharpen { strength: 5. })?;
    check(
        over.layers[0].pixels[32] == [255; 4] && over.layers[0].pixels[16] == [0, 0, 0, 255],
        "Sharpen overshoot not clamped",
    )?;
    let op = O::Unsharp {
        radius: 1,
        amount: 2.,
        threshold: 0.,
    };
    let p = run(&d, op)?;
    let edge = (-2f64).exp();
    let norm = 1. + 2. * edge;
    let mut blurred = 0.;
    for y in -1i32..=1 {
        for x in -1i32..=1 {
            let w = if x == 0 { 1. } else { edge } * if y == 0 { 1. } else { edge } / (norm * norm);
            blurred += d.layers[0].pixels[((3 + y) * 7 + 3 + x) as usize][0] as f64 * w;
        }
    }
    let expected = (120. + 2. * (120. - blurred)).clamp(0., 255.).round() as u8;
    check(
        p.layers[0].pixels[24] == [expected, expected, expected, 255],
        "Unsharp differs from independent dense Gaussian",
    )?;
    check(
        run(
            &d,
            O::Unsharp {
                radius: 3,
                amount: 5.,
                threshold: 1.,
            },
        )? == original,
        "Unsharp threshold1 changed pixels",
    )?;
    d.layers[0].pixels[0] = [0, 255, 0, 0];
    d.layers[0].pixels[1] = [255, 0, 0, 128];
    for op in O::defaults() {
        let p = run(&d, op)?;
        check(
            p.layers[0].pixels[0] == [0, 255, 0, 0] && p.layers[0].pixels[1][3] == 128,
            "Detail changed transparent hidden RGB/partial alpha",
        )?;
    }
    d.scene.selection = Some(crate::scene::Rect {
        min: [3., 3.],
        max: [4., 4.],
    });
    let selected = core::prepare(
        &d,
        0,
        Settings {
            options: Default::default(),
            selection: true,
            operation: op,
        },
        |_, _| Ok(()),
    )?;
    check(
        selected.layers[0]
            .pixels
            .iter()
            .zip(&d.layers[0].pixels)
            .enumerate()
            .all(|(i, (a, b))| i == 24 || a == b),
        "Detail changed selection exterior",
    )?;
    d.scene.selection_mask = Some(Default::default());
    check(
        core::prepare(
            &d,
            0,
            Settings {
                options: Default::default(),
                selection: true,
                operation: op,
            },
            |_, _| Ok(()),
        )? == d.snapshot(),
        "Empty detail selection changed document",
    )?;
    let mut high = fixture()?;
    high.color = Default::default();
    let mut values = vec![[30000, 30000, 30000, 10]; 49];
    values[24] = [30001, 31003, 32005, 10];
    high.layers[0].pixels = values.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    high.layers[0].precise = Some(values.clone());
    let p = run(&high, O::Sharpen { strength: 0.5 })?;
    check(
        p.layers[0].precise.as_ref().unwrap()[24] == [30003, 33009, 36015, 10]
            && p.layers[0].pixels[24][3] == 0,
        "Native16 low-alpha sharpen lost bits/coverage",
    )?;
    high.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&high, 0));
    check(
        run(&high, O::Sharpen { strength: 0.5 })?.layers == p.layers,
        "Alpha lock altered detail RGB result",
    )?;
    let mut cancelled = false;
    check(
        core::prepare(
            &high,
            0,
            settings(O::Unsharp {
                radius: 16,
                amount: 1.,
                threshold: 0.,
            }),
            |done, _| {
                if done >= 49 {
                    cancelled = true;
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && cancelled,
        "Unsharp cancellation ignored",
    )?;
    for op in [
        O::Sharpen { strength: 5.1 },
        O::Sharpen { strength: f64::NAN },
        O::Unsharp {
            radius: 33,
            amount: 1.,
            threshold: 0.,
        },
        O::Denoise {
            luminance: -0.1,
            chroma: 0.,
            detail: 0.,
        },
    ] {
        check(run(&high, op).is_err(), "Invalid detail accepted")?;
    }
    high.layers[0].locked = true;
    check(
        run(&high, O::defaults()[0]).is_err(),
        "Locked detail target accepted",
    )?;
    check(
        core::prepare_with_budget(&d, 0, settings(O::defaults()[0]), 16, |_, _| Ok(())).is_err(),
        "Insufficient workspace budget accepted",
    )?;
    let large = Document::create(
        CanvasSpec {
            width: 1024,
            height: 1024,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    check(
        run(
            &large,
            O::Unsharp {
                radius: 32,
                amount: 1.,
                threshold: 0.,
            },
        )
        .is_err(),
        "Excessive unsharp processing accepted",
    )?;
    let mut d = fixture()?;
    d.commit_state(sharp)?;
    let file = root.join("detail-filter.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Detail native/one Undo/Redo",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&70u32.to_le_bytes());
    let fake = root.join("invalid-v70-detail.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted detail recipe",
    )?;
    d.undo();
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&70u32.to_le_bytes());
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == original,
        "Compatible v70 no-record document failed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn detail_reference_uniform_noise_lines_threshold_precision_native_cancel() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
