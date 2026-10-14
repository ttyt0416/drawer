use std::sync::atomic::{AtomicU64, Ordering};
use std::{fmt, path::PathBuf};

pub const MAX_DIMENSION: u32 = 32768;
pub const DEFAULT_DOCUMENT_BUDGET: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum Background {
    Transparent,
    Color([u8; 3]),
}

#[derive(Clone, Copy, Debug)]
pub struct CanvasSpec {
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub background: Background,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateError {
    InvalidDimensions,
    InvalidDpi,
    ResourceLimit,
    Allocation,
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "Width and height must be whole numbers from 1 to 32768.",
            Self::InvalidDpi => "DPI must be a positive finite number.",
            Self::ResourceLimit => "The canvas exceeds the available document memory budget.",
            Self::Allocation => "There is not enough memory to create the canvas.",
        })
    }
}
impl std::error::Error for CreateError {}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterLayer {
    pub name: String,
    pub pixels: Vec<[u8; 4]>,
}
pub struct Document {
    pub id: u64,
    pub title: String,
    pub source_path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub layers: Vec<RasterLayer>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub view: crate::view::View,
    pub tool: crate::assets::BrushTip,
    pub revision: u64,
    pub(crate) history: crate::history::History,
}

impl Document {
    pub fn create(spec: CanvasSpec, memory_budget: usize) -> Result<Self, CreateError> {
        if !(1..=MAX_DIMENSION).contains(&spec.width) || !(1..=MAX_DIMENSION).contains(&spec.height)
        {
            return Err(CreateError::InvalidDimensions);
        }
        if !spec.dpi.is_finite() || spec.dpi <= 0.0 {
            return Err(CreateError::InvalidDpi);
        }
        let count = (spec.width as usize)
            .checked_mul(spec.height as usize)
            .ok_or(CreateError::ResourceLimit)?;
        let bytes = count.checked_mul(4).ok_or(CreateError::ResourceLimit)?;
        if bytes > memory_budget {
            return Err(CreateError::ResourceLimit);
        }
        let color = match spec.background {
            Background::Transparent => [0; 4],
            Background::Color([r, g, b]) => [r, g, b, 255],
        };
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| CreateError::Allocation)?;
        pixels.resize(count, color);
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            id,
            title: format!("Untitled {id}"),
            source_path: None,
            width: spec.width,
            height: spec.height,
            dpi: spec.dpi,
            layers: vec![RasterLayer {
                name: "Layer 1".into(),
                pixels,
            }],
            path: None,
            dirty: true,
            view: crate::view::View::default(),
            tool: crate::assets::BrushTip::Pen,
            revision: 0,
            history: Default::default(),
        })
    }
    pub fn memory_bytes(&self) -> usize {
        self.layers.iter().map(|layer| layer.pixels.len() * 4).sum()
    }
    pub fn mark_saved(&mut self, path: PathBuf) {
        self.title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.path = Some(path);
        self.source_path = None;
        self.history.mark_saved();
        self.dirty = false;
    }
    pub fn composite_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = (y * self.width + x) as usize;
        let mut color = [0.0f32; 3];
        let mut alpha = 0.0;
        for layer in &self.layers {
            let pixel = layer.pixels[offset];
            let source_alpha = pixel[3] as f32 / 255.0;
            for channel in 0..3 {
                color[channel] =
                    pixel[channel] as f32 * source_alpha + color[channel] * (1.0 - source_alpha);
            }
            alpha = source_alpha + alpha * (1.0 - source_alpha);
        }
        if alpha == 0.0 {
            return [0; 4];
        }
        [
            (color[0] / alpha).round() as u8,
            (color[1] / alpha).round() as u8,
            (color[2] / alpha).round() as u8,
            (alpha * 255.0).round() as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> CanvasSpec {
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        }
    }
    #[test]
    fn transparent_and_opaque_start_with_one_unsaved_raster_layer() {
        let document = Document::create(spec(), 4).unwrap();
        assert_eq!(document.layers.len(), 1);
        assert_eq!(document.layers[0].pixels, [[0, 0, 0, 0]]);
        assert!(document.path.is_none() && document.dirty);
        let mut colored = spec();
        colored.width = 2;
        colored.height = 3;
        colored.background = Background::Color([20, 40, 60]);
        let document = Document::create(colored, 24).unwrap();
        assert_eq!(document.layers[0].pixels, vec![[20, 40, 60, 255]; 6]);
        assert_eq!(document.dpi, 300.0);
    }
    #[test]
    fn opened_layers_composite_in_order_with_straight_alpha() {
        let mut document = Document::create(spec(), 4).unwrap();
        document.layers[0].pixels[0] = [255, 0, 0, 128];
        document.layers.push(RasterLayer {
            name: "Top".into(),
            pixels: vec![[0, 0, 255, 128]],
        });
        assert_eq!(document.composite_pixel(0, 0), [85, 0, 170, 192]);
        document.layers[1].pixels[0] = [9, 10, 11, 255];
        assert_eq!(document.composite_pixel(0, 0), [9, 10, 11, 255]);
        for layer in &mut document.layers {
            layer.pixels[0] = [255, 40, 10, 0];
        }
        assert_eq!(document.composite_pixel(0, 0), [0; 4]);
    }
    #[test]
    fn rejects_bad_dimensions_dpi_and_budget_before_allocation() {
        for size in [0, 32769, u32::MAX] {
            let mut candidate = spec();
            candidate.width = size;
            assert!(matches!(
                Document::create(candidate, usize::MAX),
                Err(CreateError::InvalidDimensions)
            ));
        }
        for dpi in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut candidate = spec();
            candidate.dpi = dpi;
            assert!(matches!(
                Document::create(candidate, 4),
                Err(CreateError::InvalidDpi)
            ));
        }
        let candidate = CanvasSpec {
            width: 32768,
            height: 32768,
            ..spec()
        };
        assert!(matches!(
            Document::create(candidate, DEFAULT_DOCUMENT_BUDGET),
            Err(CreateError::ResourceLimit)
        ));
        assert!(matches!(
            Document::create(spec(), 3),
            Err(CreateError::ResourceLimit)
        ));
        let narrow = CanvasSpec {
            width: 32768,
            height: 1,
            ..spec()
        };
        assert_eq!(
            Document::create(narrow, 32768 * 4).unwrap().memory_bytes(),
            32768 * 4
        );
    }
}
