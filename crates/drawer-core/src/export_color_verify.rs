use super::*;
use crate::{
    color_precision::Depth,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
    raster_io::{self, Options},
};
use std::{fs, path::Path, sync::atomic::AtomicBool};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
fn raw_png(bytes: &[u8]) -> Result<Vec<[u16; 4]>, String> {
    let mut r = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(|e| e.to_string())?;
    let mut b = vec![0; r.output_buffer_size().ok_or("PNG allocation")?];
    let info = r.next_frame(&mut b).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Sixteen || info.color_type != png::ColorType::Rgba {
        return Err("Expected raw RGBA16 PNG".into());
    }
    Ok(b.as_chunks::<8>()
        .0
        .iter()
        .map(|s| [0, 1, 2, 3].map(|n| u16::from_be_bytes([s[2 * n], s[2 * n + 1]])))
        .collect())
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("export-color-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 5,
            height: 1,
            dpi: 299.75,
            background: Background::Transparent,
        },
        B,
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let pixels = vec![
        [12345, 23456, 34567, 65535],
        [65535, 0, 65535, 32768],
        [0; 4],
        [65535; 4],
        [16780, 59861, 14670, 65535],
    ];
    d.layers[0].pixels = pixels.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    d.layers[0].precise = Some(pixels.clone());
    let before = d.snapshot();
    let identity = (d.path.clone(), d.dirty, d.history_counts());
    let c = AtomicBool::new(false);
    let p3_bytes = crate::color_precision_verify::display_p3_fixture();
    let p3_path = dir.join("output-p3.icc");
    fs::write(&p3_path, &p3_bytes).map_err(|e| e.to_string())?;
    let own = dir.join("own-srgb.icc");
    fs::write(&own, profile_bytes(&RgbProfile::Srgb)?).map_err(|e| e.to_string())?;
    let o = Options {
        format: Format::Png,
        alpha: true,
        depth: Depth::Sixteen,
        ..Default::default()
    };
    let mut s = Settings {
        profile: Some(p3_path.clone()),
        convert: false,
        ..Default::default()
    };
    let embed = raster_io::prepare_managed(&d, B, &c, &s, o)?;
    check(
        embed.pixels == pixels && raw_png(&embed.encode(o, B, &c)?)? == pixels,
        "Embed-only altered numeric RGB16",
    )?;
    for intent in [
        Intent::Perceptual,
        Intent::Relative,
        Intent::Saturation,
        Intent::Absolute,
    ] {
        s.convert = true;
        s.intent = intent;
        let p = raster_io::prepare_managed(&d, B, &c, &s, o)?;
        if intent != Intent::Absolute {
            let target =
                moxcms::ColorProfile::new_from_slice(&p3_bytes).map_err(|e| e.to_string())?;
            let native_intent = match intent {
                Intent::Perceptual => moxcms::RenderingIntent::Perceptual,
                Intent::Relative => moxcms::RenderingIntent::RelativeColorimetric,
                Intent::Saturation => moxcms::RenderingIntent::Saturation,
                Intent::Absolute => moxcms::RenderingIntent::AbsoluteColorimetric,
            };
            let expected = moxcms::ColorProfile::new_srgb()
                .create_transform_f64(
                    moxcms::Layout::Rgb,
                    &target,
                    moxcms::Layout::Rgb,
                    moxcms::TransformOptions {
                        rendering_intent: native_intent,
                        prefer_fixed_point: false,
                        allow_extended_range_rgb_xyz: true,
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?;
            for (a, b) in p.pixels.iter().zip(&pixels) {
                let sample = Sample::from16(*b);
                let mut rgb = [0.; 3];
                expected
                    .transform(&sample.rgb(), &mut rgb)
                    .map_err(|e| e.to_string())?;
                let want = if sample.alpha() == 0. {
                    [0; 4]
                } else {
                    sample.with_rgb(rgb)?.rgba16()
                };
                check(
                    *a == want,
                    "Output rendering intent/native CMS reference or alpha mismatch",
                )?;
            }
        } else {
            check(
                p.pixels.iter().zip(&pixels).all(|(a, b)| a[3] == b[3]),
                "Absolute transform changed alpha",
            )?;
        }
        check(
            p.losses(o)
                .iter()
                .any(|s| s.contains(&format!("{intent:?}"))),
            "Output intent hidden",
        )?;
        let same = Settings {
            profile: Some(own.clone()),
            intent,
            ..Default::default()
        };
        check(
            raster_io::prepare_managed(&d, B, &c, &same, o)?.pixels == pixels,
            "Same RGB profile conversion drifted low bits",
        )?;
    }
    s.intent = Intent::Relative;
    s.convert = true;
    let converted = raster_io::prepare_managed(&d, B, &c, &s, o)?;
    check(converted.pixels != pixels, "RGB conversion not applied")?;
    let eight = Options {
        depth: Depth::Eight,
        ..o
    };
    let encoded = converted.encode(eight, B, &c)?;
    let mut reader = png::Decoder::new(std::io::Cursor::new(encoded))
        .read_info()
        .map_err(|e| e.to_string())?;
    let mut raw = vec![0; reader.output_buffer_size().ok_or("PNG8 output buffer")?];
    reader.next_frame(&mut raw).map_err(|e| e.to_string())?;
    let display = Transform::with_intent(
        &RgbProfile::embedded("Encoded P3".into(), p3_bytes.clone())?,
        &RgbProfile::Srgb,
        Intent::Relative,
    )?;
    for (n, pixel) in raw.as_chunks::<4>().0.iter().enumerate() {
        check(
            converted.preview_sample(n, eight)?.rgba8()
                == display.map(Sample::from8(*pixel))?.rgba8(),
            "Output proof did not simulate actual encoded channel quantization",
        )?;
    }

    for format in [Format::Png, Format::Jpeg, Format::Tiff, Format::Webp] {
        for include in [false, true] {
            s.embed = include;
            let options = Options {
                format,
                alpha: format != Format::Jpeg,
                ..Default::default()
            };
            let p = raster_io::prepare_managed(&d, B, &c, &s, options)?;
            let bytes = p.encode(options, B, &c)?;
            let profile = match format {
                Format::Tiff => {
                    let mut reader = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes))
                        .map_err(|e| e.to_string())?;
                    if reader
                        .find_tag(tiff::tags::Tag::IccProfile)
                        .map_err(|e| e.to_string())?
                        .is_some()
                    {
                        Some(
                            reader
                                .get_tag_u8_vec(tiff::tags::Tag::IccProfile)
                                .map_err(|e| e.to_string())?,
                        )
                    } else {
                        None
                    }
                }
                Format::Webp => image_webp::WebPDecoder::new(std::io::Cursor::new(&bytes))
                    .map_err(|e| e.to_string())?
                    .icc_profile()
                    .map_err(|e| e.to_string())?,
                _ => {
                    use image::ImageDecoder;
                    image::ImageReader::new(std::io::Cursor::new(bytes))
                        .with_guessed_format()
                        .map_err(|e| e.to_string())?
                        .into_decoder()
                        .map_err(|e| e.to_string())?
                        .icc_profile()
                        .map_err(|e| e.to_string())?
                }
            };
            check(
                profile.is_some() == include && profile.as_ref().is_none_or(|b| b == &p3_bytes),
                "ICC inclusion/omission or profile bytes mismatch",
            )?;
        }
    }
    let missing = Settings {
        profile: Some(dir.join("missing.icc")),
        ..Default::default()
    };
    check(
        raster_io::prepare_managed(&d, B, &c, &missing, o).is_err(),
        "Missing output ICC accepted",
    )?;
    let disabled = Settings {
        convert: false,
        embed: false,
        ..missing.clone()
    };
    check(
        raster_io::prepare_managed(&d, B, &c, &disabled, o)?.pixels == pixels,
        "Disabled missing profile was used",
    )?;
    let mut cmyk = p3_bytes.clone();
    cmyk[16..20].copy_from_slice(b"CMYK");
    let path = dir.join("cmyk-header.icc");
    fs::write(&path, cmyk).map_err(|e| e.to_string())?;
    let bad = Settings {
        profile: Some(path),
        ..Default::default()
    };
    let error = raster_io::prepare_managed(&d, B, &c, &bad, o)
        .err()
        .ok_or("CMYK RGB adapter accepted")?;
    check(
        error.contains("CMYK") && error.contains("unsupported"),
        "CMYK output guidance absent",
    )?;
    let rgb = RgbProfile::embedded("P3".into(), p3_bytes.clone())?;
    let mut wide = d.clone();
    wide.color.profile = rgb;
    let policy = Settings {
        profile: Some(own),
        ..Default::default()
    };
    let p = raster_io::prepare_managed(&wide, B, &c, &policy, o)?;
    check(
        p.losses(o).iter().any(|n| n.contains("clipped")),
        "Out-of-gamut samples not reported",
    )?;
    let same = raster_io::prepare_managed(
        &wide,
        B,
        &c,
        &Settings {
            profile: Some(p3_path.clone()),
            ..Default::default()
        },
        o,
    )?;
    check(
        same.pixels == pixels,
        "Same embedded profile changed channels",
    )?;
    let mut linear = moxcms::ColorProfile::new_srgb();
    linear.red_colorant = moxcms::Xyzd {
        x: 1.,
        y: 0.,
        z: 0.,
    };
    linear.green_colorant = moxcms::Xyzd {
        x: 0.,
        y: 1.,
        z: 0.,
    };
    linear.blue_colorant = moxcms::Xyzd {
        x: 0.,
        y: 0.,
        z: 1.,
    };
    let trc = moxcms::ToneReprCurve::Parametric(vec![1.]);
    linear.red_trc = Some(trc.clone());
    linear.green_trc = Some(trc.clone());
    linear.blue_trc = Some(trc);
    linear.cicp = None;
    linear.chromatic_adaptation = None;
    linear.media_white_point = Some(moxcms::Xyzd {
        x: 0.5,
        y: 1.,
        z: 0.75,
    });
    let source = RgbProfile::embedded(
        "Linear source".into(),
        linear.encode().map_err(|e| e.to_string())?,
    )?;
    linear.media_white_point = Some(moxcms::Xyzd {
        x: 0.25,
        y: 0.5,
        z: 1.,
    });
    let target = RgbProfile::embedded(
        "Linear target".into(),
        linear.encode().map_err(|e| e.to_string())?,
    )?;
    let patch = Sample::new([0.1, 0.2, 0.3, 0.5])?;
    let relative = Transform::with_intent(&source, &target, Intent::Relative)?.map(patch)?;
    let absolute = Transform::with_intent(&source, &target, Intent::Absolute)?.map(patch)?;
    check(
        relative
            .rgb()
            .into_iter()
            .zip([0.1, 0.2, 0.3])
            .all(|(a, b)| (a - b).abs() < 1e-6)
            && absolute
                .rgb()
                .into_iter()
                .zip([0.2, 0.4, 0.225])
                .all(|(a, b)| (a - b).abs() < 1e-6)
            && absolute.alpha() == 0.5,
        "ICC D.6 PCS media-white scaling/reference failed",
    )?;
    let photo = RgbProfile::embedded(
        "D50 ProPhoto".into(),
        moxcms::ColorProfile::new_pro_photo_rgb()
            .encode()
            .map_err(|e| e.to_string())?,
    )?;
    let relative = Transform::with_intent(&RgbProfile::Srgb, &photo, Intent::Relative)?
        .map(Sample::from16([65535; 4]))?;
    let absolute = Transform::with_intent(&RgbProfile::Srgb, &photo, Intent::Absolute)?
        .map(Sample::from16([65535; 4]))?;
    check(
        relative
            .rgb()
            .into_iter()
            .zip(absolute.rgb())
            .any(|(a, b)| (a - b).abs() > 1e-5),
        "White-point intents were not distinguished",
    )?;
    check(
        fs::read(p3_path).map_err(|e| e.to_string())? == p3_bytes
            && d.snapshot() == before
            && identity == (d.path.clone(), d.dirty, d.history_counts()),
        "ICC output changed profile source/document",
    )?;
    crate::asset_catalog::cleanup_verification(root, &dir)
}
#[cfg(test)]
mod tests {
    #[test]
    fn rgb_embed_conversion_four_intents_whitepoint_gamut_and_unchanged_source() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
