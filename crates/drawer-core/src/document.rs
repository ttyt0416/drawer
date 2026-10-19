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
    ViewLimit,
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "Width and height must be whole numbers from 1 to 32768.",
            Self::InvalidDpi => "DPI must be a positive finite number.",
            Self::ResourceLimit => "The canvas exceeds the available document memory budget.",
            Self::Allocation => "There is not enough memory to create the canvas.",
            Self::ViewLimit => "The workspace supports up to 256 views.",
        })
    }
}
impl std::error::Error for CreateError {}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterLayer {
    pub locked: bool,
    pub name: String,
    pub pixels: Vec<[u8; 4]>,
    pub placement: Option<PlacedRaster>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedRaster {
    pub width: u32,
    pub height: u32,
    pub offset: [i64; 2],
}
impl PlacedRaster {
    pub fn valid(self) -> bool {
        (1..=MAX_DIMENSION).contains(&self.width)
            && (1..=MAX_DIMENSION).contains(&self.height)
            && self
                .offset
                .iter()
                .all(|n| n.unsigned_abs() <= 1_000_000_000)
    }
}
impl RasterLayer {
    pub fn extent(&self, width: u32, height: u32) -> PlacedRaster {
        self.placement.unwrap_or(PlacedRaster {
            width,
            height,
            offset: [0; 2],
        })
    }
    pub fn valid(&self, width: u32, height: u32) -> bool {
        let p = self.extent(width, height);
        p.valid()
            && (1..=4096).contains(&self.name.len())
            && self.pixels.len() == p.width as usize * p.height as usize
    }
    pub fn pixel_index(&self, width: u32, height: u32, x: i64, y: i64) -> Option<usize> {
        let p = self.extent(width, height);
        let x = x.checked_sub(p.offset[0])?;
        let y = y.checked_sub(p.offset[1])?;
        if x < 0 || y < 0 || x >= p.width as i64 || y >= p.height as i64 {
            return None;
        }
        Some((y * p.width as i64 + x) as usize)
    }
    pub fn pixel(&self, width: u32, height: u32, x: i64, y: i64) -> [u8; 4] {
        self.pixel_index(width, height, x, y)
            .and_then(|i| self.pixels.get(i))
            .copied()
            .unwrap_or([0; 4])
    }
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
    pub scene: crate::scene::Scene,
    pub animation: Option<crate::animation::Timeline>,
    pub current_frame: u32,
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
                locked: false,
                name: "Layer 1".into(),
                pixels,
                placement: None,
            }],
            path: None,
            dirty: true,
            view: crate::view::View::default(),
            tool: crate::assets::BrushTip::Pen,
            revision: 0,
            scene: Default::default(),
            animation: None,
            current_frame: 0,
            history: Default::default(),
        })
    }
    pub fn memory_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.pixels.len() * 4)
            .sum::<usize>()
            + self.scene.size_bytes()
            + self.animation.as_ref().map_or(0, |a| a.memory_bytes())
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
        self.composite_pixel_with(x, y, |_, layer| {
            layer.pixel(self.width, self.height, x as i64, y as i64)
        })
    }
    pub(crate) fn composite_pixel_with(
        &self,
        x: u32,
        y: u32,
        mut pixel: impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) -> [u8; 4] {
        self.composite_pixel_filtered(x, y, true, &mut pixel)
    }
    pub(crate) fn composite_pixel_filtered(
        &self,
        x: u32,
        y: u32,
        animation: bool,
        mut pixel: impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) -> [u8; 4] {
        self.composite_pixel_graph(x, y, animation, &self.scene.paper, &mut pixel)
    }
    pub(crate) fn composite_pixel_graph(
        &self,
        x: u32,
        y: u32,
        animation: bool,
        graph: &crate::paper::Graph,
        mut pixel: impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) -> [u8; 4] {
        let mut color = graph.composite(self, x, y, &mut pixel);
        for cell in self
            .animation
            .iter()
            .filter(|_| animation)
            .flat_map(|a| a.visible_cells(self.current_frame))
        {
            color.add(cell.pixel(x as i64, y as i64));
        }
        color.rgba()
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
            locked: false,
            name: "Top".into(),
            pixels: vec![[0, 0, 255, 128]],
            placement: None,
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
