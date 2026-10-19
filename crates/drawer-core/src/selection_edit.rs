//! Coverage-based wand and selection brush. Painting layers remain read-only.
use crate::{
    document::Document,
    scene::{Point, Scene},
    selection::{Mask, Run},
    selection_tools::{self, Shape},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Combine {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}
impl Combine {
    pub const ALL: [Self; 4] = [Self::Replace, Self::Add, Self::Subtract, Self::Intersect];
    pub fn coverage(self, a: u8, b: u8) -> u8 {
        let a = a as u32;
        let b = b as u32;
        (match self {
            Self::Replace => b,
            Self::Add => a + b - (a * b + 127) / 255,
            Self::Subtract => (a * (255 - b) + 127) / 255,
            Self::Intersect => (a * b + 127) / 255,
        }) as u8
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub region: crate::bucket::Settings,
    pub connected: bool,
    pub combine: Combine,
    pub diameter: f64,
    pub hardness: f64,
    pub strength: f64,
    pub erase: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            region: Default::default(),
            connected: true,
            combine: Combine::Replace,
            diameter: 24.0,
            hardness: 0.75,
            strength: 1.0,
            erase: false,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.region.valid()
            && self.diameter.is_finite()
            && (0.5..=4096.0).contains(&self.diameter)
            && [self.hardness, self.strength]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
}
pub(crate) struct Grid {
    origin: [i32; 2],
    pub(crate) size: [usize; 2],
}
impl Grid {
    pub(crate) fn new(doc: &Document) -> Result<Self, String> {
        Self::with_bounds(doc, &[])
    }
    pub(crate) fn with_bounds(doc: &Document, extra: &[Point]) -> Result<Self, String> {
        let inv = doc.scene.transform.inverse();
        let points = [
            [0.0; 2],
            [doc.width as f64, 0.0],
            [doc.width as f64, doc.height as f64],
            [0.0, doc.height as f64],
        ]
        .into_iter()
        .chain(extra.iter().copied())
        .map(|p| inv.map(p));
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in points {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        // Preserve an existing moved selection outside the canvas when composing.
        let shifted = inv.map(doc.scene.selection_translation);
        let zero = inv.map([0.0; 2]);
        let delta = [shifted[0] - zero[0], shifted[1] - zero[1]];
        let mut add_bounds = |a: Point, b: Point| {
            for i in 0..2 {
                min[i] = min[i].min(a[i] + delta[i]);
                max[i] = max[i].max(b[i] + delta[i]);
            }
        };
        if let Some(mask) = &doc.scene.selection_mask {
            for r in &mask.runs {
                add_bounds(
                    [r.x as f64, r.y as f64],
                    [r.x as f64 + r.length as f64, r.y as f64 + 1.0],
                );
            }
        } else if let Some(rect) = doc.scene.selection {
            add_bounds(rect.min, rect.max);
        }
        min = min.map(f64::floor);
        max = max.map(f64::ceil);
        if min
            .iter()
            .chain(&max)
            .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
        {
            return Err("Selection page coordinates exceed limit".into());
        }
        let size = [(max[0] - min[0]) as usize, (max[1] - min[1]) as usize];
        if size.iter().any(|n| *n == 0 || *n > 32768)
            || size[0].saturating_mul(size[1]) > 4 * 1024 * 1024
        {
            return Err("Selection rasterization exceeds work limit".into());
        }
        Ok(Self {
            origin: min.map(|v| v as i32),
            size,
        })
    }
    pub(crate) fn sample(
        &self,
        doc: &Document,
        clip: bool,
        mut coverage: impl FnMut(Point) -> u8,
    ) -> Vec<u8> {
        let mut values = vec![0u8; self.size[0] * self.size[1]];
        for y in 0..self.size[1] {
            for x in 0..self.size[0] {
                let mut sum = 0u32;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let p = doc.scene.transform.map([
                            self.origin[0] as f64 + x as f64 + (sx as f64 + 0.5) / 4.0,
                            self.origin[1] as f64 + y as f64 + (sy as f64 + 0.5) / 4.0,
                        ]);
                        if !clip
                            || (p[0] >= 0.0
                                && p[1] >= 0.0
                                && p[0] < doc.width as f64
                                && p[1] < doc.height as f64)
                        {
                            sum += coverage(p) as u32;
                        }
                    }
                }
                values[y * self.size[0] + x] = ((sum + 8) / 16) as u8;
            }
        }
        values
    }
    pub(crate) fn base(&self, doc: &Document, add: bool) -> Vec<u8> {
        if add && doc.scene.selection.is_none() && doc.scene.selection_mask.is_none() {
            vec![0; self.size[0] * self.size[1]]
        } else {
            self.sample(
                doc,
                doc.scene.selection.is_none() && doc.scene.selection_mask.is_none(),
                |p| doc.scene.selection_coverage(p),
            )
        }
    }
    pub(crate) fn scene(&self, doc: &Document, values: &[u8]) -> Result<Scene, String> {
        let mut mask = Mask::default();
        for y in 0..self.size[1] {
            let mut x = 0;
            while x < self.size[0] {
                let a = x;
                let alpha = values[y * self.size[0] + x];
                x += 1;
                while x < self.size[0] && values[y * self.size[0] + x] == alpha {
                    x += 1;
                }
                if alpha > 0 {
                    if mask.runs.len() == 32768 {
                        return Err("Selection run limit".into());
                    }
                    mask.runs.push(Run {
                        x: self.origin[0] + a as i32,
                        y: self.origin[1] + y as i32,
                        length: (x - a) as u32,
                        alpha,
                    });
                }
            }
        }
        selection_tools::prepare_shape(
            doc,
            Shape {
                rect: None,
                mask: Some(mask),
                translation: [0.0; 2],
            },
        )
    }
}
pub fn wand(doc: &Document, layer: usize, seed: Point, s: Settings) -> Result<Scene, String> {
    if !s.valid() {
        return Err("Invalid wand settings".into());
    }
    let region =
        crate::bucket::Region::build_with_connectivity(doc, layer, seed, s.region, s.connected)?;
    let grid = Grid::new(doc)?;
    let incoming = grid.sample(doc, true, |p| {
        region.coverage(p[0].floor() as u32, p[1].floor() as u32)
    });
    let base = grid.base(doc, s.combine == Combine::Add);
    let values = base
        .into_iter()
        .zip(incoming)
        .map(|(a, b)| s.combine.coverage(a, b))
        .collect::<Vec<_>>();
    grid.scene(doc, &values)
}
pub struct Brush {
    page: String,
    document: u64,
    revision: u64,
    grid: Grid,
    base: Vec<u8>,
    paint: Vec<u8>,
    settings: Settings,
    last: Option<(Point, f64)>,
    work: usize,
    version: u64,
}
impl Brush {
    pub fn begin(doc: &Document, s: Settings) -> Result<Self, String> {
        if !s.valid() {
            return Err("Invalid selection brush settings".into());
        }
        let grid = Grid::new(doc)?;
        let base = grid.base(doc, !s.erase);
        let paint = vec![0; base.len()];
        Ok(Self {
            page: doc
                .scene
                .selections
                .page
                .clone()
                .unwrap_or_else(selection_tools::page_id),
            document: doc.id,
            revision: doc.revision,
            grid,
            base,
            paint,
            settings: s,
            last: None,
            work: 0,
            version: 0,
        })
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn sample(&mut self, doc: &Document, point: Point, pressure: f64) -> Result<(), String> {
        if self.document != doc.id || self.revision != doc.revision {
            return Err("Selection brush source changed".into());
        }
        if point.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || !pressure.is_finite()
            || !(0.0..=1.0).contains(&pressure)
        {
            return Err("Invalid selection brush sample".into());
        }
        let (a, pa) = self.last.unwrap_or((point, pressure));
        let b = point;
        let radius = self.settings.diameter / 2.0;
        let inv = doc.scene.transform.inverse();
        let lo = [a[0].min(b[0]) - radius, a[1].min(b[1]) - radius];
        let hi = [a[0].max(b[0]) + radius, a[1].max(b[1]) + radius];
        let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(|p| inv.map(p));
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in corners {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let start = [
            (min[0].floor() - self.grid.origin[0] as f64).clamp(0.0, self.grid.size[0] as f64)
                as usize,
            (min[1].floor() - self.grid.origin[1] as f64).clamp(0.0, self.grid.size[1] as f64)
                as usize,
        ];
        let end = [
            (max[0].ceil() - self.grid.origin[0] as f64).clamp(0.0, self.grid.size[0] as f64)
                as usize,
            (max[1].ceil() - self.grid.origin[1] as f64).clamp(0.0, self.grid.size[1] as f64)
                as usize,
        ];
        self.work = self.work.saturating_add(
            (end[0] - start[0])
                .saturating_mul(end[1] - start[1])
                .saturating_mul(16),
        );
        if self.work > 64 * 1024 * 1024 {
            return Err("Selection brush work limit".into());
        }
        let vector = [b[0] - a[0], b[1] - a[1]];
        let length = vector[0] * vector[0] + vector[1] * vector[1];
        for y in start[1]..end[1] {
            for x in start[0]..end[0] {
                let mut amount = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let p = doc.scene.transform.map([
                            self.grid.origin[0] as f64 + x as f64 + (sx as f64 + 0.5) / 4.0,
                            self.grid.origin[1] as f64 + y as f64 + (sy as f64 + 0.5) / 4.0,
                        ]);
                        if p[0] < 0.0
                            || p[1] < 0.0
                            || p[0] >= doc.width as f64
                            || p[1] >= doc.height as f64
                        {
                            continue;
                        }
                        let t = if length == 0.0 {
                            1.0
                        } else {
                            (((p[0] - a[0]) * vector[0] + (p[1] - a[1]) * vector[1]) / length)
                                .clamp(0.0, 1.0)
                        };
                        let distance = (p[0] - a[0] - t * vector[0])
                            .hypot(p[1] - a[1] - t * vector[1])
                            / radius;
                        let shape = if distance <= self.settings.hardness {
                            1.0
                        } else if distance >= 1.0 {
                            0.0
                        } else {
                            (1.0 - distance) / (1.0 - self.settings.hardness)
                        };
                        amount += shape * self.settings.strength * (pa + (pressure - pa) * t);
                    }
                }
                let i = y * self.grid.size[0] + x;
                self.paint[i] = self.paint[i].max((amount * 255.0 / 16.0).round() as u8);
            }
        }
        self.last = Some((point, pressure));
        self.version += 1;
        Ok(())
    }
    pub fn preview(&self, doc: &Document) -> Result<Scene, String> {
        if self.document != doc.id || self.revision != doc.revision {
            return Err("Selection brush source changed".into());
        }
        if self.paint.iter().all(|a| *a == 0) {
            return Ok(doc.scene.clone());
        }
        let operation = if self.settings.erase {
            Combine::Subtract
        } else {
            Combine::Add
        };
        let values = self
            .base
            .iter()
            .zip(&self.paint)
            .map(|(a, b)| operation.coverage(*a, *b))
            .collect::<Vec<_>>();
        let mut scene = self.grid.scene(doc, &values)?;
        scene.selections.page = Some(self.page.clone());
        Ok(scene)
    }
}

pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        scene::Rect,
    };
    if Combine::Add.coverage(128, 128) != 192
        || Combine::Subtract.coverage(128, 128) != 64
        || Combine::Intersect.coverage(128, 128) != 64
    {
        return Err("Selection composition integer oracle".into());
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 5,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        },
        1000,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels = vec![
        [200, 0, 0, 255],
        [0, 0, 0, 255],
        [200, 0, 0, 255],
        [0, 0, 0, 255],
        [200, 0, 0, 255],
    ];
    let before = d.snapshot();
    let local = wand(&d, 0, [0.5, 0.5], Settings::default())?;
    let global = wand(
        &d,
        0,
        [0.5, 0.5],
        Settings {
            connected: false,
            ..Default::default()
        },
    )?;
    if local.selection_coverage([0.5, 0.5]) != 255
        || local.selection_coverage([2.5, 0.5]) != 0
        || global.selection_coverage([2.5, 0.5]) != 255
        || global.selection_coverage([1.5, 0.5]) != 0
        || d.snapshot() != before
    {
        return Err("Wand connected/global distinction or source mutation".into());
    }
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 2,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    let add = wand(
        &d,
        0,
        [0.5, 0.5],
        Settings {
            combine: Combine::Add,
            ..Default::default()
        },
    )?;
    if add.selection_coverage([2.5, 0.5]) != 128 || add.selection_coverage([0.5, 0.5]) != 255 {
        return Err("Wand old selection composition".into());
    }
    let intersect = wand(
        &d,
        0,
        [0.5, 0.5],
        Settings {
            combine: Combine::Intersect,
            connected: false,
            ..Default::default()
        },
    )?;
    if intersect.selection_coverage([2.5, 0.5]) != 128
        || intersect.selection_coverage([0.5, 0.5]) != 0
    {
        return Err("Wand intersection".into());
    }
    let mut edge = Document::create(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        },
        1000,
    )
    .map_err(|e| e.to_string())?;
    edge.layers[0].pixels = vec![[50, 60, 70, 0], [0, 0, 0, 128], [0, 0, 0, 255]];
    if wand(&edge, 0, [0.5, 0.5], Settings::default())?.selection_coverage([1.5, 0.5]) != 127 {
        return Err("Wand partial-alpha edge coverage".into());
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        10000,
    )
    .map_err(|e| e.to_string())?;
    let original = d.snapshot();
    let s = Settings {
        diameter: 8.0,
        hardness: 1.0,
        strength: 0.5,
        ..Default::default()
    };
    let mut brush = Brush::begin(&d, s)?;
    brush.sample(&d, [8.0; 2], 1.0)?;
    let first = brush.preview(&d)?;
    brush.sample(&d, [8.0; 2], 1.0)?;
    let repeated = brush.preview(&d)?;
    if first != repeated
        || first.selection_coverage([7.5; 2]) != 128
        || first.selection_coverage([4.5, 6.5]) != 88
        || first.selection_coverage([0.5; 2]) != 0
        || d.snapshot() != original
    {
        return Err("Selection brush strength/nonaccumulation or source mutation".into());
    }
    d.edit_scene(first)?;
    if d.layers != original.layers || d.history_counts() != (1, 0) {
        return Err("Selection brush changed painting pixels/history".into());
    }
    let mut brush = Brush::begin(&d, s)?;
    brush.sample(&d, [8.0; 2], 1.0)?;
    let add = brush.preview(&d)?;
    if add.selection_coverage([7.5; 2]) != 192 {
        return Err("New selection stroke failed to add strength".into());
    }
    let mut erase = Brush::begin(&d, Settings { erase: true, ..s })?;
    erase.sample(&d, [8.0; 2], 1.0)?;
    let erased = erase.preview(&d)?;
    if erased.selection_coverage([7.5; 2]) != 64 {
        return Err("Selection brush erasure".into());
    }
    let mut pressure = Brush::begin(&d, Settings { strength: 1.0, ..s })?;
    pressure.sample(&d, [8.0; 2], 0.5)?;
    if pressure.preview(&d)? != add {
        return Err("Selection brush pressure strength".into());
    }
    let mut empty = Brush::begin(&d, Settings { strength: 0.0, ..s })?;
    empty.sample(&d, [8.0; 2], 1.0)?;
    if empty.preview(&d)? != d.scene {
        return Err("Zero strength changed selection".into());
    }
    let path = root.join("selection-brush.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Selection tools native reopen".into());
    }
    d.undo();
    if d.snapshot() != original {
        return Err("Selection brush Undo".into());
    }
    d.redo();
    if d.scene.selection_coverage([7.5; 2]) != 128 {
        return Err("Selection brush Redo".into());
    }
    d.scene.transform = crate::scene::Affine([0.0, 1.0, -1.0, 0.0, 16.0, 0.0]);
    d.scene.selection_mask = None;
    d.scene.selection = Some(Rect {
        min: [4.0, 4.0],
        max: [8.0, 8.0],
    });
    d.scene.selection_translation = [0.5, 0.0];
    let before = d.snapshot();
    let mut brush = Brush::begin(&d, s)?;
    brush.sample(&d, [7.0, 6.0], 1.0)?;
    if brush.preview(&d)?.selection_coverage([7.5, 6.5]) == 0 || d.snapshot() != before {
        return Err("Transformed/translated selection brush".into());
    }
    d.scene.transform = Default::default();
    d.scene.selection = None;
    d.scene.selection_translation = [0.0; 2];
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: -2,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    let mut outside = Brush::begin(&d, s)?;
    outside.sample(&d, [8.0; 2], 1.0)?;
    if outside.preview(&d)?.selection_coverage([-1.5, 0.5]) != 128 {
        return Err("Selection brush discarded moved offcanvas selection".into());
    }
    if brush.sample(&d, [f64::NAN, 0.0], 1.0).is_ok()
        || Brush::begin(
            &d,
            Settings {
                diameter: f64::INFINITY,
                ..s
            },
        )
        .is_ok()
    {
        return Err("Invalid brush accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn connected_global_composition_strength_pressure_transforms_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
