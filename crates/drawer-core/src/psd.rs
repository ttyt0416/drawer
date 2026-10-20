//! Independent raw RGB8 PSD subset from Adobe's public file format specification.
//! Export choices explicitly bake unsupported effects. Import reads the merged preview only.
use crate::{
    document::{Document, RasterLayer},
    render::Renderer,
};
use std::{
    fs,
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Cancel,
    RenderedLayers,
    RenderedImage,
}
fn error(s: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.to_string())
}
fn u16be(w: &mut impl Write, n: u16) -> io::Result<()> {
    w.write_all(&n.to_be_bytes())
}
fn u32be(w: &mut impl Write, n: u32) -> io::Result<()> {
    w.write_all(&n.to_be_bytes())
}
fn r16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
fn r32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
pub fn layers_supported(d: &Document) -> bool {
    (d.scene.include_drafts_in_output || !crate::layer_roles::has_drafts(d))
        && !crate::layer_blend::graph_active(d)
        && d.scene.render_order.is_default()
        && d.animation.is_none()
        && d.scene.gradients.layers.is_empty()
        && d.scene.objects.is_empty()
        && d.scene.lines.layers.is_empty()
        && d.scene.worlds.is_empty()
        && d.scene
            .paper
            .layers
            .values()
            .all(|p| !p.hidden && p.blend == crate::brush_composite::Mode::Normal)
        && d.scene
            .paper
            .folders
            .iter()
            .all(|f| !f.paper.hidden && f.paper.blend == crate::brush_composite::Mode::Normal)
}
pub const LOSS: &str = "PSD stores rendered RGB8 sRGB pixels; 16-bit document precision is reduced only in this output. Paper recipes, file-layer caches and editable folder effects are baked; folders become raster substitutes. Brush history and other Drawer metadata are not included. Layer blend settings are baked in rendered-image export; editable blend-mode metadata is not preserved. Keep the .drawer original. Merged-preview import does not restore PSD layers and uses 300 DPI.";
pub fn export(
    d: &Document,
    path: &Path,
    choice: Choice,
    replace: bool,
    budget: usize,
) -> io::Result<bool> {
    if choice == Choice::Cancel {
        return Ok(false);
    }
    if !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || d.layers.iter().any(|l| !l.valid(d.width, d.height))
        || d.width > 30000
        || d.height > 30000
    {
        return Err(error("Invalid document or PSD dimensions exceed 30000"));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("psd"))
    {
        return Err(error("Choose a .psd destination"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let path = parent.join(path.file_name().ok_or_else(|| error("Missing filename"))?);
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err(error("Choose a regular destination"));
    }
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        path
    };
    if d.path.as_ref() == Some(&path) || d.source_path.as_ref() == Some(&path) {
        return Err(error("Preserve the source; choose another filename"));
    }
    if path.exists() && !replace {
        return Err(error("Existing PSD requires explicit replacement"));
    }
    if choice == Choice::RenderedLayers
        && !d.scene.include_drafts_in_output
        && crate::layer_roles::has_drafts(d)
    {
        return Err(error(
            "Draft-excluding output requires Rendered image; keep the .drawer source or explicitly include drafts",
        ));
    }
    let extra = if !d.scene.include_drafts_in_output && crate::layer_roles::has_drafts(d) {
        d.memory_bytes().saturating_mul(2)
    } else {
        0
    };
    if extra > budget / 2 {
        return Err(error("Draft output snapshot exceeds export memory"));
    }
    let render = Renderer::for_export(d).map_err(error)?;
    let merged = if d.color != crate::color_storage::Config::legacy() {
        let count = d.width as usize * d.height as usize;
        if count.saturating_mul(4) > budget.saturating_sub(extra) / 4 {
            return Err(error("PSD RGB8 output budget"));
        }
        let mut pixels = Vec::with_capacity(count);
        for y in 0..d.height {
            for x in 0..d.width {
                pixels.push(
                    render
                        .display_sample([x as f64 + 0.5, y as f64 + 0.5])
                        .map_err(error)?
                        .rgba8(),
                );
            }
        }
        pixels
    } else {
        render
            .output(d.dpi, budget.saturating_sub(extra) / 4)
            .map_err(error)?
            .2
    };
    let layers = match choice {
        Choice::RenderedLayers => {
            if !layers_supported(d) {
                return Err(error(
                    "Layer blend modes, source raster order, animation, page objects and hidden layers require Rendered image",
                ));
            }
            let mut remaining = budget / 4;
            let mut layers = Vec::new();
            for node in d.scene.paper.root_nodes(d.layers.len()).into_iter().rev() {
                let mut layer = d
                    .scene
                    .paper
                    .render_node(d, node, remaining)
                    .map_err(error)?;
                if d.color != crate::color_storage::Config::legacy() {
                    let transform = crate::color_profile::Transform::new(
                        &d.color.profile,
                        &crate::color_profile::RgbProfile::Srgb,
                    )
                    .map_err(error)?;
                    layer.pixels = (0..layer.pixels.len())
                        .map(|n| transform.map(layer.sample_at(n)).map(|s| s.rgba8()))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(error)?;
                    layer.precise = None;
                }
                remaining = remaining.saturating_sub(layer.pixels.len() * 4);
                layers.push(layer);
            }
            layers
        }
        Choice::RenderedImage => vec![RasterLayer {
            precise: None,
            locked: false,
            name: "Rendered Drawer image".into(),
            pixels: merged.clone(),
            placement: None,
        }],
        Choice::Cancel => unreachable!(),
    };
    if layers.is_empty() || layers.len() > 256 {
        return Err(error("Invalid PSD layer count"));
    }
    let mut records = Vec::new();
    for layer in &layers {
        let e = layer.extent(d.width, d.height);
        for n in [
            e.offset[1],
            e.offset[0],
            e.offset[1] + e.height as i64,
            e.offset[0] + e.width as i64,
        ] {
            records.extend_from_slice(&(n as i32).to_be_bytes());
        }
        u16be(&mut records, 4)?;
        for channel in [0i16, 1, 2, -1] {
            records.extend_from_slice(&channel.to_be_bytes());
            u32be(&mut records, (2 + layer.pixels.len()) as u32)?;
        }
        records.extend_from_slice(b"8BIMnorm\xff\0\0\0");
        let mut extra = Vec::new();
        u32be(&mut extra, 0)?;
        u32be(&mut extra, 0)?;
        extra.extend_from_slice(b"\x05Layer\0\0"); // Pascal fallback, four-byte aligned.
        let name = layer.name.encode_utf16().collect::<Vec<_>>();
        extra.extend_from_slice(b"8BIMluni");
        u32be(&mut extra, (4 + 2 * name.len()) as u32)?;
        u32be(&mut extra, name.len() as u32)?;
        for n in name {
            u16be(&mut extra, n)?;
        }
        u32be(&mut records, extra.len() as u32)?;
        records.extend(extra);
    }
    let info_len = 2
        + records.len()
        + layers
            .iter()
            .map(|l| 4 * (2 + l.pixels.len()))
            .sum::<usize>();
    let padded = info_len + info_len % 2;
    if padded > u32::MAX as usize - 8 {
        return Err(error("PSD section limit"));
    }
    let mut filename = path.file_name().unwrap().to_owned();
    filename.push(".psd-write.tmp");
    let tmp = path.with_file_name(filename);
    let f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)?;
    let mut w = BufWriter::new(f);
    let result = (|| {
        w.write_all(b"8BPS\0\x01\0\0\0\0\0\0")?;
        u16be(&mut w, 4)?;
        u32be(&mut w, d.height)?;
        u32be(&mut w, d.width)?;
        u16be(&mut w, 8)?;
        u16be(&mut w, 3)?;
        u32be(&mut w, 0)?;
        u32be(&mut w, 0)?;
        u32be(&mut w, (4 + padded + 4) as u32)?;
        u32be(&mut w, padded as u32)?;
        w.write_all(&(-(layers.len() as i16)).to_be_bytes())?;
        w.write_all(&records)?;
        for layer in &layers {
            for c in 0..4 {
                u16be(&mut w, 0)?;
                plane(&mut w, &layer.pixels, c)?;
            }
        }
        if info_len % 2 != 0 {
            w.write_all(&[0])?;
        }
        u32be(&mut w, 0)?;
        u16be(&mut w, 0)?;
        for c in 0..4 {
            plane(&mut w, &merged, c)?;
        }
        w.flush()?;
        w.get_ref().sync_all()
    })();
    drop(w);
    let result = result.and_then(|_| {
        if replace {
            fs::rename(&tmp, &path)
        } else {
            fs::hard_link(&tmp, &path).and_then(|_| fs::remove_file(&tmp))
        }
    });
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(true)
}
fn plane(w: &mut impl Write, pixels: &[[u8; 4]], c: usize) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    for chunk in pixels.chunks(buffer.len()) {
        for (n, p) in chunk.iter().enumerate() {
            buffer[n] = p[c];
        }
        w.write_all(&buffer[..chunk.len()])?;
    }
    Ok(())
}
/// Bounded raw RGB8 merged-preview import; other PSD encodings fail explicitly.
pub fn open(path: &Path, budget: usize) -> io::Result<Document> {
    let path = path.canonicalize()?;
    let f = fs::File::open(&path)?;
    let size = f.metadata()?.len();
    let mut r = BufReader::new(f);
    let mut magic = [0; 12];
    r.read_exact(&mut magic)?;
    if &magic != b"8BPS\0\x01\0\0\0\0\0\0" {
        return Err(error("Unsupported PSD header"));
    }
    let channels = r16(&mut r)?;
    let height = r32(&mut r)?;
    let width = r32(&mut r)?;
    let depth = r16(&mut r)?;
    let mode = r16(&mut r)?;
    if channels != 4 || depth != 8 || mode != 3 || width > 30000 || height > 30000 {
        return Err(error("PSD preview import supports raw RGBA8 RGB mode only"));
    }
    for _ in 0..3 {
        let n = r32(&mut r)? as u64;
        let end = r
            .stream_position()?
            .checked_add(n)
            .ok_or_else(|| error("PSD section overflow"))?;
        if end > size {
            return Err(error("Truncated PSD section"));
        }
        r.seek(SeekFrom::Start(end))?;
    }
    if r16(&mut r)? != 0 {
        return Err(error("Compressed PSD preview import is not supported"));
    }
    let count = width as u64 * height as u64;
    if count > budget as u64 / 4 || r.stream_position()?.checked_add(count * 4) != Some(size) {
        return Err(error("PSD preview size or memory limit"));
    }
    let mut d = Document::create(
        crate::document::CanvasSpec {
            width,
            height,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        budget,
    )
    .map_err(error)?;
    let mut buffer = [0; 8192];
    for c in 0..4 {
        for pixels in d.layers[0].pixels.chunks_mut(buffer.len()) {
            r.read_exact(&mut buffer[..pixels.len()])?;
            for (n, p) in pixels.iter_mut().enumerate() {
                p[c] = buffer[n];
            }
        }
    }
    d.layers[0].name = "PSD merged preview".into();
    d.title = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    d.source_path = Some(path);
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        paper::{Node, Paper},
        test_support::TempDir,
        texture::Pattern,
    };
    fn fixture() -> Document {
        let mut d = Document::create(
            CanvasSpec {
                width: 9,
                height: 5,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1024,
        )
        .unwrap();
        for (n, p) in d.layers[0].pixels.iter_mut().enumerate() {
            *p = [220, 40 + n as u8 * 3, 90, 20 + n as u8 * 3];
        }
        d.layers[0].name = "밑그림".into();
        d.scene.paper.layers.insert(
            0,
            Paper {
                pattern: Pattern {
                    source: 1,
                    strength: 0.7,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let n = d.scene.paper.group_all(1, "한지 폴더").unwrap();
        d.scene.paper.folders[n].paper.pattern = Pattern {
            source: 2,
            strength: 0.6,
            ..Default::default()
        };
        d.layers.push(RasterLayer {
            precise: None,
            locked: true,
            name: "Top blue".into(),
            pixels: vec![[30, 90, 220, 70]; 45],
            placement: Some(crate::document::PlacedRaster {
                width: 9,
                height: 5,
                offset: [1, -1],
            }),
        });
        d
    }
    #[test]
    fn rendered_psd_cancel_layers_whole_reopen_and_source_are_unchanged() {
        let root = TempDir::new();
        let path = root.0.join("out.psd");
        let d = fixture();
        let state = d.snapshot();
        let history = d.history_counts();
        let dirty = d.dirty;
        assert!(!export(&d, &path, Choice::Cancel, false, 1).unwrap());
        assert!(!path.exists());
        for choice in [Choice::RenderedLayers, Choice::RenderedImage] {
            export(&d, &path, choice, true, 100000).unwrap();
            let opened = crate::files::open(&path, 100000).unwrap();
            let expected = Renderer::new(&d).output(d.dpi, 100000).unwrap().2;
            assert_eq!(opened.layers[0].pixels, expected);
            assert!(d.snapshot() == state);
            assert_eq!(d.history_counts(), history);
            assert_eq!(d.dirty, dirty);
        }
        let bytes = fs::read(&path).unwrap();
        assert!(export(&d, &path, Choice::RenderedLayers, false, 100000).is_err());
        assert!(export(&d, &path, Choice::RenderedLayers, true, 1).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mut imported = open(&path, 100000).unwrap();
        imported.scene.paper.group_all(1, "New folder").unwrap();
        assert!(export(&imported, &path, Choice::RenderedLayers, true, 100000).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(root.0.join("out.psd.psd-write.tmp"), "foreign temporary").unwrap();
        assert!(export(&d, &path, Choice::RenderedLayers, true, 100000).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read_to_string(root.0.join("out.psd.psd-write.tmp")).unwrap(),
            "foreign temporary"
        );
    }
    #[test]
    fn page_objects_require_explicit_whole_image_and_invalid_psd_is_bounded() {
        let root = TempDir::new();
        let path = root.0.join("out.psd");
        let mut d = fixture();
        d.scene.objects.push(crate::scene::PageObject::Line {
            start: [0.0; 2],
            end: [8.0, 4.0],
            width_px: 2.0,
            color: [255, 255, 0, 255],
        });
        assert!(export(&d, &path, Choice::RenderedLayers, false, 100000).is_err());
        assert!(!path.exists());
        export(&d, &path, Choice::RenderedImage, false, 100000).unwrap();
        assert_eq!(
            open(&path, 100000).unwrap().layers[0].pixels,
            Renderer::new(&d).output(d.dpi, 100000).unwrap().2
        );
        let mut bytes = fs::read(&path).unwrap();
        bytes[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(open(&path, 100000).is_err());
        bytes.truncate(12);
        fs::write(&path, &bytes).unwrap();
        assert!(open(&path, 100000).is_err());
    }
    #[test]
    fn psd_decoder_fixture() {
        let root = TempDir::new();
        let folder = std::env::var_os("DRAWER_PSD_VERIFY_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.0.clone());
        let mut d = fixture();
        export(
            &d,
            &folder.join("roots.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
        crate::png_export::export(&d, &folder.join("roots-reference.png"), 100000).unwrap();
        let n = d.scene.paper.group_all(2, "Nested paper").unwrap();
        d.scene.paper.folders[n].paper.pattern = Pattern {
            source: 1,
            strength: 0.2,
            ..Default::default()
        };
        export(
            &d,
            &folder.join("nested.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
        crate::png_export::export(&d, &folder.join("reference.png"), 100000).unwrap();
        d.layers[1].locked = false;
        let before = Renderer::new(&d).output(d.dpi, 100000).unwrap().2;
        crate::paper::bake(&mut d, Node::Folder(n)).unwrap();
        assert_eq!(Renderer::new(&d).output(d.dpi, 100000).unwrap().2, before);
        export(
            &d,
            &folder.join("baked.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
    }
}
