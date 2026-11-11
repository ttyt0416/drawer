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
    LayerDefaults(String),
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "Width and height must be whole numbers from 1 to 32768.",
            Self::InvalidDpi => "DPI must be a positive finite number.",
            Self::ResourceLimit => "The canvas exceeds the available document memory budget.",
            Self::Allocation => "There is not enough memory to create the canvas.",
            Self::ViewLimit => "The workspace supports up to 256 views.",
            Self::LayerDefaults(message) => message,
        })
    }
}
impl std::error::Error for CreateError {}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterLayer {
    pub tiled: Option<crate::tiled_raster::Image>,
    pub precise: Option<Vec<[u16; 4]>>,
    pub locked: bool,
    pub name: String,
    pub pixels: Vec<[u8; 4]>,
    pub placement: Option<PlacedRaster>,
}
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub(crate) fn prepare_pixels(
        &self,
        changes: impl Iterator<Item = (usize, [u8; 4])> + Clone,
        budget: usize,
    ) -> Result<Option<crate::tiled_raster::Image>, String> {
        let Some(source) = &self.tiled else {
            return Ok(None);
        };
        let mut image = source.clone();
        image.prepare(changes.clone().map(|(n, _)| n), budget)?;
        for (n, p) in changes {
            image.set_prepared(n, p);
        }
        image.compact();
        Ok(Some(image))
    }
    pub fn allocation_bytes(&self) -> usize {
        self.tiled.as_ref().map_or_else(
            || {
                self.pixels.capacity().saturating_mul(4)
                    + self
                        .precise
                        .as_ref()
                        .map_or(0, |v| v.capacity().saturating_mul(8))
            },
            crate::tiled_raster::Image::bytes,
        )
    }
    pub fn read_at(&self, n: usize) -> Option<[u8; 4]> {
        self.tiled
            .as_ref()
            .map_or_else(|| self.pixels.get(n).copied(), |t| t.get(n))
    }
    pub fn memory_bytes(&self) -> usize {
        if let Some(t) = &self.tiled {
            return t.width as usize * t.height as usize * 4;
        }
        self.pixels.len().saturating_mul(4)
            + self
                .precise
                .as_ref()
                .map_or(0, |v| v.len().saturating_mul(8))
    }
    pub fn sample(
        &self,
        width: u32,
        height: u32,
        x: i64,
        y: i64,
    ) -> crate::color_precision::Sample {
        self.pixel_index(width, height, x, y)
            .map_or_else(Default::default, |n| self.sample_at(n))
    }
    pub fn sample_at(&self, n: usize) -> crate::color_precision::Sample {
        if let Some(t) = &self.tiled {
            return crate::color_precision::Sample::from8(t.get(n).unwrap_or([0; 4]));
        }
        self.precise
            .as_ref()
            .and_then(|v| v.get(n))
            .copied()
            .map(crate::color_precision::Sample::from16)
            .unwrap_or_else(|| {
                crate::color_precision::Sample::from8(self.pixels.get(n).copied().unwrap_or([0; 4]))
            })
    }
    pub fn same_color(&self, other: &Self) -> bool {
        if self.tiled.is_some() || other.tiled.is_some() {
            return self.tiled == other.tiled
                && self.pixels == other.pixels
                && self.precise == other.precise;
        }
        if self.pixels.len() != other.pixels.len() {
            return false;
        }
        match (&self.precise, &other.precise) {
            (None, None) => self.pixels == other.pixels,
            (Some(a), Some(b)) => a == b,
            (Some(a), None) => a
                .iter()
                .zip(&other.pixels)
                .all(|(a, b)| *a == b.map(|v| v as u16 * 257)),
            (None, Some(b)) => self
                .pixels
                .iter()
                .zip(b)
                .all(|(a, b)| a.map(|v| v as u16 * 257) == *b),
        }
    }
    pub fn color_valid(&self) -> bool {
        self.precise.as_ref().is_none_or(|v| {
            v.len() == self.pixels.len()
                && v.iter().zip(&self.pixels).all(|(p, preview)| {
                    (p[3] != 0 || p[..3] == [0; 3])
                        && crate::color_precision::Sample::from16(*p).rgba8() == *preview
                })
        })
    }

    pub fn extent(&self, width: u32, height: u32) -> PlacedRaster {
        self.placement.unwrap_or(PlacedRaster {
            width,
            height,
            offset: [0; 2],
        })
    }
    pub fn valid(&self, width: u32, height: u32) -> bool {
        if let Some(t) = &self.tiled {
            return t.width == width
                && t.height == height
                && t.valid()
                && self.pixels.is_empty()
                && self.precise.is_none()
                && self.placement.is_none()
                && !self.name.is_empty()
                && self.name.len() <= 4096;
        }
        let p = self.extent(width, height);
        p.valid()
            && (1..=4096).contains(&self.name.len())
            && self.pixels.len() == p.width as usize * p.height as usize
            && self
                .precise
                .as_ref()
                .is_none_or(|v| v.len() == self.pixels.len())
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
            .and_then(|i| self.read_at(i))
            .unwrap_or([0; 4])
    }
}
#[derive(Clone)]
pub struct Document {
    pub(crate) working_budget: usize,
    pub(crate) raster_budget: usize,
    pub(crate) resource_error: Option<String>,
    pub work_time: Option<crate::work_time::Snapshot>,
    pub id: u64,
    pub color: crate::color_storage::Config,
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
    pub camera_guides: crate::camera2d::Guides,
    pub onion: crate::timeline_view::Onion,
    pub light_table: crate::light_table::Session,
    pub keyframe_content: Option<crate::keyframes::Owner>,
    pub(crate) history: crate::history::History,
}

