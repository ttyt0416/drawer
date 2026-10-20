use crate::color_precision::{Depth, Dither, Pixels, Sample};
pub fn display_p3_fixture() -> Vec<u8> {
    moxcms::ColorProfile::new_display_p3().encode().unwrap()
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify() -> Result<(), String> {
    check(
        Depth::default() == Depth::Sixteen && !Dither::default().enabled,
        "Precision/dither defaults",
    )?;
    for depth in [Depth::Eight, Depth::Sixteen] {
        let m = depth.maximum();
        for n in 0..m {
            let boundary = (n as f64 + 0.5) / m as f64;
            for (value, expected) in [(boundary - 1e-12, n), (boundary + 1e-12, n + 1)] {
                let out = Sample::new([value, value, value, 1.])?.store(
                    depth,
                    Default::default(),
                    [0; 2],
                );
                check(
                    out == [expected as u16, expected as u16, expected as u16, m as u16],
                    "Half-quantization boundary +/- epsilon",
                )?;
            }
        }
    }
    let source = (0..=255)
        .map(|n| [n, n, 255 - n, if n == 0 { 0 } else { n }])
        .collect::<Vec<_>>();
    let a = Pixels::Eight(source.clone());
    let b = a.convert(Depth::Sixteen, Default::default(), 256)?;
    let c = b.convert(Depth::Eight, Default::default(), 256)?;
    let mut expected = source;
    expected[0] = [0; 4];
    check(
        c == Pixels::Eight(expected),
        "8/16 integer roundtrip or transparent normalization",
    )?;
    let samples = (0..=65535)
        .map(|n| {
            Sample::new([n as f64 / 65535., n as f64 / 65535., n as f64 / 65535., 1.]).unwrap()
        })
        .collect::<Vec<_>>();
    let ramp = Pixels::from_samples(&samples, Depth::Sixteen, Default::default(), 65536)?;
    let Pixels::Sixteen(values) = &ramp else {
        return Err("Wrong precision storage".into());
    };
    check(
        values
            .iter()
            .enumerate()
            .all(|(n, p)| *p == [n as u16, n as u16, n as u16, 65535]),
        "16-bit gradient lost low bits",
    )?;
    let low = Sample::new([0.123456, 0.456789, 0.789012, 0.00001])?;
    check(
        low.rgba8() == [0; 4],
        "Very low alpha must be zero only at final 8-bit storage",
    )?;
    let mut accumulated = Sample::default();
    for _ in 0..1000 {
        accumulated = accumulated.over(low, crate::layer_blend::Mode::Normal, 1.)?;
    }
    let alpha = 1. - (1. - 0.00001f64).powi(1000);
    let reference = [0.123456, 0.456789, 0.789012, alpha];
    let actual = accumulated.rgba16();
    let expected = reference.map(|v| (v * 65535. + 0.5).floor() as u16);
    check(
        actual == expected && actual[3] > 600,
        "1000 half-transparent dabs rounded during accumulation",
    )?;
    check(
        accumulated.rgba8()[3] == 3,
        "Final 8-bit export of accumulated fractional alpha",
    )?;
    let mut output = Sample::new([0.3, 0.6, 0.2, 0.4])?;
    let mut premul = [0.12, 0.24, 0.08];
    let mut alpha = 0.4;
    for n in 0..1000 {
        let rgb = [
            (n % 97) as f64 / 96.,
            (n % 89) as f64 / 88.,
            (n % 83) as f64 / 82.,
        ];
        let a = (n % 17 + 1) as f64 / 10000.;
        output = output.over(
            Sample::new([rgb[0], rgb[1], rgb[2], a])?,
            crate::layer_blend::Mode::Normal,
            1.,
        )?;
        for i in 0..3 {
            premul[i] = rgb[i] * a + premul[i] * (1. - a);
        }
        alpha = a + alpha * (1. - a);
    }
    let expected = Sample::new([
        premul[0] / alpha,
        premul[1] / alpha,
        premul[2] / alpha,
        alpha,
    ])?
    .rgba16();
    check(
        output
            .rgba16()
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 1),
        "1000-color source-over versus independent premultiplied float64 reference",
    )?;
    let value = Sample::new([0.501, 0.502, 0.503, 0.5])?;
    let dither = Dither {
        enabled: true,
        seed: 1729,
    };
    let first = (0..1024)
        .map(|x| value.store(Depth::Eight, dither, [x, 23]))
        .collect::<Vec<_>>();
    check(
        first
            == (0..1024)
                .map(|x| value.store(Depth::Eight, dither, [x, 23]))
                .collect::<Vec<_>>()
            && first.windows(2).any(|p| p[0] != p[1])
            && first.iter().all(|p| p[3] == 128),
        "Dither reproducibility, variation or alpha bypass",
    )?;
    let adjusted =
        Sample::new([0.251, 0.502, 0.753, 0.1234])?.adjusted(crate::layer_effects::Adjustment {
            brightness: 0.1,
            ..Default::default()
        })?;
    check(
        adjusted.rgba16() == Sample::new([0.351, 0.602, 0.853, 0.1234])?.rgba16(),
        "Float correction rounded through an 8-bit stage",
    )?;
    for alpha in [0., 0.5, 1.] {
        let source = Sample::new([0.5, 0.5, 0.5, alpha])?;
        let mono = source.expression(crate::layer_search::Expression::Mono, 0.5, 0.5)?;
        check(
            mono.rgba16() == if alpha == 0. { [0; 4] } else { [65535; 4] },
            "High precision expression >= thresholds/alpha",
        )?;
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        check(
            Sample::new([invalid, 0., 0., 1.]).is_err(),
            "Nonfinite color accepted",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::Sample;
    #[test]
    fn extended_rgb_is_clamped_at_storage_and_brush_burn_uses_its_own_formula() {
        let sample = Sample::new([-0.2, 1.2, 0.5, 0.5]).unwrap();
        assert_eq!(sample.components(), [-0.2, 1.2, 0.5, 0.5]);
        assert_eq!(sample.rgba16(), [0, 65535, 32768, 32768]);
        let base = Sample::new([0.4, 0.4, 0.4, 0.5]).unwrap();
        let source = Sample::new([0.2, 0.2, 0.2, 0.5]).unwrap();
        let brush = base
            .brush(
                source,
                crate::brush_composite::Mode::BlackBurn,
                0.5,
                [0; 2],
                0,
            )
            .unwrap();
        assert_eq!(
            brush.rgba16(),
            Sample::new([0.24, 0.24, 0.24, 0.625]).unwrap().rgba16()
        );
        assert_ne!(
            brush.rgba16(),
            base.over(source, crate::layer_blend::Mode::ColorBurn, 0.5)
                .unwrap()
                .rgba16()
        );
    }
    #[test]
    fn float_mix_coordinates_and_tonal_preserve_sub_byte_values() {
        let source = Sample::from16([12345, 23456, 34567, 32768]);
        for space in crate::color_mix::Space::ALL {
            let roundtrip = crate::color_mix::rgb_float(
                space,
                crate::color_mix::coordinates_float(space, source.rgb()),
            );
            assert!(
                source
                    .with_rgb(roundtrip)
                    .unwrap()
                    .rgba16()
                    .iter()
                    .zip(source.rgba16())
                    .all(|(a, b)| a.abs_diff(b) <= 1)
            );
        }
        let b = Sample::from16([56789, 45678, 34567, 12345]);
        assert_eq!(
            source
                .mix_space(b, 0.317, crate::color_mix::Space::Standard, 0.)
                .unwrap()
                .rgba16(),
            source.mix(b, 0.317).unwrap().rgba16()
        );
        let gray = Sample::new([0.5, 0.5, 0.5, 0.12345]).unwrap();
        let adjusted = gray
            .tonal(
                crate::tonal::Settings {
                    mode: crate::tonal::Mode::Dodge,
                    band: crate::tonal::Band::Midtones,
                    strength: 0.1,
                },
                1.,
            )
            .unwrap();
        assert_eq!(
            adjusted.rgba16(),
            Sample::new([0.55, 0.55, 0.55, 0.12345]).unwrap().rgba16()
        );
        assert_ne!(adjusted.rgba16(), Sample::from8(adjusted.rgba8()).rgba16());
        let mixed = Sample::new([0.5, 0.5, 0.5, 1.]).unwrap();
        let corrected = crate::lab_mix::correct_float(
            mixed,
            Sample::new([0., 0., 0., 1.]).unwrap(),
            Sample::new([1.; 4]).unwrap(),
            0.5,
            1.,
        )
        .unwrap();
        assert!((corrected.components()[0] - 0.46632660928353725).abs() < 1e-7);
    }
    #[test]
    fn precision_half_boundaries_1000_dabs_gradient_dither_corrections() {
        super::verify().unwrap();
    }
    #[test]
    fn embedded_float_rgb_profiles_identity_whitepoint_alpha_and_rejection() {
        use crate::color_profile::{RgbProfile, Transform, WorkingSpace};
        use moxcms::ColorProfile;
        let sample = Sample::from16([12345, 34567, 56789, 32768]);
        for profile in [
            ColorProfile::new_srgb(),
            ColorProfile::new_display_p3(),
            ColorProfile::new_pro_photo_rgb(),
        ] {
            let embedded =
                RgbProfile::embedded("Local test RGB".into(), profile.encode().unwrap()).unwrap();
            assert_eq!(
                Transform::new(&embedded, &embedded)
                    .unwrap()
                    .map(sample)
                    .unwrap(),
                sample
            );
            let working = WorkingSpace::new(&embedded).unwrap();
            for alpha in [0, 32768, 65535] {
                let gray = Sample::from16([32768, 32768, 32768, alpha]);
                let mapped = working.to_srgb(gray).unwrap();
                assert_eq!(mapped.rgba16()[3], alpha);
                let restored = working.from_srgb(mapped).unwrap();
                assert!(
                    restored
                        .rgba16()
                        .iter()
                        .zip(gray.rgba16())
                        .all(|(a, b)| a.abs_diff(b) <= 8)
                );
            }
        }
        let p3 = RgbProfile::embedded(
            "P3".into(),
            ColorProfile::new_display_p3().encode().unwrap(),
        )
        .unwrap();
        let patch = Sample::new([0.7, 0.3, 0.2, 0.5]).unwrap();
        let mapped = Transform::new(&p3, &RgbProfile::Srgb)
            .unwrap()
            .map(patch)
            .unwrap();
        assert_ne!(mapped.rgba16()[..3], patch.rgba16()[..3]);
        assert_eq!(mapped.alpha(), patch.alpha());
        assert_eq!(
            Transform::new(&RgbProfile::AssumedSrgb, &RgbProfile::Srgb)
                .unwrap()
                .map(sample)
                .unwrap(),
            sample
        );
        assert!(
            RgbProfile::embedded(
                "Gray must be converted explicitly".into(),
                ColorProfile::new_gray_with_gamma(2.2).encode().unwrap()
            )
            .is_err()
        );
        assert!(RgbProfile::embedded("Invalid".into(), vec![0; 512]).is_err());
    }
    #[test]
    fn wide_rgb_normal_identity_and_fractional_composition_match_unclamped_cms_reference() {
        use crate::color_profile::{RgbProfile, Transform, WorkingSpace};
        use moxcms::{ColorProfile, Layout, TransformOptions};
        let p3 = ColorProfile::new_display_p3();
        let bytes = p3.encode().unwrap();
        let profile = RgbProfile::embedded("P3".into(), bytes.clone()).unwrap();
        let p3 = ColorProfile::new_from_slice(&bytes).unwrap();
        let space = WorkingSpace::new(&profile).unwrap();
        let red = Sample::new([1., 0., 0., 0.5]).unwrap();
        assert_eq!(
            space
                .compose(Sample::default(), red, crate::layer_blend::Mode::Normal, 1.)
                .unwrap()
                .rgba16(),
            red.rgba16()
        );
        assert_eq!(
            space
                .compose(red, red, crate::layer_blend::Mode::Normal, 1.)
                .unwrap()
                .rgba16(),
            [65535, 0, 0, 49151]
        );
        let mapped = Transform::new(&profile, &RgbProfile::Srgb)
            .unwrap()
            .map(red)
            .unwrap();
        assert!(mapped.rgb().iter().any(|v| *v < 0. || *v > 1.));
        let srgb = ColorProfile::new_srgb();
        let options = TransformOptions {
            rendering_intent: moxcms::RenderingIntent::RelativeColorimetric,
            prefer_fixed_point: false,
            allow_extended_range_rgb_xyz: true,
            ..Default::default()
        };
        let forward = p3
            .create_transform_f64(Layout::Rgb, &srgb, Layout::Rgb, options)
            .unwrap();
        let reverse = srgb
            .create_transform_f64(Layout::Rgb, &p3, Layout::Rgb, options)
            .unwrap();
        let a = [1., 0., 0.];
        let b = [0., 0., 1.];
        let mut ar = [0.; 3];
        let mut br = [0.; 3];
        forward.transform(&a, &mut ar).unwrap();
        forward.transform(&b, &mut br).unwrap();
        let aa = 0.5;
        let ba = 0.25;
        let alpha = ba + aa * (1. - ba);
        let working = [0, 1, 2].map(|i| (br[i] * ba + ar[i] * aa * (1. - ba)) / alpha);
        let mut output = [0.; 3];
        reverse.transform(&working, &mut output).unwrap();
        let reference = Sample::new([output[0], output[1], output[2], alpha])
            .unwrap()
            .rgba16();
        let actual = space
            .compose(
                red,
                Sample::new([0., 0., 1., ba]).unwrap(),
                crate::layer_blend::Mode::Normal,
                1.,
            )
            .unwrap()
            .rgba16();
        assert!(
            actual
                .iter()
                .zip(reference)
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
    }
}
