use crate::{
    color_precision::Sample,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    pixel_effects::{self as pixels, Operation as P},
    stylized_effects::Operation as O,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn canvas(w: u32, h: u32, background: Background) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.,
            background,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn fixture() -> Result<Document, String> {
    let mut d = canvas(5, 5, Background::Transparent)?;
    d.layers[0].pixels.fill([0, 255, 0, 0]);
    d.layers[0].pixels[12] = [240, 120, 60, 128];
    Ok(d)
}
fn run(d: &Document, op: O) -> Result<crate::sizing::DocumentState, String> {
    pixels::prepare(
        d,
        0,
        P::Style(op),
        false,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )
}
pub fn zero(op: O) -> O {
    match op {
        O::Aberration { offset, .. } => O::Aberration {
            offset,
            strength: 0.,
        },
        O::Film {
            contrast,
            fade,
            temperature,
            grain,
            seed,
            ..
        } => O::Film {
            strength: 0.,
            contrast,
            fade,
            temperature,
            grain,
            seed,
        },
        O::Pencil {
            outline,
            shading,
            radius,
            ..
        } => O::Pencil {
            strength: 0.,
            outline,
            shading,
            radius,
        },
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let choices = crate::detail_filter::Operation::all();
    for (i, a) in choices.iter().enumerate() {
        for (j, b) in choices.iter().enumerate() {
            check(
                a.same_kind(*b) == (i == j),
                "Effect chooser marks a different effect as selected",
            )?;
        }
    }
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    for op in O::defaults() {
        check(
            run(&d, zero(op))? == original,
            "Zero style changed source storage",
        )?;
        let p = run(&d, op)?;
        check(
            p != original && d.snapshot() == original && d.history_counts() == counts,
            "Style source/history isolation",
        )?;
    }
    let op = O::Aberration {
        offset: [1, 0],
        strength: 1.,
    };
    let separated = run(&d, op)?;
    let mut edge = fixture()?;
    edge.layers[0].pixels.swap(10, 12);
    check(
        run(&edge, op)?.layers[0].pixel(5, 5, -1, 2) == [0, 0, 60, 128],
        "Off-canvas color fringe discarded",
    )?;
    for (x, value) in [
        (1, [0, 0, 60, 128]),
        (2, [0, 120, 0, 128]),
        (3, [240, 0, 0, 128]),
    ] {
        check(
            separated.layers[0].pixel(5, 5, x, 2) == value,
            "Channel displacement/alpha differs from independent reference",
        )?;
    }
    check(
        separated.layers[0].extent(5, 5).offset == [-1, 0],
        "Channel separation lost outside-canvas bounds",
    )?;
    let half = run(
        &d,
        O::Aberration {
            offset: [1, 0],
            strength: 0.5,
        },
    )?;
    check(
        half.layers[0].pixel(5, 5, 3, 2) == [240, 0, 0, 64]
            && half.layers[0].pixel(5, 5, 2, 2) == [120, 120, 30, 128],
        "Partial aberration gained black/hidden RGB fringe",
    )?;
    let vertical = run(
        &d,
        O::Aberration {
            offset: [0, -1],
            strength: 1.,
        },
    )?;
    check(
        vertical.layers[0].pixel(5, 5, 2, 1) == [240, 0, 0, 128]
            && vertical.layers[0].pixel(5, 5, 2, 3) == [0, 0, 60, 128],
        "Signed vertical channel offsets",
    )?;
    let mut high = fixture()?;
    high.color = Default::default();
    let mut values = vec![[0; 4]; 25];
    values[12] = [10000, 50000, 65535, 10];
    high.layers[0].pixels = values.iter().map(|v| Sample::from16(*v).rgba8()).collect();
    high.layers[0].precise = Some(values);
    check(
        run(&high, op)?.layers[0].sample(5, 5, 3, 2).rgba16() == [10000, 0, 0, 10],
        "Native16 low-alpha channel shift lost bits",
    )?;
    high.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&high, 0));
    let p = run(&high, op)?;
    check(
        p.layers[0].placement == high.layers[0].placement
            && p.layers[0]
                .precise
                .as_ref()
                .unwrap()
                .iter()
                .zip(high.layers[0].precise.as_ref().unwrap())
                .all(|(a, b)| a[3] == b[3])
            && p.layers[0].sample(5, 5, 2, 2).rgba16() == [0, 50000, 0, 10],
        "Alpha-locked16-bit aberration changed alpha/bounds or failed",
    )?;
    let mut gray = canvas(5, 5, Background::Color([64, 128, 192]))?;
    let film = O::Film {
        strength: 1.,
        contrast: 0.5,
        fade: 0.2,
        temperature: 0.25,
        grain: 0.,
        seed: 42,
    };
    let toned = run(&gray, film)?;
    let expected = [64f64, 128., 192.].map(|v| {
        let x = v / 255.;
        let curve = 0.5 * x + 0.5 * x * x * (3. - 2. * x);
        curve * 0.96 + 0.02
    });
    let expected = [
        ((expected[0] + 0.02) * 255.).round() as u8,
        (expected[1] * 255.).round() as u8,
        ((expected[2] - 0.02) * 255.).round() as u8,
        255,
    ];
    check(
        toned.layers[0].pixels[12] == expected,
        "Film curve/fade/tint differs from independent reference",
    )?;
    let grain = O::Film {
        strength: 1.,
        contrast: 0.5,
        fade: 0.2,
        temperature: 0.25,
        grain: 0.8,
        seed: 42,
    };
    let a = run(&gray, grain)?;
    check(
        run(&gray, grain)? == a
            && run(
                &gray,
                O::Film {
                    strength: 1.,
                    contrast: 0.5,
                    fade: 0.2,
                    temperature: 0.25,
                    grain: 0.8,
                    seed: 43,
                },
            )?
            .layers
                != a.layers,
        "Film grain seed reproducibility/change",
    )?;
    let neutral = O::Film {
        strength: 1.,
        contrast: 0.,
        fade: 0.,
        temperature: 0.,
        grain: 0.,
        seed: u64::MAX,
    };
    check(
        run(&gray, neutral)? == gray.snapshot(),
        "Neutral film values changed storage",
    )?;
    let shade = run(
        &d,
        O::Pencil {
            strength: 1.,
            outline: 0.,
            shading: 1.,
            radius: 1,
        },
    )?;
    check(
        shade.layers[0].pixel(5, 5, 2, 2) == [141, 141, 141, 128],
        "Pencil luminance/shading independent reference",
    )?;
    for y in 0..5 {
        for x in 0..5 {
            gray.layers[0].pixels[y * 5 + x] = if x < 2 { [0, 0, 0, 255] } else { [255; 4] };
        }
    }
    let outline = run(
        &gray,
        O::Pencil {
            strength: 1.,
            outline: 1.,
            shading: 0.,
            radius: 1,
        },
    )?;
    check(
        outline.layers[0].pixel(5, 5, 2, 2) == [0, 0, 0, 255]
            && outline.layers[0].pixel(5, 5, 4, 2) == [255; 4],
        "Pencil Sobel contour or uniform paper",
    )?;
    let no_outline = run(
        &gray,
        O::Pencil {
            strength: 1.,
            outline: 0.,
            shading: 0.,
            radius: 1,
        },
    )?;
    check(
        no_outline.layers[0].pixels.iter().all(|p| *p == [255; 4]),
        "Pencil contour/shading not independent",
    )?;
    for op in O::defaults().into_iter().skip(1) {
        let p = run(&d, op)?;
        check(
            p.layers[0].pixels[0] == d.layers[0].pixels[0]
                && p.layers[0]
                    .pixels
                    .iter()
                    .zip(&d.layers[0].pixels)
                    .all(|(a, b)| a[3] == b[3]),
            "Film/pencil changed hidden transparent RGB or coverage",
        )?;
        let p = run(&high, op)?;
        check(
            p.layers[0].precise.as_ref().unwrap()[12][3] == 10,
            "Film/pencil lost sub-byte alpha",
        )?;
    }
    d.scene.selection = Some(crate::scene::Rect {
        min: [2., 2.],
        max: [3., 3.],
    });
    let selected = pixels::prepare(
        &d,
        0,
        P::Style(op),
        true,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        selected.layers[0].pixel(5, 5, 1, 2)[3] == 0
            && selected.layers[0].pixel(5, 5, 3, 2)[3] == 0,
        "Style escaped selection",
    )?;
    d.scene.selection = None;
    for op in [
        O::Aberration {
            offset: [33, 0],
            strength: 1.,
        },
        O::Film {
            strength: f64::NAN,
            contrast: 0.,
            fade: 0.,
            temperature: 0.,
            grain: 0.,
            seed: 0,
        },
        O::Pencil {
            strength: 1.,
            outline: 1.,
            shading: 1.,
            radius: 0,
        },
    ] {
        check(run(&d, op).is_err(), "Invalid style accepted")?;
    }
    let wide = canvas(32768, 1, Background::Transparent)?;
    check(
        run(&wide, op).is_err(),
        "Oversized aberration extent accepted",
    )?;
    let mut cancelled = false;
    check(
        pixels::prepare(
            &d,
            0,
            P::Style(O::Aberration {
                offset: [32, 0],
                strength: 1.,
            }),
            false,
            DEFAULT_DOCUMENT_BUDGET,
            |done, _| {
                if done >= 128 {
                    cancelled = true;
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && cancelled
            && d.snapshot() == original,
        "Style cancellation changed source/ignored signal",
    )?;
    d.commit_state(separated)?;
    let file = root.join("stylized-effect.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Style settings/native/one Undo/Redo",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&72u32.to_le_bytes());
    let fake = root.join("invalid-v72-style.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted style recipe",
    )?;
    let mut legacy = crate::pixel_effects_verify::fixture()?;
    legacy.commit_state(pixels::prepare(
        &legacy,
        0,
        P::defaults()[0],
        false,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?)?;
    crate::files::save(&mut legacy, &file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&72u32.to_le_bytes());
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == legacy.snapshot(),
        "Existing v72 pixel recipe failed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn style_independent_channels_film_pencil_alpha_precision_seed_native_cancel() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
