use crate::{
    color_deepen::{self as core, Destination, Settings},
    color_precision::{Depth, Sample},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels = vec![
        [96, 96, 96, 127],
        [210, 90, 40, 255],
        [250, 20, 10, 200],
        [9, 0, 255, 0],
    ];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let ordinary = Settings {
        brightness: 0.,
        saturation: 0.5,
        contrast: 0.,
    };
    let mapped = crate::color_profile::WorkingSpace::new(&crate::color_profile::RgbProfile::Srgb)?
        .adjusted(Sample::new([0.2, 0.4, 0.6, 0.5])?, ordinary.adjustment())?;
    check(
        mapped
            .rgb()
            .iter()
            .zip([0.1, 0.4, 0.7])
            .all(|(a, b)| (a - b).abs() < 1e-10)
            && mapped.alpha() == 0.5,
        "Independent HSL saturation preserves hue/lightness/alpha",
    )?;
    let contrast = Settings {
        brightness: 0.,
        saturation: 0.,
        contrast: 0.5,
    };
    let slope = 1. / (1. - 0.99 * 0.25);
    let mapped = crate::color_profile::WorkingSpace::new(&crate::color_profile::RgbProfile::Srgb)?
        .adjusted(Sample::new([0.2, 0.4, 0.6, 0.5])?, contrast.adjustment())?;
    check(
        mapped
            .rgb()
            .iter()
            .zip([0.5 - 0.3 * slope, 0.5 - 0.1 * slope, 0.5 + 0.1 * slope])
            .all(|(a, b)| (a - b).abs() < 1e-10),
        "Midtone contrast is independent from saturation",
    )?;
    let mut d = fixture()?;
    let settings = Settings {
        brightness: 0.03,
        saturation: 0.6,
        contrast: 0.5,
    };
    let original = d.snapshot();
    let id = d.scene.layer_ids.id(0);
    let pixels = core::prepare(&d, Destination::Pixels(id), settings, false, |_, _| Ok(()))?;
    let layer = core::prepare(&d, Destination::Layer, settings, false, |_, _| Ok(()))?;
    check(
        pixels.state.layers[0] != d.layers[0]
            && layer.state.layers[0] == d.layers[0]
            && layer.state.layers.len() == 2,
        "Pixel and correction-layer destinations are distinct",
    )?;
    let gray = pixels.state.layers[0].pixels[0];
    check(
        gray[0] == gray[1] && gray[1] == gray[2],
        "Gray input must not acquire chroma",
    )?;
    check(
        pixels.clipped > 0
            && pixels.checked == 3
            && pixels.warning.pixels.contains(&[255, 32, 32, 255]),
        "Clipping counts and red warning samples are visible",
    )?;
    for (a, b) in pixels.state.layers[0]
        .pixels
        .iter()
        .zip(&d.layers[0].pixels)
    {
        check(a[3] == b[3], "Pixel alpha must stay exact")?;
    }
    check(
        pixels.state.layers[0].pixels[3] == d.layers[0].pixels[3],
        "Hidden transparent RGB is preserved",
    )?;
    let pdoc = crate::layer_merge::preview(&pixels.state, d.current_frame)?;
    let ldoc = crate::layer_merge::preview(&layer.state, d.current_frame)?;
    for x in 0..4 {
        let p = crate::render::Renderer::new(&pdoc).pixel([x as f64 + 0.5, 0.5]);
        let l = crate::render::Renderer::new(&ldoc).pixel([x as f64 + 0.5, 0.5]);
        check(
            p.iter().zip(l).all(|(a, b)| a.abs_diff(b) <= 1),
            "Both destinations share the floating color formula (storage rounding only)",
        )?;
    }
    for prepared in [pixels, layer] {
        let counts = d.history_counts();
        d.commit_state(prepared.state)?;
        check(
            d.history_counts().0 == counts.0 + 1,
            "One color-deepen command is one Undo",
        )?;
        let path = root.join("color-deepen.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Color-deepen native preserves parameters and mode",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&77u32.to_le_bytes());
        let path = root.join("color-deepen-old.drawer");
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Old header rejects color-deepen records",
        )?;
        check(
            d.undo() && d.snapshot() == original,
            "Undo restores all sources and parameters",
        )?;
    }
    for destination in [Destination::Pixels(id), Destination::Layer] {
        let zero = Settings {
            brightness: 0.,
            saturation: 0.,
            contrast: 0.,
        };
        let counts = d.history_counts();
        let dirty = d.dirty;
        d.commit_state(core::prepare(&d, destination, zero, false, |_, _| Ok(()))?.state)?;
        check(
            d.snapshot() == original && d.history_counts() == counts && d.dirty == dirty,
            "Neutral settings preserve dirty/Redo/last values",
        )?;
    }
    crate::selection_tools::rectangle(&mut d, [1., 0.], [2., 1.])?;
    d.scene.transform = crate::scene::Affine([1., 0., 0., 1., 1., 0.]);
    let selected = d.snapshot();
    let p = core::prepare(&d, Destination::Pixels(id), settings, true, |_, _| Ok(()))?;
    check(
        p.state.layers[0].pixels[..2] == d.layers[0].pixels[..2]
            && p.state.layers[0].pixels[3] == d.layers[0].pixels[3],
        "Pixels outside the transformed selection remain exact",
    )?;
    check(
        p.state.layers[0].pixels[2] != d.layers[0].pixels[2],
        "Selection follows document transform in pixel mode",
    )?;
    let l = core::prepare(&d, Destination::Layer, settings, true, |_, _| Ok(()))?;
    check(
        l.state.layers[0] == d.layers[0]
            && l.state
                .scene
                .layer_effects
                .rasters
                .values()
                .any(|e| e.mask.is_some() && e.adjustment.is_some()),
        "Correction stores a frozen selection mask",
    )?;
    let pdoc = crate::layer_merge::preview(&p.state, d.current_frame)?;
    let ldoc = crate::layer_merge::preview(&l.state, d.current_frame)?;
    for x in 0..4 {
        let a = crate::render::Renderer::new(&pdoc).pixel([x as f64 + 0.5, 0.5]);
        let b = crate::render::Renderer::new(&ldoc).pixel([x as f64 + 0.5, 0.5]);
        check(
            a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1),
            "Selection-limited modes agree",
        )?;
    }
    let mut stopped = false;
    check(
        core::prepare(&d, Destination::Pixels(id), settings, true, |done, _| {
            if done > 0 {
                stopped = true;
                Err("cancel".into())
            } else {
                Ok(())
            }
        })
        .is_err()
            && stopped
            && d.snapshot() == selected,
        "Mid-work cancellation has no partial result",
    )?;
    check(
        core::prepare(
            &d,
            Destination::Pixels(id),
            Settings {
                brightness: f64::NAN,
                ..settings
            },
            false,
            |_, _| Ok(()),
        )
        .is_err(),
        "Nonfinite settings rejected",
    )?;
    d.layers[0].locked = true;
    check(
        core::prepare(&d, Destination::Pixels(id), settings, false, |_, _| Ok(())).is_err(),
        "Locked pixel source rejected",
    )?;
    d.layers[0].locked = false;
    d.scene.selection_mask = Some(Default::default());
    check(
        core::prepare(&d, Destination::Layer, settings, true, |_, _| Ok(()))?.state == d.snapshot(),
        "Explicit empty selection is an exact no-op",
    )?;
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
    let sample = Sample::from16([12345, 23456, 34567, 1]);
    high.layers[0].pixels[0] = sample.rgba8();
    high.layers[0].precise.as_mut().unwrap()[0] = sample.rgba16();
    let s = core::prepare(
        &high,
        Destination::Pixels(1),
        settings,
        false,
        |_, _| Ok(()),
    )?;
    let expected = crate::color_profile::WorkingSpace::new(&high.color.profile)?
        .adjusted(sample, settings.adjustment())?
        .rgba16();
    check(
        s.state.layers[0].precise.as_ref().unwrap()[0] == expected && expected[3] == 1,
        "Native low bits and extreme low alpha survive",
    )?;
    let mut wide = Document::create_with_color(
        CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        crate::color_storage::Config {
            depth: Depth::Sixteen,
            profile: crate::color_profile::RgbProfile::embedded(
                "Display P3".into(),
                moxcms::ColorProfile::new_display_p3()
                    .encode()
                    .map_err(|e| e.to_string())?,
            )?,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let samples = [[42000, 20000, 5000, 30000], [10000, 35000, 50000, 60000]];
    wide.layers[0].pixels = samples.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    wide.layers[0].precise = Some(samples.to_vec());
    wide.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    let p = core::prepare(&wide, Destination::Pixels(1), settings, true, |_, _| Ok(()))?;
    let l = core::prepare(&wide, Destination::Layer, settings, true, |_, _| Ok(()))?;
    let pdoc = crate::layer_merge::preview(&p.state, wide.current_frame)?;
    let ldoc = crate::layer_merge::preview(&l.state, wide.current_frame)?;
    let a = crate::float_render::Renderer::new(&pdoc)?
        .pixel([0.5, 0.5])?
        .rgba16();
    let b = crate::float_render::Renderer::new(&ldoc)?
        .pixel([0.5, 0.5])?
        .rgba16();
    check(
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2),
        &format!("Partial selection/profile mixing must agree: {a:?} vs {b:?}"),
    )?;
    check(
        p.state.layers[0].precise.as_ref().unwrap()[1] == samples[1]
            && a[3] == samples[0][3]
            && b[3] == samples[0][3],
        "Profile soft-selection preserves outside native data and exact alpha",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn color_deepen_modes_gray_clipping_selection_precision_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