impl Document {
    /// Freeze editable current state without cloning session Undo/Redo caches.
    pub fn current_copy(&self) -> Self {
        Self {
            working_budget: self.working_budget,
            raster_budget: self.raster_budget,
            resource_error: None,
            work_time: self.work_time.clone(),
            id: self.id,
            color: self.color.clone(),
            title: self.title.clone(),
            source_path: self.source_path.clone(),
            width: self.width,
            height: self.height,
            dpi: self.dpi,
            layers: self.layers.clone(),
            path: self.path.clone(),
            dirty: self.dirty,
            view: self.view,
            tool: self.tool,
            revision: self.revision,
            scene: self.scene.clone(),
            animation: self.animation.clone(),
            current_frame: self.current_frame,
            camera_guides: self.camera_guides,
            onion: self.onion,
            light_table: self.light_table.clone(),
            keyframe_content: self.keyframe_content,
            history: Default::default(),
        }
    }
    pub fn mark_recovered(&mut self, title: String) {
        self.path = None;
        self.source_path = None;
        self.title = title;
        self.history = Default::default();
        self.dirty = true;
    }
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
        let sparse = bytes > DEFAULT_DOCUMENT_BUDGET && memory_budget >= 65536;
        if bytes > memory_budget && !sparse {
            return Err(CreateError::ResourceLimit);
        }
        let color = match spec.background {
            Background::Transparent => [0; 4],
            Background::Color([r, g, b]) => [r, g, b, 255],
        };
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(if sparse { 0 } else { count })
            .map_err(|_| CreateError::Allocation)?;
        pixels.resize(if sparse { 0 } else { count }, color);
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            working_budget: DEFAULT_DOCUMENT_BUDGET,
            raster_budget: memory_budget,
            resource_error: None,
            work_time: None,
            id,
            color: crate::color_storage::Config::legacy(),
            title: format!("Untitled {id}"),
            source_path: None,
            width: spec.width,
            height: spec.height,
            dpi: spec.dpi,
            layers: vec![RasterLayer {
                tiled: sparse
                    .then(|| crate::tiled_raster::Image::new(spec.width, spec.height, color)),
                precise: None,
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
            camera_guides: Default::default(),
            onion: Default::default(),
            light_table: Default::default(),
            keyframe_content: None,
            history: Default::default(),
        })
    }
    pub(crate) fn tile_budget(&self, layer: usize) -> usize {
        let other = crate::tiled_raster::allocated_layers(&self.layers)
            .saturating_sub(self.layers[layer].allocation_bytes());
        self.raster_budget.saturating_sub(other).min(
            self.working_budget
                .saturating_sub(self.resident_bytes())
                .saturating_add(self.layers[layer].allocation_bytes()),
        )
    }
    pub fn image_budget(&self) -> usize {
        self.raster_budget
    }
    pub fn set_working_budget(&mut self, bytes: usize) {
        self.working_budget = bytes.min(DEFAULT_DOCUMENT_BUDGET);
        self.trim_history_memory();
    }
    pub(crate) fn snapshot_bytes(&self) -> usize {
        crate::tiled_raster::clone_layers_bytes(&self.layers)
            + self.color.memory_bytes()
            + self.scene.size_bytes()
            + self.animation.as_ref().map_or(0, |a| a.memory_bytes())
    }
    pub fn allocation_bytes(&self) -> usize {
        crate::tiled_raster::allocated_layers(&self.layers)
            + self.color.memory_bytes()
            + self.light_table.bytes()
            + self.scene.size_bytes()
            + self.animation.as_ref().map_or(0, |a| a.memory_bytes())
    }
    pub fn memory_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(RasterLayer::memory_bytes)
            .sum::<usize>()
            + self.color.memory_bytes()
            + self.light_table.bytes()
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
        self.light_table.mark_saved();
        self.dirty = false;
    }
    pub fn composite_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if !self.scene.keyframes.is_default() {
            return crate::render::Renderer::new(self).pixel([x as f64 + 0.5, y as f64 + 0.5]);
        }
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
        for (track, cell_index, cell) in self
            .animation
            .iter()
            .filter(|_| animation)
            .flat_map(|a| a.indexed_visible_cells(self.current_frame))
        {
            let weight = crate::layer_properties::factor(
                self,
                crate::layer_properties::Target::Track(track as u16),
            ) * if self
                .scene
                .file_objects
                .get(crate::layer_search::Key::Track(
                    self.scene.catalog.tracks.id(track),
                ))
                .is_some()
            {
                1.
            } else {
                crate::layer_properties::factor(
                    self,
                    crate::layer_properties::Target::Cell {
                        track: track as u16,
                        cell: cell_index as u16,
                    },
                )
            };
            let track_id = self.scene.catalog.tracks.id(track);
            let original = self
                .scene
                .transform
                .inverse()
                .map([x as f64 + 0.5, y as f64 + 0.5]);
            let pixel = self
                .scene
                .file_objects
                .pixel(crate::layer_search::Key::Track(track_id), original)
                .or_else(|| {
                    self.scene.file_objects.pixel(
                        crate::layer_search::Key::Cell {
                            track: track_id,
                            cell: self.scene.catalog.cell_id(track_id, cell_index),
                        },
                        original,
                    )
                })
                .unwrap_or_else(|| cell.pixel(x as i64, y as i64));
            let blend = crate::layer_blend::mode(
                self,
                crate::layer_properties::Target::Track(track as u16),
            );
            if !blend.is_normal() {
                let mut top = crate::paper::Color::default();
                top.add(pixel);
                top.rgb.iter_mut().for_each(|c| *c *= weight as f32);
                top.alpha *= weight as f32;
                blend.over(&mut color, top);
            } else if weight == 1.0 {
                color.add(pixel);
            } else {
                let mut top = crate::paper::Color::default();
                top.add(pixel);
                color.mix_layer(top, weight as f32);
            }
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
            tiled: None,
            precise: None,
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
            Document::create(candidate, 65535),
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
