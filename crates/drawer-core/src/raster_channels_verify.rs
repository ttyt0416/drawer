use super::*;
use crate::{
    color_precision::{Depth, Sample},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    raster_io::{self, Format, Options},
};
use std::{fs, io::Cursor, path::Path};
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("raster-channels-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 299.75,
                background: Background::Transparent,
            },
            B,
            crate::color_storage::Config {
                depth: Depth::Sixteen,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let high = vec![
            [1234, 1234, 1234, 65535],
            [65000, 20000, 45000, 32768],
            [0; 4],
            [17000, 50000, 2000, 65535],
            [4097, 4097, 4097, 20000],
            [50000, 50000, 50000, 65535],
        ];
        d.layers[0].pixels = high.iter().map(|v| Sample::from16(*v).rgba8()).collect();
        d.layers[0].precise = Some(high);
        let original = d.snapshot();
        let identity = (
            d.history_counts(),
            d.path.clone(),
            d.source_path.clone(),
            d.dirty,
            d.revision,
        );
        let p = raster_io::prepare(&d, B, &c)?;
        for dpi in [72.5, 299.75] {
            let p = p.resized(3, 2, dpi, B, &c)?;
            for grayscale in [false, true] {
                for alpha in [false, true] {
                    for depth in [Depth::Eight, Depth::Sixteen] {
                        for embed in [false, true] {
                            let o = Options {
                                format: Format::Png,
                                grayscale,
                                alpha,
                                depth,
                                ..Default::default()
                            };
                            let mut prepared = p.clone();
                            if !embed {
                                prepared.icc = None;
                            }
                            let bytes = prepared.encode(o, B, &c)?;
                            let mut decoder = png::Decoder::new(Cursor::new(&bytes))
                                .read_info()
                                .map_err(|e| e.to_string())?;
                            let expected = match (grayscale, alpha) {
                                (true, true) => png::ColorType::GrayscaleAlpha,
                                (true, false) => png::ColorType::Grayscale,
                                (false, true) => png::ColorType::Rgba,
                                (false, false) => png::ColorType::Rgb,
                            };
                            if decoder.info().color_type != expected
                                || decoder.info().bit_depth
                                    != if depth == Depth::Sixteen {
                                        png::BitDepth::Sixteen
                                    } else {
                                        png::BitDepth::Eight
                                    }
                                || decoder.info().icc_profile.is_some() != embed
                            {
                                return Err("PNG channel/depth/ICC header mismatch".into());
                            }
                            if grayscale
                                && embed
                                && moxcms::ColorProfile::new_from_slice(
                                    decoder.info().icc_profile.as_ref().unwrap(),
                                )
                                .map_err(|e| e.to_string())?
                                .color_space
                                    != moxcms::DataColorSpace::Gray
                            {
                                return Err("Gray PNG contains non-Gray ICC".into());
                            }
                            let dims = decoder.info().pixel_dims.ok_or("Missing PNG resolution")?;
                            if dims.xppu != ppm(dpi)? as u32
                                || dims.yppu != dims.xppu
                                || dims.unit != png::Unit::Meter
                            {
                                return Err("PNG pixels-per-meter conversion".into());
                            }
                            let mut decoded = vec![0; decoder.output_buffer_size().unwrap()];
                            let info = decoder
                                .next_frame(&mut decoded)
                                .map_err(|e| e.to_string())?;
                            let stride = info.color_type.samples()
                                * if depth == Depth::Sixteen { 2 } else { 1 };
                            for (n, raw) in decoded[..info.buffer_size()]
                                .chunks_exact(stride)
                                .enumerate()
                            {
                                let want = prepared.sample(n, o)?;
                                let values = if depth == Depth::Sixteen {
                                    raw.as_chunks::<2>()
                                        .0
                                        .iter()
                                        .map(|v| u16::from_be_bytes(*v))
                                        .collect::<Vec<_>>()
                                } else {
                                    raw.iter().map(|v| *v as u16).collect()
                                };
                                let want = if depth == Depth::Sixteen {
                                    want.rgba16()
                                } else {
                                    want.rgba8().map(u16::from)
                                };
                                let expected = if grayscale {
                                    if alpha {
                                        vec![want[0], want[3]]
                                    } else {
                                        vec![want[0]]
                                    }
                                } else {
                                    want[..if alpha { 4 } else { 3 }].to_vec()
                                };
                                if values != expected {
                                    return Err(
                                        "PNG encoded channel/alpha samples differ from preview"
                                            .into(),
                                    );
                                }
                            }
                            let path = dir.join("gray-roundtrip.png");
                            fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                            let imported = crate::image_import::ImageSource::read(&path, B, &c)
                                .and_then(|s| s.decode(0, true, B, &c))
                                .map_err(|e| e.to_string())?;
                            for (n, got) in imported.image.pixels.iter().enumerate() {
                                let want = prepared.preview_sample(n, o)?.rgba8();
                                if *got != want {
                                    return Err(format!(
                                        "PNG Gray/RGB import changed alpha/colors: {got:?} != {want:?}"
                                    ));
                                }
                            }
                            if depth == Depth::Sixteen {
                                let precise = &imported
                                    .color
                                    .as_ref()
                                    .ok_or("Lost16-bit PNG samples")?
                                    .pixels;
                                for (n, got) in precise.iter().enumerate() {
                                    let want = prepared.sample(n, o)?.rgba16();
                                    if *got != want {
                                        return Err(
                                            "GrayAlpha16 import lost low bits or alpha".into()
                                        );
                                    }
                                }
                            }
                            if !prepared
                                .notices(o)
                                .iter()
                                .any(|s| s.contains("approximates"))
                            {
                                return Err("Integer PPM approximation not reported".into());
                            }
                        }
                    }
                }
            }
            for alpha in [false, true] {
                let o = Options {
                    format: Format::Bmp,
                    alpha,
                    background: [12, 80, 240],
                    ..Default::default()
                };
                let bytes = p.encode(o, B, &c)?;
                let u32at = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                if u32at(14) != if alpha { 124 } else { 40 }
                    || u32at(10) != if alpha { 138 } else { 54 }
                    || u32at(38) != ppm(dpi)? as u32
                    || u32at(42) != ppm(dpi)? as u32
                {
                    return Err("BMP INFO/V5/DPI header mismatch".into());
                }
                if alpha
                    && (u32at(14 + 52) != 0xff000000
                        || u32at(30) != 3
                        || u32at(14 + 56) != 0x73524742)
                {
                    return Err("BMP V5 alpha/color masks".into());
                }
                let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp)
                    .map_err(|e| e.to_string())?
                    .into_rgba8();
                for (n, got) in decoded.pixels().enumerate() {
                    if got.0 != p.preview_sample(n, o)?.rgba8() {
                        return Err(
                            "Independent BMP decoder alpha/padded-row orientation mismatch".into(),
                        );
                    }
                }
                let mut invalid = bytes.clone();
                invalid[14..18].copy_from_slice(&13u32.to_le_bytes());
                if image::load_from_memory_with_format(&invalid, image::ImageFormat::Bmp).is_ok() {
                    return Err("Unsupported BMP header accepted".into());
                }
                if image::load_from_memory_with_format(&bytes[..30], image::ImageFormat::Bmp)
                    .is_ok()
                {
                    return Err("Truncated BMP accepted".into());
                }
            }
        }
        let o = Options {
            grayscale: true,
            background: [12, 80, 240],
            quality: 100,
            chroma_quality: 100,
            ..Default::default()
        };
        let bytes = p.encode(o, B, &c)?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?
            .into_rgb8();
        if decoded.pixels().any(|v| v[0] != v[1] || v[1] != v[2]) {
            return Err("Grayscale JPEG did not retain equal RGB channels".into());
        }
        let settings = crate::export_color::Settings {
            profile: Some(dir.join("missing.icc")),
            ..Default::default()
        };
        let before_settings = settings.clone();
        let gray = Options {
            format: Format::Png,
            grayscale: true,
            alpha: true,
            ..Default::default()
        };
        let prepared = raster_io::prepare_managed(&d, B, &c, &settings, gray)?;
        if settings != before_settings
            || raster_io::prepare_managed(
                &d,
                B,
                &c,
                &settings,
                Options {
                    grayscale: false,
                    ..gray
                },
            )
            .is_ok()
        {
            return Err("Inactive RGB ICC was applied or overwritten".into());
        }
        let target = dir.join("existing.png");
        fs::write(&target, b"previous export").map_err(|e| e.to_string())?;
        c.store(true, std::sync::atomic::Ordering::Relaxed);
        if prepared.write(&d, &target, gray, true, true, B, &c).is_ok()
            || fs::read(&target).unwrap() != b"previous export"
        {
            return Err("Canceled channel export replaced existing file".into());
        }
        if d.snapshot() != original
            || identity
                != (
                    d.history_counts(),
                    d.path.clone(),
                    d.source_path.clone(),
                    d.dirty,
                    d.revision,
                )
            || d.dpi != 299.75
        {
            return Err("Channel output changed source precision/profile/DPI/path/history".into());
        }
        Ok(())
    })();
    result.and(crate::asset_catalog::cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn png_gray_alpha_8_16_bmp_info_v5_decimal_dpi_independent_readback() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
