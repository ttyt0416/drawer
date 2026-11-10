//! Explicit grayscale transfer and BMP INFO24/V5-alpha32 wire layouts.
use crate::color_precision::Sample;
use std::sync::atomic::AtomicBool;
pub(crate) fn gray(p: Sample) -> Result<Sample, String> {
    let linear = |v: f64| {
        let v = v.clamp(0., 1.);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let rgb = p.rgb();
    let y = 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
    let g = if y <= 0.0031308 {
        12.92 * y
    } else {
        1.055 * y.powf(1. / 2.4) - 0.055
    };
    p.with_rgb([g; 3])
}
pub(crate) fn gray_profile() -> Result<Vec<u8>, String> {
    let mut p = moxcms::ColorProfile::new_gray_with_gamma(2.2);
    p.gray_trc = moxcms::ColorProfile::new_srgb().red_trc;
    let mut bytes = p.encode().map_err(|e| e.to_string())?;
    for (slot, value) in bytes[24..36]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([2000u16, 1, 1, 0, 0, 0])
    {
        slot.copy_from_slice(&value.to_be_bytes());
    }
    Ok(bytes)
}
pub(crate) fn own_gray_profile(bytes: &[u8]) -> bool {
    gray_profile().is_ok_and(|p| p == bytes)
}
/// 1 inch is exactly0.0254m; the integer field is an approximation of source DPI.
pub fn ppm(dpi: f64) -> Result<i32, String> {
    if !dpi.is_finite() || !(1.0..=4800.).contains(&dpi) {
        return Err("Output DPI must be1..4800".into());
    }
    Ok((dpi / 0.0254).round() as i32)
}
pub(crate) fn bmp(
    data: &[u8],
    size: [u32; 2],
    alpha: bool,
    dpi: f64,
    c: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let [width, height] = size;
    let channels = if alpha { 4 } else { 3 };
    let stride = (width as usize * channels).div_ceil(4) * 4;
    if width == 0
        || height == 0
        || width > 32768
        || height > 32768
        || width as usize * height as usize > 4 * 1024 * 1024
        || data.len() != width as usize * height as usize * channels
    {
        return Err("Invalid BMP channel dimensions".into());
    }
    let header = if alpha { 124usize } else { 40 };
    let offset = 14 + header;
    let count = stride * height as usize;
    let mut bytes = vec![0; offset + count];
    bytes[..2].copy_from_slice(b"BM");
    let put =
        |b: &mut [u8], at: usize, value: u32| b[at..at + 4].copy_from_slice(&value.to_le_bytes());
    put(&mut bytes, 2, (offset + count) as u32);
    put(&mut bytes, 10, offset as u32);
    put(&mut bytes, 14, header as u32);
    put(&mut bytes, 18, width);
    put(&mut bytes, 22, height);
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&(if alpha { 32u16 } else { 24 }).to_le_bytes());
    put(&mut bytes, 30, if alpha { 3 } else { 0 });
    put(&mut bytes, 34, count as u32);
    let resolution = ppm(dpi)? as u32;
    put(&mut bytes, 38, resolution);
    put(&mut bytes, 42, resolution);
    if alpha {
        for (at, value) in [
            (40, 0x00ff0000),
            (44, 0x0000ff00),
            (48, 0x000000ff),
            (52, 0xff000000),
            (56, 0x73524742),
            (108, 4),
        ] {
            put(&mut bytes, 14 + at, value);
        }
    }
    for y in 0..height as usize {
        crate::asset_catalog::canceled(c)?;
        let row = offset + (height as usize - 1 - y) * stride;
        for x in 0..width as usize {
            let src = (y * width as usize + x) * channels;
            let dst = row + x * channels;
            bytes[dst..dst + 3].copy_from_slice(&[data[src + 2], data[src + 1], data[src]]);
            if alpha {
                bytes[dst + 3] = data[src + 3];
            }
        }
    }
    Ok(bytes)
}
#[path = "raster_channels_verify.rs"]
pub mod verify;
