use super::*;
use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::png_io_verify::fixture()?;
    d.layers[0].pixels = vec![[200, 30, 80, 255], [20, 120, 180, 128], [0; 4]];
    Ok(d)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("raster-codecs");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        roundtrips(&dir)?;
        guards(&dir)
    })();
    result.and(fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
pub fn roundtrips(root: &Path) -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let history = d.history_counts();
    let p = prepare(&d, B, &AtomicBool::new(false))?;
    for format in Format::ALL {
        for alpha in [false, true] {
            if format == Format::Jpeg && alpha {
                continue;
            }
            let o = Options {
                format,
                alpha,
                ..Default::default()
            };
            let path = root.join(format!(
                "{}-{alpha}.{}",
                format.extension(),
                format.extension()
            ));
            p.write(&d, &path, o, true, false, B, &AtomicBool::new(false))?;
            let source = crate::image_import::ImageSource::read(&path, B, &AtomicBool::new(false))
                .map_err(|e| e.to_string())?;
            let page = source
                .decode(0, true, B, &AtomicBool::new(false))
                .map_err(|e| e.to_string())?;
            if (page.image.width, page.image.height) != (3, 1) {
                return Err("Codec roundtrip dimensions changed".into());
            }
            if format != Format::Jpeg {
                for (n, c) in page.image.pixels.iter().enumerate() {
                    let want = p.sample(n, o)?.rgba8();

                    for i in 0..4 {
                        let (a, b) = if i == 3 {
                            (c[i] as f64, want[i] as f64)
                        } else {
                            (
                                c[i] as f64 * c[3] as f64 / 255.,
                                want[i] as f64 * want[3] as f64 / 255.,
                            )
                        };
                        if (a - b).abs() > 1.1 {
                            return Err(format!(
                                "{format:?} alpha={alpha} channel mismatch: {c:?} expected {want:?}"
                            ));
                        }
                    }
                }
            }
        }
    }
    for compression in [
        Compression::None,
        Compression::Lzw,
        Compression::Deflate,
        Compression::Packbits,
    ] {
        let o = Options {
            format: Format::Tiff,
            alpha: true,
            depth: Depth::Sixteen,
            compression,
            ..Default::default()
        };
        let path = root.join(format!("tiff-{compression:?}.tif"));
        p.write(&d, &path, o, true, false, B, &AtomicBool::new(false))?;
        let imported = crate::image_import::ImageSource::read(&path, B, &AtomicBool::new(false))
            .map_err(|e| e.to_string())?
            .decode(0, true, B, &AtomicBool::new(false))
            .map_err(|e| e.to_string())?;
        if imported.bits != 16 || imported.color.is_none() {
            return Err("TIFF sixteen-bit path quantized to8 bits".into());
        }
    }
    for rle in [false, true] {
        let o = Options {
            format: Format::Tga,
            alpha: true,
            rle,
            ..Default::default()
        };
        let bytes = p.encode(o, B, &AtomicBool::new(false))?;
        if bytes[2] != if rle { 10 } else { 2 } {
            return Err("TGA compression option ignored".into());
        }
    }
    let mut d16 = Document::create_with_color(
        crate::document::CanvasSpec {
            width: 1,
            height: 1,
            dpi: 72.,
            background: crate::document::Background::Transparent,
        },
        B,
        crate::color_storage::Config {
            depth: Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let high = [1234, 34567, 65000, 32768];
    d16.layers[0].precise = Some(vec![high]);
    d16.layers[0].pixels = vec![Sample::from16(high).rgba8()];
    let precise = prepare(&d16, B, &AtomicBool::new(false))?;
    let path = root.join("precise.tif");
    precise.write(
        &d16,
        &path,
        Options {
            format: Format::Tiff,
            depth: Depth::Sixteen,
            alpha: true,
            ..Default::default()
        },
        true,
        false,
        B,
        &AtomicBool::new(false),
    )?;
    let page = crate::image_import::ImageSource::read(&path, B, &AtomicBool::new(false))
        .map_err(|e| e.to_string())?
        .decode(0, true, B, &AtomicBool::new(false))
        .map_err(|e| e.to_string())?;
    if !page.color.as_ref().unwrap().pixels[0]
        .iter()
        .any(|v| !v.is_multiple_of(257))
    {
        return Err("TIFF high precision samples lost detail".into());
    }
    let reopened = page.document(B).map_err(|e| e.to_string())?;
    if reopened.layers[0].precise.is_none() {
        return Err("TIFF document lost precise storage".into());
    }
    if d.snapshot() != before || d.history_counts() != history {
        return Err("Still export modified source".into());
    }
    Ok(())
}
pub fn guards(root: &Path) -> Result<(), String> {
    let mut d = fixture()?;
    let p = prepare(&d, B, &AtomicBool::new(false))?;
    let o = Options::default();
    let path = root.join("guard.jpg");
    if p.write(&d, &path, o, false, false, B, &AtomicBool::new(false))
        .is_ok()
        || path.exists()
    {
        return Err("Unconfirmed output published".into());
    }
    fs::write(&path, b"original").map_err(|e| e.to_string())?;
    if p.write(&d, &path, o, true, false, B, &AtomicBool::new(false))
        .is_ok()
        || fs::read(&path).unwrap() != b"original"
    {
        return Err("Output replacement guard failed".into());
    }
    if p.write(&d, &path, o, true, true, B, &AtomicBool::new(true))
        .is_ok()
        || fs::read(&path).unwrap() != b"original"
    {
        return Err("Canceled output overwrote file".into());
    }
    let temp = root.join("guard.jpg.raster-write.tmp");
    fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    if p.write(&d, &path, o, true, true, B, &AtomicBool::new(false))
        .is_ok()
        || fs::read(&temp).unwrap() != b"foreign"
    {
        return Err("Foreign temp overwritten".into());
    }
    fs::remove_file(temp).map_err(|e| e.to_string())?;
    d.source_path = Some(path.clone());
    if p.write(&d, &path, o, true, true, B, &AtomicBool::new(false))
        .is_ok()
    {
        return Err("Source overwrite allowed".into());
    }
    d.source_path = None;
    d.current_frame += 1;
    if p.write(&d, &path, o, true, true, B, &AtomicBool::new(false))
        .is_ok()
    {
        return Err("Stale export accepted".into());
    }
    if p.encode(
        Options {
            format: Format::Jpeg,
            alpha: true,
            ..o
        },
        B,
        &AtomicBool::new(false),
    )
    .is_ok()
    {
        return Err("Unsupported JPEG alpha accepted".into());
    }
    if validate_tga(&[0; 18]).is_ok() {
        return Err("Invalid TGA accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn verified_formats_alpha_compression_and_tiff_sixteen_bit() {
        let t = crate::test_support::TempDir::new();
        super::roundtrips(&t.0).unwrap();
    }
    #[test]
    fn lossy_options_cancel_confirm_source_stale_and_foreign_temp() {
        let t = crate::test_support::TempDir::new();
        super::guards(&t.0).unwrap();
    }
}
