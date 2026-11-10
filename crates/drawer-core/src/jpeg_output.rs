//! Baseline JPEG with independent quality tables and explicit chroma sampling.
use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampling {
    #[default]
    S444,
    S422,
    S420,
}
impl Sampling {
    pub fn label(self) -> &'static str {
        match self {
            Self::S444 => "4:4:4",
            Self::S422 => "4:2:2",
            Self::S420 => "4:2:0",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub luma: u8,
    pub chroma: u8,
    pub sampling: Sampling,
}
pub struct Image<'a> {
    pub rgb: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub icc: Option<&'a [u8]>,
}
// ITU-T T.81 Annex K example tables, in natural row order, not zigzag order.
const LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];
fn table(base: [u16; 64], quality: u8) -> Box<[u16; 64]> {
    let q = u32::from(quality);
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    Box::new(base.map(|v| ((u32::from(v) * scale + 50) / 100).clamp(1, 255) as u16))
}
struct Output<'a> {
    bytes: Vec<u8>,
    limit: usize,
    cancel: &'a AtomicBool,
}
impl Write for Output<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("JPEG encoding canceled"));
        }
        if self.bytes.len().saturating_add(b.len()) > self.limit {
            return Err(io::Error::other("JPEG output memory limit"));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn encode(
    image: Image<'_>,
    s: Settings,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if !(1..=100).contains(&s.luma)
        || !(1..=100).contains(&s.chroma)
        || image.width == 0
        || image.height == 0
        || image.width > 32768
        || image.height > 32768
        || image.width as usize * image.height as usize > 4 * 1024 * 1024
        || image
            .rgb
            .len()
            .saturating_mul(12)
            .saturating_add(image.icc.map_or(0, |v| v.len().saturating_mul(3)))
            > budget
        || image.rgb.len() != image.width as usize * image.height as usize * 3
        || !image.dpi.is_finite()
        || !(1.0..=4800.).contains(&image.dpi)
        || image.icc.is_some_and(|v| v.len() > 4 * 1024 * 1024)
    {
        return Err("Invalid baseline JPEG options/image/profile".into());
    }
    crate::asset_catalog::canceled(cancel)?;
    let mut out = Output {
        bytes: vec![],
        limit: (budget / 2).min(64 * 1024 * 1024),
        cancel,
    };
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, 100);
    encoder.set_density(jpeg_encoder::PixelDensity::dpi(image.dpi.round() as u16));
    encoder.set_sampling_factor(match s.sampling {
        Sampling::S444 => jpeg_encoder::SamplingFactor::R_4_4_4,
        Sampling::S422 => jpeg_encoder::SamplingFactor::R_4_2_2,
        Sampling::S420 => jpeg_encoder::SamplingFactor::R_4_2_0,
    });
    encoder.set_quantization_tables(
        jpeg_encoder::QuantizationTableType::Custom(table(LUMA, s.luma)),
        jpeg_encoder::QuantizationTableType::Custom(table(CHROMA, s.chroma)),
    );
    if let Some(icc) = image.icc {
        encoder.add_icc_profile(icc).map_err(|e| e.to_string())?;
    }
    encoder
        .encode(
            image.rgb,
            image.width as u16,
            image.height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(|e| e.to_string())?;
    crate::asset_catalog::canceled(cancel)?;
    Ok(out.bytes)
}
#[path = "jpeg_output_verify.rs"]
pub mod verify;
