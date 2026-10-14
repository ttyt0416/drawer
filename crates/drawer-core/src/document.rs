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

pub struct RasterLayer {
    pub name: String,
    pub pixels: Vec<[u8; 4]>,
}
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub layers: Vec<RasterLayer>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
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
        Ok(Self {
            width: spec.width,
            height: spec.height,
            dpi: spec.dpi,
            layers: vec![RasterLayer {
                name: "Layer 1".into(),
                pixels,
            }],
            path: None,
            dirty: true,
        })
    }
    pub fn memory_bytes(&self) -> usize {
        self.layers.iter().map(|layer| layer.pixels.len() * 4).sum()
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
