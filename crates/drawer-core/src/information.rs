use crate::{aids, document::Document, scene::Point};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Document,
    Pixels,
    Millimeters,
    Centimeters,
    Inches,
    Points,
}
impl Unit {
    pub const ALL: [Self; 6] = [
        Self::Document,
        Self::Pixels,
        Self::Millimeters,
        Self::Centimeters,
        Self::Inches,
        Self::Points,
    ];
    pub fn factor(self, dpi: f64, document: aids::Unit) -> f64 {
        match self {
            Self::Document => match document {
                aids::Unit::Pixels => 1.0,
                aids::Unit::Millimeters => 25.4 / dpi,
                aids::Unit::Inches => 1.0 / dpi,
            },
            Self::Pixels => 1.0,
            Self::Millimeters => 25.4 / dpi,
            Self::Centimeters => 2.54 / dpi,
            Self::Inches => 1.0 / dpi,
            Self::Points => 72.0 / dpi,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub theme: Theme,
    pub side: Side,
    pub visible: bool,
    pub unit: Unit,
    pub rgb: bool,
    pub hsv: bool,
    pub hsl: bool,
    pub cmyk: bool,
    pub output_profile: Option<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            theme: Theme::Dark,
            side: Side::Left,
            visible: false,
            unit: Unit::Document,
            rgb: true,
            hsv: false,
            hsl: false,
            cmyk: false,
            output_profile: None,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self
                .output_profile
                .as_ref()
                .is_none_or(|p| p.is_absolute() && p.as_os_str().len() <= 4096)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub start: Point,
    pub end: Point,
    pub delta: Point,
    pub length: f64,
}
pub fn measure(
    start: Point,
    end: Point,
    dpi: f64,
    unit: Unit,
    document: aids::Unit,
) -> Option<Measurement> {
    if !dpi.is_finite()
        || dpi <= 0.0
        || !start
            .iter()
            .chain(end.iter())
            .all(|p| p.is_finite() && p.abs() <= 1e12)
    {
        return None;
    }
    let f = unit.factor(dpi, document);
    let start = start.map(|v| v * f);
    let end = end.map(|v| v * f);
    let delta = [end[0] - start[0], end[1] - start[1]];
    Some(Measurement {
        start,
        end,
        delta,
        length: delta[0].hypot(delta[1]),
    })
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub rgba: [u8; 4],
    pub hsv: [f64; 3],
    pub hsl: [f64; 3],
}
pub fn color(rgba: [u8; 4]) -> Color {
    let r = rgba[0] as f64 / 255.0;
    let g = rgba[1] as f64 / 255.0;
    let b = rgba[2] as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let light = (max + min) / 2.0;
    Color {
        rgba,
        hsv: [h, if max == 0.0 { 0.0 } else { d / max }, max],
        hsl: [
            h,
            if d == 0.0 {
                0.0
            } else {
                d / (1.0 - (2.0 * light - 1.0).abs())
            },
            light,
        ],
    }
}
pub fn sample(document: &Document, point: Point) -> Option<Color> {
    if point.iter().any(|p| !p.is_finite())
        || point[0] < 0.0
        || point[1] < 0.0
        || point[0] >= document.width as f64
        || point[1] >= document.height as f64
    {
        return None;
    }
    Some(color(
        crate::render::Renderer::new(document)
            .display_sample([point[0].floor() + 0.5, point[1].floor() + 0.5])
            .ok()?
            .rgba8(),
    ))
}
pub struct OutputProfile {
    transform: Arc<moxcms::Transform8BitExecutor>,
}
impl OutputProfile {
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(io::Error::other("Output profile exceeds 4 MiB"));
        }
        let profile = moxcms::ColorProfile::new_from_slice_with_options(
            bytes,
            moxcms::ParsingOptions {
                max_profile_size: 4 * 1024 * 1024,
                max_allowed_clut_size: 4 * 1024 * 1024,
                max_allowed_trc_size: 1_000_000,
            },
        )
        .map_err(|e| io::Error::other(format!("Invalid output ICC profile: {e}")))?;
        if profile.color_space != moxcms::DataColorSpace::Cmyk {
            return Err(io::Error::other("Select a CMYK output ICC profile"));
        }
        let transform = moxcms::ColorProfile::new_srgb()
            .create_transform_8bit(
                moxcms::Layout::Rgb,
                &profile,
                moxcms::Layout::Rgba,
                Default::default(),
            )
            .map_err(|e| io::Error::other(format!("No usable RGB-to-CMYK conversion: {e}")))?;
        Ok(Self { transform })
    }
    pub fn convert(&self, rgba: [u8; 4]) -> io::Result<[f64; 4]> {
        let mut output = [0u8; 4];
        self.transform
            .transform(&rgba[..3], &mut output)
            .map_err(io::Error::other)?;
        Ok(output.map(|v| v as f64 * 100.0 / 255.0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_four_five_units_dpi_rotated_view_boundaries_transparency_and_hsv_hsl() {
        let p = [10.0, 20.0];
        let q = [13.0, 24.0];
        let m = measure(p, q, 300.0, Unit::Pixels, aids::Unit::Pixels).unwrap();
        assert_eq!(m.delta, [3.0, 4.0]);
        assert_eq!(m.length, 5.0);
        for dpi in [72.0, 300.0, 600.0] {
            let mm = measure(p, q, dpi, Unit::Millimeters, aids::Unit::Pixels).unwrap();
            assert!((mm.length - 5.0 * 25.4 / dpi).abs() < 1e-12);
            assert!(
                (measure(p, q, dpi, Unit::Points, aids::Unit::Pixels)
                    .unwrap()
                    .length
                    - 5.0 * 72.0 / dpi)
                    .abs()
                    < 1e-12
            );
        }
        let mut d = Document::create(
            crate::document::CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            32,
        )
        .unwrap();
        d.edit_pixel(0, 0, 0, [255, 0, 0, 128]);
        d.edit_pixel(0, 1, 0, [0, 255, 0, 0]);
        let before = d.snapshot();
        assert_eq!(sample(&d, [0.999, 0.0]).unwrap().rgba, [255, 0, 0, 128]);
        assert_eq!(sample(&d, [1.0, 0.0]).unwrap().rgba, [0; 4]);
        assert!(sample(&d, [-0.01, 0.0]).is_none());
        assert!(sample(&d, [2.0, 0.0]).is_none());
        let red = color([255, 0, 0, 0]);
        assert_eq!(red.hsv, [0.0, 1.0, 1.0]);
        assert_eq!(red.hsl, [0.0, 1.0, 0.5]);
        assert_eq!(d.snapshot(), before);
        let view = crate::view::View {
            zoom: 64.0,
            angle: 37.0,
            flipped: true,
            ..Default::default()
        };
        for point in [p, q] {
            let screen = view.document_to_screen(point, [100.0; 2], [400.0; 2], 1.5);
            let actual = view.screen_to_document(screen, [100.0; 2], [400.0; 2], 1.5);
            assert!((actual[0] - point[0]).abs() < 1e-10 && (actual[1] - point[1]).abs() < 1e-10);
        }
    }
    fn fixture(cmyk: [u8; 4]) -> Vec<u8> {
        use moxcms::*;
        let mut p = ColorProfile::new_srgb();
        p.color_space = DataColorSpace::Cmyk;
        p.profile_class = ProfileClass::OutputDevice;
        p.pcs = DataColorSpace::Xyz;
        p.cicp = None;
        p.red_colorant = Default::default();
        p.green_colorant = Default::default();
        p.blue_colorant = Default::default();
        p.red_trc = None;
        p.green_trc = None;
        p.blue_trc = None;
        p.lut_b_to_a_perceptual = Some(LutWarehouse::Lut(LutDataType {
            num_input_channels: 3,
            num_output_channels: 4,
            num_clut_grid_points: 2,
            matrix: Matrix3d::IDENTITY,
            num_input_table_entries: 256,
            num_output_table_entries: 256,
            input_table: LutStore::Store16(
                (0..3).flat_map(|_| (0u16..=255).map(|v| v * 257)).collect(),
            ),
            clut_table: LutStore::Store16(
                (0..8).flat_map(|_| cmyk.map(|v| v as u16 * 257)).collect(),
            ),
            output_table: LutStore::Store16(
                (0..4).flat_map(|_| (0u16..=255).map(|v| v * 257)).collect(),
            ),
            lut_type: LutType::Lut16,
        }));
        p.encode().unwrap()
    }
    #[test]
    fn local_cmyk_profile_uses_selected_lut_without_fallback_or_source_writes() {
        let root = crate::test_support::TempDir::new();
        let first = fixture([51, 102, 153, 204]);
        let path = root.0.join("test-output.icc");
        fs::write(&path, &first).unwrap();
        let profile = OutputProfile::read(&path).unwrap();
        let out = profile.convert([128, 64, 32, 0]).unwrap();
        for (a, b) in out.into_iter().zip([20.0, 40.0, 60.0, 80.0]) {
            assert!((a - b).abs() < 0.5);
        }
        let second = OutputProfile::from_bytes(&fixture([25, 75, 125, 175])).unwrap();
        assert_ne!(second.convert([128, 64, 32, 255]).unwrap(), out);
        assert_eq!(fs::read(&path).unwrap(), first);
        assert!(OutputProfile::from_bytes(b"corrupt").is_err());
        assert!(
            OutputProfile::from_bytes(&moxcms::ColorProfile::new_srgb().encode().unwrap()).is_err()
        );
    }
}
