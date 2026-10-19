//! Frozen, four-connected closed-region masks. References are read-only.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::{Target, distance, point_valid},
    scene::Point,
};
pub struct Region {
    width: u32,
    height: u32,
    bits: Vec<u8>,
}
impl Region {
    pub fn bytes(&self) -> usize {
        self.bits.len()
    }
    pub fn contains(&self, p: Point) -> bool {
        p[0] >= 0.0
            && p[1] >= 0.0
            && p[0] < self.width as f64
            && p[1] < self.height as f64
            && self.bits[p[1] as usize * self.width as usize + p[0] as usize] == 2
    }
    pub fn build(doc: &Document, seed: Point) -> Result<Self, String> {
        if !point_valid(seed)
            || seed[0] < 0.0
            || seed[1] < 0.0
            || seed[0] >= doc.width as f64
            || seed[1] >= doc.height as f64
        {
            return Err("Choose a point inside the canvas".into());
        }
        let count = doc.width as usize * doc.height as usize;
        if count > 4 * 1024 * 1024
            || count.saturating_mul(5).saturating_add(doc.resident_bytes())
                > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Reference region memory limit (4M pixels)".into());
        }
        let mut bits = Vec::new();
        bits.try_reserve_exact(count)
            .map_err(|_| "Region allocation failed")?;
        bits.resize(count, 0);
        let mut work = 0usize;
        let mut used = false;
        let s = &doc.scene.lines.settings;
        for t in &s.references {
            match *t {
                Target::Raster(n) => {
                    let l = doc.layers.get(n).ok_or("Missing raster reference")?;
                    if !doc.scene.paper.raster_visible(n, doc.layers.len()) {
                        continue;
                    }
                    used = true;
                    for y in 0..doc.height {
                        for x in 0..doc.width {
                            work += 1;
                            if work > 64 * 1024 * 1024 {
                                return Err("Reference region work limit".into());
                            }
                            let p = l.pixel(doc.width, doc.height, x as i64, y as i64);
                            let lum =
                                (54 * p[0] as u32 + 183 * p[1] as u32 + 19 * p[2] as u32) / 256;
                            if (255 - lum) * p[3] as u32 / 255 >= s.darkness as u32 {
                                bits[y as usize * doc.width as usize + x as usize] = 1
                            }
                        }
                    }
                }
                Target::Vector(id) => {
                    let l = doc
                        .scene
                        .lines
                        .layers
                        .iter()
                        .find(|l| l.id == id)
                        .ok_or("Missing vector reference")?;
                    if l.hidden {
                        continue;
                    }
                    used = true;
                    for path in &l.paths {
                        if let Some(fill) = path.fill {
                            for y in 0..doc.height {
                                for x in 0..doc.width {
                                    work = work.saturating_add(path.points.len());
                                    if work > 64 * 1024 * 1024 {
                                        return Err("Reference region work limit".into());
                                    }
                                    let point = doc
                                        .scene
                                        .transform
                                        .inverse()
                                        .map([x as f64 + 0.5, y as f64 + 0.5]);
                                    if crate::shapes::contains(&path.points, point) {
                                        let p = fill;
                                        let lum = (54 * p[0] as u32
                                            + 183 * p[1] as u32
                                            + 19 * p[2] as u32)
                                            / 256;
                                        if (255 - lum) * p[3] as u32 / 255 >= s.darkness as u32 {
                                            bits[y as usize * doc.width as usize + x as usize] = 1;
                                        }
                                    }
                                }
                            }
                        }
                        if !path.stroke || path.color[3] == 0 {
                            continue;
                        }
                        for (a, b) in path.segments() {
                            let a = doc.scene.transform.map(a);
                            let b = doc.scene.transform.map(b);
                            let scale = crate::line_tools::maximum_scale(doc.scene.transform);
                            // At least half the diagonal closes subpixel lines on a four-connected grid.
                            let r = (path.width * scale / 2.0).max(std::f64::consts::FRAC_1_SQRT_2);
                            let lo = [
                                (a[0].min(b[0]) - r).floor().max(0.0) as u32,
                                (a[1].min(b[1]) - r).floor().max(0.0) as u32,
                            ];
                            let hi = [
                                (a[0].max(b[0]) + r).ceil().clamp(0.0, doc.width as f64) as u32,
                                (a[1].max(b[1]) + r).ceil().clamp(0.0, doc.height as f64) as u32,
                            ];
                            for y in lo[1]..hi[1] {
                                for x in lo[0]..hi[0] {
                                    work += 1;
                                    if work > 64 * 1024 * 1024 {
                                        return Err("Reference region work limit".into());
                                    }
                                    if distance([x as f64 + 0.5, y as f64 + 0.5], a, b).0 <= r {
                                        bits[y as usize * doc.width as usize + x as usize] = 1
                                    }
                                }
                            }
                        }
                    }
                }
                _ => return Err("Unsupported boundary reference".into()),
            }
        }
        if !used {
            return Err("Choose at least one visible line reference".into());
        }
        let start = seed[1] as usize * doc.width as usize + seed[0] as usize;
        if bits[start] != 0 {
            return Err("The starting point is on a reference line".into());
        }
        let mut queue = Vec::<u32>::new();
        queue
            .try_reserve_exact(count)
            .map_err(|_| "Region queue allocation failed")?;
        queue.push(start as u32);
        bits[start] = 2;
        let mut n = 0;
        while n < queue.len() {
            let p = queue[n] as usize;
            n += 1;
            let x = p % doc.width as usize;
            let y = p / doc.width as usize;
            if x == 0 || y == 0 || x + 1 == doc.width as usize || y + 1 == doc.height as usize {
                return Err("Reference boundary is open; no pixels changed".into());
            }
            for q in [p - 1, p + 1, p - doc.width as usize, p + doc.width as usize] {
                if bits[q] == 0 {
                    bits[q] = 2;
                    queue.push(q as u32)
                }
            }
        }
        Ok(Self {
            width: doc.width,
            height: doc.height,
            bits,
        })
    }
}
pub fn fill(doc: &mut Document, layer: usize, seed: Point, color: [u8; 4]) -> Result<bool, String> {
    let region = Region::build(doc, seed)?;
    let raster = doc.layers.get(layer).ok_or("Missing fill target")?;
    if raster.locked || !doc.scene.paper.raster_visible(layer, doc.layers.len()) {
        return Err("Fill target is hidden or locked".into());
    }
    let mut changes = vec![];
    let limit = (DEFAULT_DOCUMENT_BUDGET / 4)
        .min(DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes() + region.bytes()));
    for y in 0..doc.height {
        for x in 0..doc.width {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            if !region.contains(p) {
                continue;
            }
            let mask = doc.scene.selection_coverage(p) as f64 / 255.0;
            if mask == 0.0 {
                continue;
            }
            if let Some(offset) = raster.pixel_index(doc.width, doc.height, x as i64, y as i64) {
                let before = raster.pixels[offset];
                let sa = color[3] as f64 / 255.0 * mask;
                let da = before[3] as f64 / 255.0;
                let a = sa + da * (1.0 - sa);
                let mut after = [0; 4];
                if a > 0.0 {
                    for c in 0..3 {
                        after[c] = ((color[c] as f64 * sa + before[c] as f64 * da * (1.0 - sa)) / a)
                            .round() as u8;
                    }
                    after[3] = (a * 255.0).round() as u8;
                }
                if after != before {
                    if (changes.len() + 1) * std::mem::size_of::<crate::history::PixelChange>()
                        > limit
                    {
                        return Err("Fill undo memory limit; no pixels changed".into());
                    }
                    changes.push(crate::history::PixelChange {
                        offset,
                        before,
                        after,
                    })
                }
            }
        }
    }
    doc.commit_multi(vec![(layer, changes)], None)
}
