use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    layer_blend::Mode,
    layer_properties::{self as p, Patch, Target},
    render::Renderer,
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
const GRAY: [f64; 28] = [
    0.8, 0.2, 0.16, 0., 0., 0., 0.2, 0.8, 0.84, 1., 0.8, 1., 1., 0.8, 0.32, 0.3488, 0.68, 0.5, 0.8,
    0.6, 1., 0.6, 0.68, 0.2, 0.2, 0.2, 0.8, 0.25,
];
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    check(
        Mode::ALL.len() == 28
            && Mode::ALL
                .iter()
                .map(|m| m.label())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 28,
        "Blend menu options are missing or aliased",
    )?;
    for (mode, expected) in Mode::ALL.into_iter().zip(GRAY) {
        check(
            mode.rgb([0.2; 3], [0.8; 3])
                .iter()
                .all(|v| (*v - expected).abs() < 1e-12),
            "Independent normalized gray blend value",
        )?;
        for ab in [0u8, 128, 255] {
            for a in [0u8, 128, 255] {
                for opacity in [0., 0.5, 1.] {
                    let bg = ab as f64 / 255.;
                    let src = a as f64 / 255. * opacity;
                    let alpha = src + bg * (1. - src);
                    let rgb = if alpha == 0. {
                        0
                    } else {
                        ((0.2 * bg * (1. - src) + 0.8 * src * (1. - bg) + expected * src * bg)
                            / alpha)
                            .clamp(0., 1.)
                            .mul_add(255., 0.)
                            .round() as u8
                    };
                    let expected = [rgb, rgb, rgb, (alpha * 255.).round() as u8];
                    let actual = mode.compose([51, 51, 51, ab], [204, 204, 204, a], opacity);
                    check(
                        actual
                            .into_iter()
                            .zip(expected)
                            .all(|(a, b)| a.abs_diff(b) <= 1),
                        "Independent source-over/opacity/empty-backdrop table",
                    )?;
                }
            }
        }
    }
    let b = [0.2, 0.4, 0.6];
    let s = [0.8, 0.2, 0.1];
    for (mode, expected) in [
        (
            Mode::Hue,
            [0.6082857142857143, 0.26542857142857146, 0.2082857142857143],
        ),
        (Mode::Saturation, [0.0785, 0.4285, 0.7785]),
        (Mode::Color, [0.793, 0.193, 0.093]),
        (Mode::Brightness, [0.207, 0.407, 0.607]),
    ] {
        check(
            mode.rgb(b, s)
                .into_iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-12),
            "Independent nonseparable RGB component replacement",
        )?;
    }
    check(
        Mode::Color
            .rgb([0.9; 3], [1., 0., 0.])
            .into_iter()
            .zip([1., 6. / 7., 6. / 7.])
            .all(|(a, b)| (a - b).abs() < 1e-12),
        "Gamut clipping did not preserve luminosity",
    )?;
    let lights = [
        Mode::SoftLight,
        Mode::HardLight,
        Mode::VividLight,
        Mode::LinearLight,
        Mode::PinLight,
    ];
    let expected = [0.4949647869859769, 0.74, 0.875, 0.95, 0.6];
    for (mode, expected) in lights.into_iter().zip(expected) {
        check(
            (mode.channel(0.35, 0.8) - expected).abs() < 1e-12,
            "Light mode variants share an incorrect alias",
        )?;
    }
    check(
        (Mode::ColorDodge.channel(0.1, 0.5) - 0.2).abs() < 1e-12
            && (Mode::GlowDodge.channel(0.1, 0.5) - 5. / 18.).abs() < 1e-12,
        "Color dodge/glow dodge alias",
    )?;
    check(
        Mode::Add.compose([204, 204, 204, 255], [204, 204, 204, 255], 0.5) == [230, 230, 230, 255]
            && Mode::AddGlow.compose([204, 204, 204, 255], [204, 204, 204, 255], 0.5) == [255; 4],
        "Clipped addition and delayed glow addition are not distinct",
    )?;
    check(
        Mode::DarkerColor.rgb([1., 0., 0.], [0., 1., 0.]) == [1., 0., 0.]
            && Mode::Darken.rgb([1., 0., 0.], [0., 1., 0.]) == [0.; 3]
            && Mode::LighterColor.rgb([1., 0., 0.], [0., 1., 0.]) == [0., 1., 0.]
            && Mode::Lighten.rgb([1., 0., 0.], [0., 1., 0.]) == [1., 1., 0.],
        "Whole-color and channel extrema conflated",
    )?;
    check(
        Mode::ColorBurn.compose([255, 255, 255, 128], [0, 0, 0, 128], 1.) == [170, 170, 170, 192],
        "Premultiplied white endpoint was not restored before burn",
    )?;
    check(
        Mode::GlowDodge.compose([64, 128, 192, 255], [0, 0, 0, 255], 1.) == [64, 128, 192, 255],
        "Black input incorrectly darkened glow dodge",
    )?;
    let colors = [
        [0, 0, 0],
        [255, 255, 255],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [64, 128, 192],
        [192, 64, 128],
    ];
    for mode in Mode::ALL {
        for b in colors {
            for s in colors {
                for ab in [0u8, 128, 255] {
                    for a in [0u8, 128, 255] {
                        let actual =
                            mode.compose([b[0], b[1], b[2], ab], [s[0], s[1], s[2], a], 1.);
                        let alpha = (a as u32 * 255 + ab as u32 * (255 - a as u32) + 127) / 255;
                        check(
                            actual[3] == alpha as u8 && (actual[3] != 0 || actual == [0; 4]),
                            "Primary-color/gray/transparency grid alpha",
                        )?;
                    }
                }
            }
        }
        let mut base = crate::paper::Color::default();
        base.add([80, 120, 160, 128]);
        let mut source = crate::paper::Color::default();
        source.add([160, 40, 20, 192]);
        mode.atop(&mut base, source);
        check(
            base.rgba()[3] == 128,
            "Clipping blend changed fixed base alpha",
        )?;
    }
    documents(root)?;
    proof()?;
    Ok(())
}
fn documents(root: &std::path::Path) -> Result<(), String> {
    for mode in Mode::ALL {
        let mut d = crate::layer_roles_verify::page()?;
        for (i, p) in d.layers[0].pixels.iter_mut().enumerate() {
            *p = [
                (i * 37 % 256) as u8,
                (i * 61 % 256) as u8,
                (i * 19 % 256) as u8,
                [0, 128, 255][i % 3],
            ];
        }
        let mut top = d.layers[0].clone();
        for (i, p) in top.pixels.iter_mut().enumerate() {
            *p = [
                (i * 47 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 71 % 256) as u8,
                [255, 128, 0][i % 3],
            ];
        }
        d.layers.push(top);
        let before = d.snapshot();
        let state = p::prepare_patch(
            &d,
            &[Target::Raster(2)],
            Patch {
                blend: Some(mode),
                opacity: Some(50),
                ..Default::default()
            },
        )?;
        d.commit_state(state)?;
        let renderer = Renderer::new(&d);
        for i in 0..64 {
            let expected = mode.compose(d.layers[0].pixels[i], d.layers[1].pixels[i], 0.5);
            check(
                renderer.pixel([(i % 8) as f64 + 0.5, (i / 8) as f64 + 0.5]) == expected,
                "Layer/grid render disagrees with selected blend",
            )?;
        }
        let path = root.join(format!("extended-blend-{mode:?}.drawer"));
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == d.snapshot() && d.undo() && d.snapshot() == before && d.redo(),
            "All-mode native persistence/Undo",
        )?;
        if mode != Mode::Normal {
            let note =
                crate::layer_blend::exchange_notice(&d).ok_or("Missing blend exchange warning")?;
            check(
                note.contains(mode.label()) && !crate::psd::layers_supported(&d),
                "Unsupported editable PSD mode was silently exported",
            )?;
            let psd = root.join("blend-exchange.psd");
            if psd.exists() {
                std::fs::remove_file(&psd).map_err(|e| e.to_string())?;
            }
            check(
                crate::psd::export(
                    &d,
                    &psd,
                    crate::psd::Choice::RenderedLayers,
                    false,
                    1024 * 1024,
                )
                .is_err()
                    && !psd.exists(),
                "Unsupported PSD layer mode wrote a partial file",
            )?;
            crate::psd::export(
                &d,
                &psd,
                crate::psd::Choice::RenderedImage,
                false,
                1024 * 1024,
            )
            .map_err(|e| e.to_string())?;
            let reopened = crate::psd::open(&psd, 1024 * 1024).map_err(|e| e.to_string())?;
            check(
                reopened.layers[0].pixels.iter().enumerate().all(|(i, p)| {
                    *p == Renderer::new(&d).pixel([(i % 8) as f64 + 0.5, (i / 8) as f64 + 0.5])
                }),
                "Rendered PSD lost extended blend",
            )?;
        }
    }
    Ok(())
}
fn proof() -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].pixels.fill([80, 120, 160, 128]);
    let before = d.snapshot();
    let bytes = crate::soft_proof::gray_fixture();
    let on = crate::soft_proof::Proof::from_bytes(
        &crate::soft_proof::Settings {
            enabled: true,
            ..Default::default()
        },
        &bytes,
    )?;
    let off = crate::soft_proof::Proof::from_bytes(&Default::default(), &bytes)?;
    for mode in Mode::ALL {
        let pixel = mode.compose([80, 120, 160, 128], [200, 40, 10, 128], 0.5);
        let mapped = on.map(pixel);
        check(
            mapped[3] == pixel[3]
                && off.map(pixel) == pixel
                && mode.compose([80, 120, 160, 128], [200, 40, 10, 128], 0.5) == pixel
                && d.snapshot() == before,
            "ICC view proof altered blend space/alpha or document",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn all_28_modes_independent_values_alpha_primary_grid_native_and_exchange() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
