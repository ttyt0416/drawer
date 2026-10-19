use crate::document::{Background, CanvasSpec, Document, MAX_DIMENSION, PlacedRaster, RasterLayer};
use std::{
    fs,
    io::{self, BufReader, BufWriter, Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"DRAWER\r\n";
const VERSION: u32 = 47;
const MAX_SCENE_BYTES: usize = 4 * 1024 * 1024;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

pub fn open(path: &Path, budget: usize) -> io::Result<Document> {
    let path = path.canonicalize()?;
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match ext.as_str() {
        "drawer" => read_native(&path, budget),
        "png" => read_png(&path, budget),
        "psd" => crate::psd::open(&path, budget),
        _ => Err(invalid(
            "Supported open formats: .drawer, .png, raw RGB8 .psd merged preview",
        )),
    }
}
fn read_png(path: &Path, budget: usize) -> io::Result<Document> {
    let mut reader = image::ImageReader::open(path)?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(budget as u64);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| invalid(error.to_string()))?
        .into_rgba8();
    let mut document = Document::create(
        CanvasSpec {
            width: image.width(),
            height: image.height(),
            dpi: 300.0,
            background: Background::Transparent,
        },
        budget,
    )
    .map_err(|e| invalid(e.to_string()))?;
    for (destination, source) in document.layers[0].pixels.iter_mut().zip(image.pixels()) {
        *destination = source.0;
    }
    document.title = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    document.source_path = Some(path.into());
    Ok(document)
}
fn read_native(path: &Path, budget: usize) -> io::Result<Document> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > budget as u64 + MAX_SCENE_BYTES as u64 + 1024 * 1024 {
        return Err(invalid("Document exceeds memory budget"));
    }
    let mut reader = BufReader::new(file);
    let mut magic = [0; 8];
    reader.read_exact(&mut magic)?;
    let version = read_u32(&mut reader)?;
    if &magic != MAGIC || !(1..=VERSION).contains(&version) {
        return Err(invalid("Unsupported Drawer document version"));
    }
    let width = read_u32(&mut reader)?;
    let height = read_u32(&mut reader)?;
    let dpi = f64::from_bits(read_u64(&mut reader)?);
    let layers = read_u32(&mut reader)? as usize;
    if !(1..=256).contains(&layers) {
        return Err(invalid("Invalid layer count"));
    }
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(invalid("Invalid document dimensions"));
    }
    let mut document = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi,
            background: Background::Transparent,
        },
        budget,
    )
    .map_err(|e| invalid(e.to_string()))?;
    document.width = width;
    document.height = height;
    document.layers.clear();
    let mut used = 0usize;
    for _ in 0..layers {
        let name_len = read_u32(&mut reader)? as usize;
        if !(1..=4096).contains(&name_len) {
            return Err(invalid("Invalid layer name"));
        }
        let mut name = vec![0; name_len];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| invalid("Layer name must be UTF-8"))?;
        let placement = if version >= 4 {
            match read_u32(&mut reader)? {
                0 => None,
                1 => Some(PlacedRaster {
                    width: read_u32(&mut reader)?,
                    height: read_u32(&mut reader)?,
                    offset: [read_u64(&mut reader)? as i64, read_u64(&mut reader)? as i64],
                }),
                _ => return Err(invalid("Invalid raster placement tag")),
            }
        } else {
            None
        };
        let locked = if version >= 7 {
            match read_u32(&mut reader)? {
                0 => false,
                1 => true,
                _ => return Err(invalid("Invalid layer lock tag")),
            }
        } else {
            false
        };
        let extent = placement.unwrap_or(PlacedRaster {
            width,
            height,
            offset: [0; 2],
        });
        if !extent.valid() {
            return Err(invalid("Invalid raster placement"));
        }
        let count = extent.width as usize * extent.height as usize;
        used = used
            .checked_add(count * 4)
            .ok_or_else(|| invalid("Document size overflow"))?;
        if used > budget || read_u64(&mut reader)? != count as u64 * 4 {
            return Err(invalid("Invalid or oversized raster payload"));
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| invalid("Not enough document memory"))?;
        pixels.resize(count, [0; 4]);
        for chunk in pixels.chunks_mut(4096) {
            let mut buffer = [0u8; 16384];
            let count = chunk.len() * 4;
            reader.read_exact(&mut buffer[..count])?;
            for (pixel, bytes) in chunk.iter_mut().zip(buffer[..count].as_chunks::<4>().0) {
                pixel.copy_from_slice(bytes);
            }
        }
        document.layers.push(RasterLayer {
            locked,
            name,
            pixels,
            placement,
        });
    }
    if version >= 2 {
        let length = read_u32(&mut reader)? as usize;
        if length > MAX_SCENE_BYTES {
            return Err(invalid("Page geometry is too large"));
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes)?;
        document.scene = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
        if !document.scene.valid()
            || !document.scene.layer_ids.matches(document.layers.len())
            || !document.scene.paper.valid_for(document.layers.len())
            || document.memory_bytes() > budget
        {
            return Err(invalid("Invalid or oversized page geometry"));
        }
    }
    if version >= 6 {
        match read_u32(&mut reader)? {
            0 => {}
            1 => {
                document.animation = Some(crate::animation::Timeline::read(
                    &mut reader,
                    budget.saturating_sub(document.memory_bytes()),
                )?)
            }
            _ => return Err(invalid("Invalid animation tag")),
        }
        if document.memory_bytes() > budget {
            return Err(invalid("Animation exceeds document memory budget"));
        }
    }
    let mut trailing = [0];
    if reader.read(&mut trailing)? != 0 {
        return Err(invalid("Unexpected document data"));
    }
    document.mark_saved(path.into());
    Ok(document)
}

