use super::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    raster_io::{self, Format, Options},
};
use std::{fs, path::Path};
pub fn header(b: &[u8]) -> Result<(u8, [Vec<u8>; 2]), String> {
    if b.get(..2) != Some(&[255, 216]) {
        return Err("JPEG SOI".into());
    }
    let mut at = 2;
    let mut sampling = None;
    let mut tables = [vec![], vec![]];
    while at + 4 <= b.len() {
        if b[at] != 255 {
            return Err("JPEG marker".into());
        }
        let marker = b[at + 1];
        if marker == 218 {
            break;
        }
        let len = u16::from_be_bytes([b[at + 2], b[at + 3]]) as usize;
        if len < 2 || at + 2 + len > b.len() {
            return Err("JPEG segment length".into());
        }
        let data = &b[at + 4..at + 2 + len];
        if marker == 192 {
            if data.len() != 15 || data[0] != 8 || data[5] != 3 || data[10] != 17 || data[13] != 17
            {
                return Err("JPEG baseline/component header".into());
            }
            sampling = Some(data[7]);
        }
        if marker == 219 {
            let mut p = 0;
            while p < data.len() {
                if p + 65 > data.len() || data[p] > 1 {
                    return Err("JPEG DQT precision/id".into());
                }
                tables[data[p] as usize] = data[p + 1..p + 65].to_vec();
                p += 65;
            }
        }
        at += 2 + len;
    }
    if tables.iter().any(|t| t.len() != 64) {
        return Err("Missing JPEG quantization tables".into());
    }
    Ok((sampling.ok_or("Missing baseline frame")?, tables))
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 17,
            height: 9,
            dpi: 299.75,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    for y in 0..9 {
        for x in 0..17 {
            d.layers[0].pixels[y * 17 + x] = if x % 4 == 0 || y == 2 || y == 6 {
                [240, 20, 60, 255]
            } else if (x + y) % 2 == 0 {
                [20, 180, 230, 128]
            } else {
                [60, 220, 30, 255]
            };
        }
    }
    Ok(d)
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("jpeg-output-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let d = fixture()?;
        let before = d.snapshot();
        let id = (d.path.clone(), d.dirty, d.history_counts());
        let p = raster_io::prepare(&d, B, &c)?;
        for (sampling, hv) in [
            (Sampling::S444, 17),
            (Sampling::S422, 33),
            (Sampling::S420, 34),
        ] {
            let options = |luma, chroma| Options {
                format: Format::Jpeg,
                quality: luma,
                chroma_quality: chroma,
                sampling,
                ..Default::default()
            };
            let mut all = vec![];
            for (luma, chroma) in [(1, 1), (100, 100), (27, 83), (27, 4), (92, 83)] {
                let o = options(luma, chroma);
                let encoded = p.encoded_preview(o, B, &c)?;
                let (actual, tables) = header(encoded.bytes())?;
                if actual != hv {
                    return Err("Selected JPEG sampling missing from actual frame".into());
                }
                if luma == 1 && tables.iter().flatten().any(|n| *n != 255)
                    || luma == 100 && tables.iter().flatten().any(|n| *n != 1)
                {
                    return Err("JPEG quality endpoints do not control actual DQT".into());
                }
                let raw =
                    image::load_from_memory_with_format(encoded.bytes(), image::ImageFormat::Jpeg)
                        .map_err(|e| e.to_string())?
                        .into_rgba8();
                if raw.dimensions() != (17, 9)
                    || encoded.size != [17, 9]
                    || raw.pixels().map(|p| p.0).collect::<Vec<_>>() != encoded.pixels
                {
                    return Err("Compression preview differs from independent JPEG decoder".into());
                }
                if p.encode(o, B, &c)? != encoded.bytes() {
                    return Err("JPEG encoding not deterministic".into());
                }
                let path = dir.join(format!(
                    "{}-{luma}-{chroma}.jpg",
                    sampling.label().replace(':', "")
                ));
                p.write_encoded(&d, &path, &encoded, true, false, B, &c)?;
                if fs::read(&path).map_err(|e| e.to_string())? != encoded.bytes() {
                    return Err("Saved file differs from reviewed compressed bytes".into());
                }
                all.push(tables);
            }
            if all[2][0] != all[3][0]
                || all[2][1] == all[3][1]
                || all[2][0] == all[4][0]
                || all[2][1] != all[4][1]
            {
                return Err("Luma/chroma quality controls are coupled".into());
            }
        }
        let mut linear = moxcms::ColorProfile::new_srgb();
        let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
        linear.red_trc = Some(curve.clone());
        linear.green_trc = Some(curve.clone());
        linear.blue_trc = Some(curve);
        let profile = linear.encode().map_err(|e| e.to_string())?;
        let path = dir.join("linear.icc");
        fs::write(&path, &profile).map_err(|e| e.to_string())?;
        let o = Options::default();
        let managed = raster_io::prepare_managed(
            &d,
            B,
            &c,
            &crate::export_color::Settings {
                profile: Some(path),
                ..Default::default()
            },
            o,
        )?;
        let encoded = managed.encoded_preview(o, B, &c)?;
        let decoded =
            image::load_from_memory_with_format(encoded.bytes(), image::ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?
                .into_rgba8();
        let transform = crate::color_profile::Transform::new(
            &crate::color_profile::RgbProfile::embedded("Linear".into(), profile)?,
            &crate::color_profile::RgbProfile::Srgb,
        )?;
        for (pixel, preview) in decoded.pixels().zip(&encoded.pixels) {
            if transform
                .map(crate::color_precision::Sample::from8(pixel.0))?
                .rgba8()
                != *preview
            {
                return Err("JPEG actual readback did not apply output RGB proof transform".into());
            }
        }
        let mut matte = fixture()?;
        matte.layers[0].pixels.fill([0; 4]);
        let p = raster_io::prepare(&matte, B, &c)?;
        let encoded = p.encoded_preview(
            Options {
                quality: 100,
                chroma_quality: 100,
                background: [20, 90, 140],
                ..Default::default()
            },
            B,
            &c,
        )?;
        if encoded.pixels.iter().any(|p| {
            p[3] != 255
                || p[..3]
                    .iter()
                    .zip([20u8, 90, 140])
                    .any(|(a, b)| a.abs_diff(b) > 3)
        }) {
            return Err("JPEG alpha matte not composited before compression".into());
        }
        if p.encoded_preview(Options::default(), B, &AtomicBool::new(true))
            .is_ok()
            || p.encoded_preview(
                Options {
                    format: Format::Png,
                    ..Default::default()
                },
                B,
                &c,
            )
            .is_ok()
        {
            return Err("JPEG preview cancel/format guard failed".into());
        }
        if d.snapshot() != before || id != (d.path.clone(), d.dirty, d.history_counts()) {
            return Err("JPEG preview/write changed original".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_tables_sampling_actual_decode_matte_and_cached_publication() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
