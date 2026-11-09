use crate::{
    color_precision::Depth,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    image_import::ImageSource,
    png_export::{self, Options},
};
use image::ImageEncoder;
use std::{path::Path, sync::atomic::AtomicBool};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels = vec![[0, 0, 0, 0], [200, 100, 50, 128], [30, 180, 90, 255]];
    Ok(d)
}
pub fn png16(path: &Path) -> Result<Vec<[u16; 4]>, String> {
    let p = vec![
        [0, 0, 0, 0],
        [1234, 34567, 65000, 32768],
        [65535, 12345, 40000, 65535],
    ];
    let bytes = p
        .iter()
        .flatten()
        .flat_map(|v| u16::to_ne_bytes(*v))
        .collect::<Vec<_>>();
    image::codecs::png::PngEncoder::new(std::fs::File::create(path).map_err(|e| e.to_string())?)
        .write_image(&bytes, 3, 1, image::ExtendedColorType::Rgba16)
        .map_err(|e| e.to_string())?;
    Ok(p)
}
pub fn output(root: &Path) -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let history = d.history_counts();
    for depth in [Depth::Eight, Depth::Sixteen] {
        for transparent in [false, true] {
            let p = root.join(format!("{depth:?}-{transparent}.png"));
            png_export::export_with(
                &d,
                &p,
                B,
                Options {
                    depth,
                    transparent,
                    background: [40, 60, 80],
                },
            )
            .map_err(|e| e.to_string())?;
            let image = image::open(&p).map_err(|e| e.to_string())?;
            check(
                matches!(
                    (depth, image.color()),
                    (Depth::Eight, image::ColorType::Rgba8)
                        | (Depth::Sixteen, image::ColorType::Rgba16)
                ),
                "PNG bit depth / layout differs",
            )?;
            let rgba = image.to_rgba16();
            let samples = rgba.as_raw().as_chunks::<4>().0;
            if transparent {
                check(
                    samples[0][3] == 0 && samples[1][3] == 128 * 257 && samples[2][3] == 65535,
                    "PNG alpha0 / middle / full differs",
                )?;
            } else {
                check(
                    samples.iter().all(|p| p[3] == 65535)
                        && samples[0][..3] == [40 * 257, 60 * 257, 80 * 257],
                    "Opaque PNG matte / alpha differs",
                )?;
            }
            if transparent && depth == Depth::Sixteen {
                let page = ImageSource::read(&p, B, &AtomicBool::new(false))
                    .map_err(|e| e.to_string())?
                    .decode(0, false, B, &AtomicBool::new(false))
                    .map_err(|e| e.to_string())?;
                check(
                    page.image.pixels == d.layers[0].pixels && page.color.is_some(),
                    "8 to16 to8 PNG round trip differs",
                )?;
            }
        }
    }
    check(
        d.snapshot() == before && d.history_counts() == history && d.path.is_none(),
        "PNG export changed drawing/history/path",
    )?;
    let p = root.join("memory.png");
    std::fs::write(&p, b"keep old bytes").map_err(|e| e.to_string())?;
    check(
        png_export::export_with(
            &d,
            &p,
            1,
            Options {
                depth: Depth::Sixteen,
                ..Default::default()
            },
        )
        .is_err()
            && std::fs::read(&p).map_err(|e| e.to_string())? == b"keep old bytes",
        "PNG memory error changed existing output",
    )?;
    Ok(())
}
pub fn precision(root: &Path) -> Result<(), String> {
    let p = root.join("source16.png");
    let original = png16(&p)?;
    let source = std::fs::read(&p).map_err(|e| e.to_string())?;
    let page = ImageSource::read(&p, B, &AtomicBool::new(false))
        .map_err(|e| e.to_string())?
        .decode(0, true, B, &AtomicBool::new(false))
        .map_err(|e| e.to_string())?;
    check(
        page.bits == 16 && page.color.as_ref().unwrap().pixels == original,
        "PNG sixteen-bit source detail reduced",
    )?;
    let mut d = page.document(B).map_err(|e| e.to_string())?;
    check(
        d.color.depth == Depth::Sixteen && d.layers[0].precise.as_ref() == Some(&original),
        "New PNG document did not retain sixteen-bit buffer",
    )?;
    let (opened, report) = crate::files::open_report(&p, B).map_err(|e| e.to_string())?;
    check(
        report.is_empty() && opened.layers[0].precise.as_ref() == Some(&original),
        "File Open PNG precision differs",
    )?;
    let snap = d.snapshot();
    let out = root.join("precision.png");
    png_export::export_with(
        &d,
        &out,
        B,
        Options {
            depth: Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    check(
        image::open(out)
            .map_err(|e| e.to_string())?
            .to_rgba16()
            .as_raw()
            .as_chunks::<4>()
            .0
            == original,
        "16-bit export quantized through8 bit",
    )?;
    let file = root.join("precision.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == snap,
        "Native lost PNG high bits",
    )?;
    check(
        std::fs::read(p).map_err(|e| e.to_string())? == source,
        "PNG import changed source",
    )?;
    Ok(())
}
pub fn malformed(root: &Path) -> Result<(), String> {
    let p = root.join("valid.png");
    png_export::export(&fixture()?, &p, B).map_err(|e| e.to_string())?;
    let raw = std::fs::read(&p).map_err(|e| e.to_string())?;
    let test = |data: Vec<u8>| -> Result<(), String> {
        std::fs::write(&p, data).map_err(|e| e.to_string())?;
        check(
            ImageSource::read(&p, B, &AtomicBool::new(false)).is_err()
                && crate::files::open(&p, B).is_err(),
            "Malformed PNG silently accepted",
        )
    };
    let mut b = raw.clone();
    b[29] ^= 1;
    test(b)?;
    let mut b = raw.clone();
    b.pop();
    test(b)?;
    let mut b = raw.clone();
    b.extend(b"trailing");
    test(b)?;
    let mut b = raw.clone();
    b[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    let crc = crc32fast::hash(&b[12..29]);
    b[29..33].copy_from_slice(&crc.to_be_bytes());
    test(b)?;
    let mut b = raw.clone();
    b[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    test(b)?;
    // A corrupt ancillary chunk must fail too, including when a codec ignores it.
    let mut b = raw.clone();
    let mut extra = 3u32.to_be_bytes().to_vec();
    extra.extend(b"tEXtbad");
    extra.extend(0u32.to_be_bytes());
    b.splice(33..33, extra);
    test(b)?;
    std::fs::write(&p, raw).map_err(|e| e.to_string())?;
    check(
        ImageSource::read(&p, B, &AtomicBool::new(true)).is_err(),
        "Canceled PNG import proceeded",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let d = root.join("png-io");
    std::fs::create_dir(&d).map_err(|e| e.to_string())?;
    let result = (|| {
        output(&d)?;
        precision(&d)?;
        malformed(&d)
    })();
    result.and(std::fs::remove_dir_all(d).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn eight_sixteen_transparent_opaque_round_trip_and_source_unchanged() {
        let t = crate::test_support::TempDir::new();
        super::output(&t.0).unwrap();
    }
    #[test]
    fn sixteen_bit_source_open_export_and_native_preserve_low_bits() {
        let t = crate::test_support::TempDir::new();
        super::precision(&t.0).unwrap();
    }
    #[test]
    fn critical_ancillary_crc_truncated_huge_trailing_and_cancel() {
        let t = crate::test_support::TempDir::new();
        super::malformed(&t.0).unwrap();
    }
}