pub fn save(document: &mut Document, path: &Path) -> io::Result<()> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("drawer"))
    {
        return Err(invalid("Editable documents must be saved as .drawer"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| invalid("Missing filename"))?,
    );
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(invalid("Choose a regular destination file"));
    }
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        path
    };
    if !(1..=MAX_DIMENSION).contains(&document.width)
        || !(1..=MAX_DIMENSION).contains(&document.height)
        || !document.dpi.is_finite()
        || document.dpi <= 0.0
        || !(1..=256).contains(&document.layers.len())
    {
        return Err(invalid("Invalid document"));
    }
    for layer in &document.layers {
        if !layer.valid(document.width, document.height) {
            return Err(invalid("Invalid raster layer"));
        }
    }
    if document.animation.as_ref().is_some_and(|a| !a.valid()) {
        return Err(invalid("Invalid animation"));
    }
    if !document.scene.valid()
        || !document.scene.layer_ids.matches(document.layers.len())
        || !document.scene.paper.valid_for(document.layers.len())
        || document.scene.size_bytes() > MAX_SCENE_BYTES
    {
        return Err(invalid("Invalid or oversized page geometry"));
    }
    let scene = serde_json::to_vec(&document.scene).map_err(|e| invalid(e.to_string()))?;
    if scene.len() > MAX_SCENE_BYTES {
        return Err(invalid("Page geometry is too large"));
    }
    let mut temp_name = path.file_name().unwrap().to_owned();
    temp_name.push(".drawer-write.tmp");
    let temporary = path.with_file_name(temp_name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut writer = BufWriter::new(file);
    let result = (|| {
        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;
        writer.write_all(&document.width.to_le_bytes())?;
        writer.write_all(&document.height.to_le_bytes())?;
        writer.write_all(&document.dpi.to_bits().to_le_bytes())?;
        writer.write_all(&(document.layers.len() as u32).to_le_bytes())?;
        for layer in &document.layers {
            writer.write_all(&(layer.name.len() as u32).to_le_bytes())?;
            writer.write_all(layer.name.as_bytes())?;
            if let Some(p) = layer.placement {
                writer.write_all(&1u32.to_le_bytes())?;
                writer.write_all(&p.width.to_le_bytes())?;
                writer.write_all(&p.height.to_le_bytes())?;
                writer.write_all(&p.offset[0].to_le_bytes())?;
                writer.write_all(&p.offset[1].to_le_bytes())?;
            } else {
                writer.write_all(&0u32.to_le_bytes())?;
            }
            writer.write_all(&u32::from(layer.locked).to_le_bytes())?;
            writer.write_all(&(layer.pixels.len() as u64 * 4).to_le_bytes())?;
            for chunk in layer.pixels.chunks(4096) {
                let mut buffer = [0u8; 16384];
                for (bytes, pixel) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(chunk) {
                    bytes.copy_from_slice(pixel);
                }
                writer.write_all(&buffer[..chunk.len() * 4])?;
            }
        }
        writer.write_all(&(scene.len() as u32).to_le_bytes())?;
        writer.write_all(&scene)?;
        if let Some(animation) = &document.animation {
            writer.write_all(&1u32.to_le_bytes())?;
            animation.write(&mut writer)?;
        } else {
            writer.write_all(&0u32.to_le_bytes())?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()
    })();
    drop(writer);
    let result = result.and_then(|_| fs::rename(&temporary, &path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    document.mark_saved(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    fn sample() -> Document {
        Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 144.0,
                background: Background::Transparent,
            },
            16,
        )
        .unwrap()
    }
    #[test]
    fn pigment_style_roundtrip_and_legacy_version_ten_defaults() {
        let root = TempDir::new();
        let path = root.0.join("pigment.drawer");
        let mut d = sample();
        d.scene.mouse_strokes.push(crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: crate::painting::Style {
                clone_stamp: Default::default(),
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: crate::brush_material::Settings {
                    mode: crate::brush_material::Mode::Water,
                    supply: 0.2,
                    keep_density: true,
                    alpha_lock: true,
                    ..Default::default()
                },
                shape: Default::default(),
                tip: crate::assets::BrushTip::Pen,
                color: [40, 80, 120, 255],
                opacity: 0.5,
                hardness: 0.3,
                spacing: 0.2,
                smoothing: 0.8,
                pressure: Default::default(),
            },
            count: 1,
            ends: Default::default(),
            native_pressure: true,
        });
        save(&mut d, &path).unwrap();
        assert!(open(&path, 4096).unwrap().snapshot() == d.snapshot());
        let bytes = fs::read(&path).unwrap();
        let offset = 32 + 4 + d.layers[0].name.len() + 4 + 4 + 8 + d.layers[0].pixels.len() * 4;
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length]).unwrap();
        for record in value["mouse_strokes"].as_array_mut().unwrap() {
            record["style"].as_object_mut().unwrap().remove("material");
        }
        let json = serde_json::to_vec(&value).unwrap();
        let mut legacy = bytes[..offset].to_vec();
        legacy.extend_from_slice(&(json.len() as u32).to_le_bytes());
        legacy.extend_from_slice(&json);
        legacy.extend_from_slice(&bytes[offset + 4 + length..]);
        legacy[8..12].copy_from_slice(&10u32.to_le_bytes());
        fs::write(&path, legacy).unwrap();
        let mut expected = d.snapshot();
        expected.scene.mouse_strokes[0].style.material = Default::default();
        assert!(open(&path, 4096).unwrap().snapshot() == expected);
    }
    #[test]
    fn frozen_geometry_native_pressure_roundtrip_and_version_nine_defaults() {
        let root = TempDir::new();
        let path = root.0.join("geometry.drawer");
        let mut d = sample();
        d.scene.mouse_strokes.push(crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: crate::painting::Style {
                clone_stamp: Default::default(),
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: Default::default(),
                shape: Default::default(),
                tip: crate::assets::BrushTip::Pen,
                color: [40, 80, 120, 255],
                opacity: 0.5,
                hardness: 0.3,
                spacing: 0.2,
                smoothing: 0.8,
                pressure: Default::default(),
            },
            count: 1,
            ends: crate::stroke_ends::Settings {
                method: Default::default(),
                fade_minimum: 0.0,
                start: 2.0,
                end: 3.0,
            },
            native_pressure: true,
        });
        save(&mut d, &path).unwrap();
        assert!(open(&path, 4096).unwrap().snapshot() == d.snapshot());
        let bytes = fs::read(&path).unwrap();
        let offset = 32 + 4 + d.layers[0].name.len() + 4 + 4 + 8 + d.layers[0].pixels.len() * 4;
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length]).unwrap();
        for record in value["mouse_strokes"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("ends");
            record.as_object_mut().unwrap().remove("native_pressure");
        }
        let old_json = serde_json::to_vec(&value).unwrap();
        let mut legacy = bytes[..offset].to_vec();
        legacy.extend_from_slice(&(old_json.len() as u32).to_le_bytes());
        legacy.extend_from_slice(&old_json);
        legacy.extend_from_slice(&bytes[offset + 4 + length..]);
        legacy[8..12].copy_from_slice(&9u32.to_le_bytes());
        fs::write(&path, legacy).unwrap();
        let reopened = open(&path, 4096).unwrap();
        assert_eq!(reopened.layers, d.layers);
        assert_eq!(
            reopened.scene.mouse_strokes[0].style,
            d.scene.mouse_strokes[0].style
        );
        assert_eq!(reopened.scene.mouse_strokes[0].ends, Default::default());
        assert!(!reopened.scene.mouse_strokes[0].native_pressure);
    }
    #[test]
    fn native_round_trip_preserves_layers_and_failed_save_keeps_previous_file() {
        let root = TempDir::new();
        let path = root.0.join("same.drawer");
        let mut document = sample();
        document.layers[0].name = "원본 레이어".into();
        document.edit_pixel(0, 1, 0, [20, 40, 60, 255]);
        document.layers.push(RasterLayer {
            locked: false,
            name: "Second".into(),
            pixels: vec![[0; 4]; 2],
            placement: None,
        });
        save(&mut document, &path).unwrap();
        let before = fs::read(&path).unwrap();
        let restored = open(&path, 16).unwrap();
        assert_eq!(restored.layers, document.layers);
        assert_eq!(restored.dpi, 144.0);
        assert!(!restored.dirty);
        assert_eq!(restored.history_counts(), (0, 0));
        let temporary = root.0.join("same.drawer.drawer-write.tmp");
        fs::write(&temporary, "other operation").unwrap();
        assert!(save(&mut document, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_to_string(&temporary).unwrap(), "other operation");
        fs::remove_file(temporary).unwrap();
        document.layers[0].pixels.clear();
        assert!(save(&mut document, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    #[test]
    fn rejects_truncation_bad_versions_and_oversized_documents() {
        let root = TempDir::new();
        let path = root.0.join("bad.drawer");
        save(&mut sample(), &path).unwrap();
        let original = fs::read(&path).unwrap();
        for bytes in [
            &original[..7],
            &original[..35],
            &original[..original.len() - 1],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(open(&path, 16).is_err());
        }
        let mut bytes = original.clone();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(open(&path, 16).is_err());
        fs::write(&path, original).unwrap();
        assert!(open(&path, 7).is_err());
    }
    #[test]
    fn version_one_files_still_open_and_invalid_geometry_is_not_saved() {
        let root = TempDir::new();
        let path = root.0.join("legacy.drawer");
        let mut doc = sample();
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let placement = 32 + 4 + doc.layers[0].name.len();
        bytes.drain(placement..placement + 8);
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes.truncate(32 + 4 + "Layer 1".len() + 8 + 8);
        fs::write(&path, &bytes).unwrap();
        let reopened = open(&path, 16).unwrap();
        assert_eq!(doc.snapshot(), reopened.snapshot());
        doc.scene.transform.0[0] = f64::NAN;
        assert!(save(&mut doc, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn layer_locks_selection_translation_round_trip_and_version_six_is_unlocked() {
        let root = TempDir::new();
        let path = root.0.join("locked.drawer");
        let mut doc = sample();
        doc.layers[0].locked = true;
        doc.scene.selection_translation = [1.0, 10.0];
        save(&mut doc, &path).unwrap();
        assert!(open(&path, 16).unwrap().snapshot() == doc.snapshot());
        let mut bytes = fs::read(&path).unwrap();
        let tag = 32 + 4 + doc.layers[0].name.len() + 4;
        bytes.drain(tag..tag + 4);
        bytes[8..12].copy_from_slice(&6u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let legacy = open(&path, 16).unwrap();
        assert!(!legacy.layers[0].locked);
        assert_eq!(legacy.layers[0].pixels, doc.layers[0].pixels);
        assert_eq!(legacy.scene.selection_translation, [1.0, 10.0]);
    }
    #[test]
    fn version_seven_without_mouse_records_preserves_existing_lock_and_geometry() {
        let root = TempDir::new();
        let path = root.0.join("version7.drawer");
        let mut doc = sample();
        doc.layers[0].locked = true;
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&7u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(open(&path, 16).unwrap().snapshot() == doc.snapshot());
    }
    #[test]
    fn version_two_geometry_without_grid_loads_with_default_grid() {
        let root = TempDir::new();
        let path = root.0.join("legacy2.drawer");
        let mut doc = sample();
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let placement = 32 + 4 + doc.layers[0].name.len();
        bytes.drain(placement..placement + 8);
        let offset = 32 + 4 + doc.layers[0].name.len() + 8 + doc.layers[0].pixels.len() * 4;
        let mut scene = serde_json::to_value(&doc.scene).unwrap();
        scene.as_object_mut().unwrap().remove("grid");
        let json = serde_json::to_vec(&scene).unwrap();
        bytes.truncate(offset);
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&json);
        fs::write(&path, bytes).unwrap();
        assert_eq!(open(&path, 16).unwrap().snapshot(), doc.snapshot());
    }
    #[test]
    fn png_import_preserves_original_and_starts_as_an_unsaved_document() {
        let root = TempDir::new();
        let path = root.0.join("picture.png");
        let rgba = vec![1, 2, 3, 255, 0, 0, 0, 0];
        image::save_buffer_with_format(
            &path,
            &rgba,
            2,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let document = open(&path, 1024 * 1024).unwrap();
        assert_eq!(document.layers[0].pixels, [[1, 2, 3, 255], [0; 4]]);
        assert!(document.dirty && document.path.is_none());
        assert!(document.source_path.is_some());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(open(&path, 1).is_err());
    }
}
