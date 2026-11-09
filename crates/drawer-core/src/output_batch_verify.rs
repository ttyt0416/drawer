use super::*;
use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("output-batch-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let c = AtomicBool::new(false);
    let mut d = Document::create(
        CanvasSpec {
            width: 2,
            height: 2,
            dpi: 299.75,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.title = "한글 🎨.drawer".into();
    d.layers[0].pixels = vec![
        [255, 0, 0, 255],
        [0, 0, 255, 0],
        [20, 80, 180, 128],
        [10, 20, 30, 255],
    ];
    let before = d.snapshot();
    let identity = (d.path.clone(), d.title.clone(), d.dirty, d.history_counts());
    let mut second = d.clone();
    second.id += 10000;
    second.title = "다음 문서".into();
    let docs = vec![d.clone(), second];
    let mut s = Settings {
        geometry: Geometry {
            resize: true,
            width: 4,
            height: 3,
            resolution: true,
            dpi: 72.5,
        },
        ..Default::default()
    };
    let prepared = crate::raster_io::prepare(&d, B, &c)?.resized(2, 2, 72.5, B, &c)?;
    for format in [
        crate::raster_io::Format::Png,
        crate::raster_io::Format::Jpeg,
        crate::raster_io::Format::Tiff,
    ] {
        let o = crate::raster_io::Options {
            format,
            alpha: false,
            ..Default::default()
        };
        let bytes = prepared.encode(o, B, &c)?;
        if format == crate::raster_io::Format::Jpeg {
            let at = bytes
                .windows(5)
                .position(|s| s == b"JFIF\0")
                .ok_or("Missing JFIF density")?;
            check(
                bytes[at + 7] == 1
                    && u16::from_be_bytes(bytes[at + 8..at + 10].try_into().unwrap()) == 73,
                "JPEG rounded density",
            )?;
        }
        if format == crate::raster_io::Format::Tiff {
            let mut reader = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes))
                .map_err(|e| e.to_string())?;
            let rational = reader
                .get_tag_u32_vec(tiff::tags::Tag::XResolution)
                .map_err(|e| e.to_string())?;
            check(rational == [725000, 10000], "TIFF rational density")?;
        }
    }
    let exact = crate::raster_io::prepare(&d, B, &c)?;
    let o = crate::raster_io::Options {
        format: crate::raster_io::Format::Png,
        depth: crate::color_precision::Depth::Sixteen,
        alpha: true,
        ..Default::default()
    };
    let bytes = exact.encode(o, B, &c)?;
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(|e| e.to_string())?;
    check(
        reader.info().bit_depth == png::BitDepth::Sixteen && reader.info().icc_profile.is_some(),
        "PNG16/profile missing",
    )?;
    let mut raw = vec![0; reader.output_buffer_size().ok_or("PNG size")?];
    reader.next_frame(&mut raw).map_err(|e| e.to_string())?;
    let expected = exact
        .pixels
        .iter()
        .flat_map(|p| p.iter().flat_map(|v| v.to_be_bytes()))
        .collect::<Vec<_>>();
    check(
        raw == expected,
        "PNG16 hidden RGB/alpha changed without resizing",
    )?;
    let p = prepare(&docs, &dir, &s, B, &c)?;
    check(
        p.matches(&docs) && p.files.len() == 2,
        "Output source guard",
    )?;
    for f in &p.files {
        let decoder = png::Decoder::new(std::io::Cursor::new(&f.bytes));
        let r = decoder.read_info().map_err(|e| e.to_string())?;
        let i = r.info();
        check(
            i.width == 4
                && i.height == 3
                && i.pixel_dims.is_some_and(|m| {
                    m.unit == png::Unit::Meter && m.xppu == (72.5f64 / 0.0254).round() as u32
                }),
            "PNG dimensions/DPI metadata",
        )?;
    }
    let first = p.files[0].path.clone();
    fs::write(&first, b"original-readonly").map_err(|e| e.to_string())?;
    let mut perms = fs::metadata(&first)
        .map_err(|e| e.to_string())?
        .permissions();
    let saved = perms.clone();
    perms.set_readonly(true);
    fs::set_permissions(&first, perms).map_err(|e| e.to_string())?;
    let report = p.publish(&c);
    fs::set_permissions(&first, saved).map_err(|e| e.to_string())?;
    check(
        report.files[0].result.is_err()
            && report.files[1].result.is_ok()
            && fs::read(&first).map_err(|e| e.to_string())? == b"original-readonly",
        "Per-file failure/read-only preservation",
    )?;
    for pattern in [
        "COM¹.txt",
        "LPT²",
        "CON .png",
        "CON",
        "LPT1.txt",
        "AUX",
        "bad/name",
        "bad:name",
        "a*",
        "../escape",
        "a.",
        "{unknown}",
        " ",
    ] {
        check(
            filename(pattern, "valid", 1, "png").is_err(),
            "Reserved filename accepted",
        )?;
    }
    s.pattern = "same".into();
    check(
        prepare(&docs, &dir, &s, B, &c).is_err(),
        "Duplicate output names accepted",
    )?;
    s.pattern = "{title}-{index}".into();
    for (w, h) in [(0, 1), (1, 0), (32768, 32768)] {
        s.geometry.width = w;
        s.geometry.height = h;
        check(
            prepare(&docs, &dir, &s, B, &c).is_err(),
            "Bad output size accepted",
        )?;
    }
    s.geometry.width = 4;
    s.geometry.height = 3;
    s.geometry.dpi = 0.;
    check(
        prepare(&docs, &dir, &s, B, &c).is_err(),
        "Zero DPI accepted",
    )?;
    s.geometry.resolution = false;
    s.geometry.resize = false;
    s.geometry.width = 0;
    s.geometry.height = 0;
    let p = prepare(&docs[..1], &dir, &s, B, &c)?;
    check(
        p.files[0].width == 2 && p.files[0].dpi == 299.75,
        "Disabled geometry values applied",
    )?;
    let mut changed = docs.clone();
    changed[0].edit_pixel(0, 1, 1, [1, 2, 3, 255]);
    check(!p.matches(&changed), "Stale source accepted")?;
    c.store(true, std::sync::atomic::Ordering::Relaxed);
    check(
        p.publish(&c).files.iter().all(|f| f.result.is_err()),
        "Canceled batch wrote files",
    )?;
    check(
        d.snapshot() == before
            && identity == (d.path.clone(), d.title.clone(), d.dirty, d.history_counts()),
        "Output changed source",
    )?;
    assets::cleanup_verification(root, &dir)
}
#[cfg(test)]
mod tests {
    #[test]
    fn names_pixels_dpi_readonly_per_file_failures_cancel_and_disabled_values() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
