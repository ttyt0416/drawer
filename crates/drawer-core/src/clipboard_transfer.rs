//! Bounded clipboard geometry and public raster decoding, without OS/global state.
use crate::{document::Document, paste::ClipboardImage, scene::Rect};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bounds {
    Content,
    Selection,
}
fn selection_bounds(d: &Document) -> Result<Rect, String> {
    if !(d.scene.selection.is_some() || d.scene.selection_mask.is_some()) {
        return Err("Make an area selection for selection-bounds copy".into());
    }
    let mut out = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    let mut add = |r: Rect| {
        for p in [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]] {
            let p = d.scene.selection_point(p);
            for (i, v) in p.into_iter().enumerate() {
                out.min[i] = out.min[i].min(v);
                out.max[i] = out.max[i].max(v);
            }
        }
    };
    if let Some(mask) = &d.scene.selection_mask {
        for r in &mask.runs {
            add(Rect {
                min: [r.x as f64, r.y as f64],
                max: [r.x as f64 + r.length as f64, r.y as f64 + 1.],
            });
        }
    } else if let Some(r) = d.scene.selection {
        add(r);
    }
    if !out.valid()
        || out
            .min
            .iter()
            .chain(&out.max)
            .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Empty or oversized clipboard selection bounds".into());
    }
    Ok(out)
}
pub fn copy(
    d: &Document,
    layer: usize,
    mode: Bounds,
    budget: usize,
) -> Result<ClipboardImage, String> {
    let raster = d.layers.get(layer).ok_or("Select a raster layer")?;
    let extent = raster.extent(d.width, d.height);
    if !raster.valid(d.width, d.height) || !d.color.valid_layers(&d.layers) {
        return Err("Invalid clipboard source layer".into());
    }
    let mut min = extent.offset;
    let mut max = [min[0] + extent.width as i64, min[1] + extent.height as i64];
    if (d.scene.selection.is_some() || d.scene.selection_mask.is_some())
        || mode == Bounds::Selection
    {
        let r = selection_bounds(d)?;
        for i in 0..2 {
            let a = r.min[i].floor() as i64;
            let b = r.max[i].ceil() as i64;
            if mode == Bounds::Selection {
                min[i] = a;
                max[i] = b;
            } else {
                min[i] = min[i].max(a);
                max[i] = max[i].min(b);
            }
        }
    }
    if mode == Bounds::Content && (0..2).any(|i| min[i] >= max[i]) {
        return Err("Selection does not overlap the active layer".into());
    }
    let precise = d.color != crate::color_storage::Config::legacy();
    if mode == Bounds::Content {
        let mut left = max;
        let mut right = min;
        for y in min[1]..max[1] {
            for x in min[0]..max[0] {
                if raster
                    .sample(d.width, d.height, x, y)
                    .coverage(
                        d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]) as f64 / 255.,
                    )
                    .is_ok_and(|p| {
                        if precise {
                            p.rgba16()[3] > 0
                        } else {
                            p.rgba8()[3] > 0
                        }
                    })
                {
                    left[0] = left[0].min(x);
                    left[1] = left[1].min(y);
                    right[0] = right[0].max(x + 1);
                    right[1] = right[1].max(y + 1);
                }
            }
        }
        min = left;
        max = right;
    }
    let size = [max[0] - min[0], max[1] - min[1]];
    if size.iter().any(|v| *v <= 0 || *v > 32768)
        || size[0].saturating_mul(size[1]) > 4 * 1024 * 1024
        || size[0]
            .saturating_mul(size[1])
            .saturating_mul(if precise { 32 } else { 16 }) as u64
            > budget as u64
    {
        return Err("Empty clipboard content or clipboard work memory limit".into());
    }
    let count = (size[0] * size[1]) as usize;
    let mut pixels = Vec::with_capacity(count);
    let mut high = precise.then(|| Vec::with_capacity(count));
    let mut any = false;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let p = raster.sample(d.width, d.height, x, y).coverage(
                d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]) as f64 / 255.,
            )?;
            any |= if precise {
                p.rgba16()[3] > 0
            } else {
                p.rgba8()[3] > 0
            };
            pixels.push(p.rgba8());
            if let Some(h) = &mut high {
                h.push(p.rgba16());
            }
        }
    }
    if !any {
        return Err("The selection contains no nontransparent source pixels".into());
    }
    let packet = ClipboardImage {
        name: Some(raster.name.clone()),
        color: high.map(|pixels| crate::paste::ClipboardColor {
            profile: d.color.profile.clone(),
            pixels,
        }),
        image: crate::references::LocalImage {
            width: size[0] as u32,
            height: size[1] as u32,
            pixels,
        },
        origin: Some(min),
    };
    packet.encode(budget)?;
    Ok(packet)
}
/// Prepare before the OS write; commit only after the caller reports complete clipboard success.
pub fn cut(
    d: &Document,
    layer: usize,
    budget: usize,
) -> Result<crate::sizing::DocumentState, String> {
    let raster = d.layers.get(layer).ok_or("Select a raster layer")?;
    let flags = crate::layer_properties::inherited(d, crate::layer_properties::raster(d, layer));
    if flags.locked
        || flags.pixels_locked
        || flags.hidden
        || crate::clipping::alpha_locked(d, layer)
    {
        return Err("Choose a visible unlocked raster without alpha protection to cut".into());
    }
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Cut snapshot work memory limit".into());
    }
    let extent = raster.extent(d.width, d.height);
    let mut state = d.snapshot();
    let target = &mut state.layers[layer];
    for y in 0..extent.height {
        for x in 0..extent.width {
            let n = y as usize * extent.width as usize + x as usize;
            let point = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let coverage = d
                .scene
                .selection_coverage([point[0] as f64 + 0.5, point[1] as f64 + 0.5]);
            if coverage == 0 {
                continue;
            }
            if let Some(precise) = &mut target.precise {
                let p = crate::color_precision::Sample::from16(precise[n])
                    .coverage(1. - coverage as f64 / 255.)?;
                precise[n] = p.rgba16();
                target.pixels[n] = p.rgba8();
            } else {
                target.pixels[n][3] =
                    ((target.pixels[n][3] as u32 * (255 - coverage as u32) + 127) / 255) as u8;
                if target.pixels[n][3] == 0 && d.color != crate::color_storage::Config::legacy() {
                    target.pixels[n] = [0; 4];
                }
            }
        }
    }
    Ok(state)
}
pub fn view_center(d: &Document, ppp: f64) -> Result<[f64; 2], String> {
    if !ppp.is_finite()
        || ppp <= 0.
        || !d.view.zoom.is_finite()
        || d.view.zoom <= 0.
        || !d.view.angle.is_finite()
        || d.view.pan.iter().any(|v| !v.is_finite())
    {
        return Err("Invalid paste view geometry".into());
    }
    Ok(d.view
        .screen_to_document([0.; 2], [d.width as f64, d.height as f64], [0.; 2], ppp))
}
pub fn decode_png(bytes: &[u8], budget: usize) -> Result<ClipboardImage, String> {
    use image::ImageDecoder;
    let c = std::sync::atomic::AtomicBool::new(false);
    let info = crate::png_io::validate(bytes, &c).map_err(|e| e.to_string())?;
    if bytes.len() > budget.min(64 * 1024 * 1024) {
        return Err("Clipboard PNG encoded limit".into());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(budget as u64);
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    let decoder = image::codecs::png::PngDecoder::with_limits(std::io::Cursor::new(bytes), limits)
        .map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    let count = width as usize * height as usize;
    if count > 4 * 1024 * 1024
        || count
            .saturating_mul(if info.bits == 16 { 32 } else { 12 })
            .saturating_add(bytes.len())
            > budget
    {
        return Err("Clipboard PNG decoded work memory limit".into());
    }
    if info.bits == 16 {
        let page = crate::png_io::decode16(bytes, true, budget, &c).map_err(|e| e.to_string())?;
        return Ok(ClipboardImage {
            name: None,
            color: page.color,
            image: page.image,
            origin: None,
        });
    }
    let gray = matches!(
        decoder.color_type(),
        image::ColorType::L8 | image::ColorType::La8
    );
    let mut decoder = decoder;
    let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
    let mut rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    if let Some(profile) = profile {
        crate::image_import::convert_profile(&mut rgba, &profile, gray, &c)
            .map_err(|e| e.to_string())?;
    }
    Ok(ClipboardImage {
        name: None,
        color: None,
        image: crate::references::LocalImage::from_rgba(
            width as usize,
            height as usize,
            rgba.as_raw(),
            budget,
        )
        .map_err(|e| e.to_string())?,
        origin: None,
    })
}
pub fn public_rgba(packet: &ClipboardImage, budget: usize) -> Result<Vec<u8>, String> {
    let count = packet.image.width as usize * packet.image.height as usize;
    if count == 0
        || count > 4 * 1024 * 1024
        || packet.image.pixels.len() != count
        || count.saturating_mul(12) > budget
    {
        return Err("Public clipboard image work limit".into());
    }
    if let Some(color) = &packet.color {
        if color.pixels.len() != count || !color.profile.valid() {
            return Err("Invalid clipboard color source".into());
        }
        let transform = crate::color_profile::Transform::new(
            &color.profile,
            &crate::color_profile::RgbProfile::Srgb,
        )?;
        let mut bytes = Vec::with_capacity(count * 4);
        for p in &color.pixels {
            bytes.extend(
                transform
                    .map(crate::color_precision::Sample::from16(*p))?
                    .rgba8(),
            );
        }
        Ok(bytes)
    } else {
        Ok(packet.image.pixels.iter().flatten().copied().collect())
    }
}
pub fn decode_dibv5(bytes: &[u8], budget: usize) -> Result<ClipboardImage, String> {
    if bytes.len() < 124 || bytes.len() > budget.min(64 * 1024 * 1024) {
        return Err("Clipboard DIBV5 byte limit/header".into());
    }
    let at = |p| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    let width = at(4) as i32;
    let height = at(8) as i32;
    let bits = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
    let channels = if bits == 32 { 4 } else { 3 };
    if at(0) != 124
        || width <= 0
        || height == 0
        || height == i32::MIN
        || width > 32768
        || height.unsigned_abs() > 32768
        || !matches!(bits, 24 | 32)
        || bytes[12..14] != 1u16.to_le_bytes()
        || !matches!(at(16), 0 | 3)
        || at(32) != 0
        || at(112) != 0
        || at(116) != 0
        || !matches!(at(56), 0 | 0x73524742 | 0x57696e20)
        || at(56) == 0 && bytes[60..108].iter().any(|v| *v != 0)
    {
        return Err(
            "Unsupported DIBV5 header/masks/profile; external profile paths are not followed"
                .into(),
        );
    }
    if at(16) == 3
        && (bits != 32
            || [at(40), at(44), at(48)] != [0x00ff0000, 0x0000ff00, 0x000000ff]
            || !matches!(at(52), 0 | 0xff000000))
    {
        return Err("Unsupported DIBV5 channel masks".into());
    }
    let count = width as usize * height.unsigned_abs() as usize;
    let stride = (width as usize * channels).div_ceil(4) * 4;
    if count > 4 * 1024 * 1024
        || count
            .saturating_mul(8)
            .saturating_add(bytes.len().saturating_mul(2))
            > budget
        || 124 + stride * height.unsigned_abs() as usize > bytes.len()
    {
        return Err("Truncated DIBV5 or decoded work limit".into());
    }
    // A verified file header supplies an explicit pixel offset. The pinned image
    // decoder's headerless V5 path skips an additional12 mask bytes after V5.
    let mut file = vec![0u8; 14];
    file[..2].copy_from_slice(b"BM");
    file[2..6].copy_from_slice(&((bytes.len() + 14) as u32).to_le_bytes());
    file[10..14].copy_from_slice(&138u32.to_le_bytes());
    file.extend_from_slice(bytes);
    let decoder = image::codecs::bmp::BmpDecoder::new(std::io::Cursor::new(file))
        .map_err(|e| e.to_string())?;
    let rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(ClipboardImage {
        name: None,
        color: None,
        image: crate::references::LocalImage::from_rgba(
            width as usize,
            height.unsigned_abs() as usize,
            rgba.as_raw(),
            budget,
        )
        .map_err(|e| e.to_string())?,
        origin: None,
    })
}
#[path = "clipboard_transfer_verify.rs"]
pub mod verify;
