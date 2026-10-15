//! A pending raster stroke is previewed separately, then committed as one pixel history entry.
use crate::{
    assets::BrushTip,
    brush_cursor::{Shape, Trace},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, RasterLayer},
    history::PixelChange,
    scene::Point,
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_STROKE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub shape: Shape,
    pub tip: BrushTip,
    pub color: [u8; 4],
    pub smoothing: f64,
}
impl Style {
    fn valid(self) -> bool {
        self.shape.valid()
            && self.shape.angle.abs() <= 1e12
            && (0.0..=0.95).contains(&self.smoothing)
    }
}
pub struct Stroke {
    pub document: u64,
    pub layer: usize,
    revision: u64,
    pub preview_revision: u64,
    nonce: u64,
    style: Style,
    trace: Trace,
    last: Option<Point>,
    last_stamp: Option<Point>,
    remainder: f64,
    changes: Vec<PixelChange>,
    unmasked: Vec<[u8; 4]>,
    lookup: HashMap<usize, usize>,
    limit: usize,
}
impl Stroke {
    pub fn begin(doc: &Document, layer: usize, style: Style) -> Result<Self, String> {
        Self::with_budget(doc, layer, style, DEFAULT_DOCUMENT_BUDGET / 4)
    }
    pub fn with_budget(
        doc: &Document,
        layer: usize,
        style: Style,
        budget: usize,
    ) -> Result<Self, String> {
        if !style.valid()
            || doc
                .layers
                .get(layer)
                .is_none_or(|l| !l.valid(doc.width, doc.height))
        {
            return Err("Choose a valid raster layer and brush".into());
        }
        let limit = budget
            .min(DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes()))
            .saturating_sub(256)
            / 80;
        if limit == 0 {
            return Err("No memory available for a stroke".into());
        }
        Ok(Self {
            document: doc.id,
            layer,
            revision: doc.revision,
            preview_revision: 0,
            nonce: NEXT_STROKE.fetch_add(1, Ordering::Relaxed),
            style,
            trace: Default::default(),
            last: None,
            last_stamp: None,
            remainder: 0.0,
            changes: vec![],
            unmasked: vec![],
            lookup: HashMap::new(),
            limit,
        })
    }
    fn check(&self, doc: &Document) -> Result<(), String> {
        if doc.id != self.document
            || doc.revision != self.revision
            || doc
                .layers
                .get(self.layer)
                .is_none_or(|l| !l.valid(doc.width, doc.height))
        {
            return Err("The drawing changed; pending stroke discarded".into());
        }
        Ok(())
    }
    pub fn sample(&mut self, doc: &Document, point: Point) -> Result<(), String> {
        self.check(doc)?;
        if !point.iter().all(|n| n.is_finite() && n.abs() <= 1e12) {
            return Err("Invalid drawing coordinate".into());
        }
        let Some((_, to)) = self.trace.receive(point, self.style.smoothing) else {
            return Ok(());
        };
        if let Some(from) = self.last {
            let delta = [to[0] - from[0], to[1] - from[1]];
            let length = delta[0].hypot(delta[1]);
            let step = (self.style.shape.size * self.style.shape.aspect * 0.15).clamp(0.25, 8.0);
            if length > 1e-9 {
                let radius = self.style.shape.size * 0.5 + 1.0;
                if let Some([begin, end]) = clip(
                    from,
                    to,
                    [-radius, -radius],
                    [doc.width as f64 + radius, doc.height as f64 + radius],
                ) {
                    let first = step - self.remainder;
                    let start = ((begin * length - first) / step).ceil().max(0.0) as usize;
                    let finish = ((end * length - first) / step).floor();
                    if finish >= start as f64 {
                        let finish = finish as usize;
                        if finish - start > 1_000_000 {
                            return Err("Stroke path is too long".into());
                        }
                        for n in start..=finish {
                            let t = (first + n as f64 * step) / length;
                            self.stamp(doc, [from[0] + delta[0] * t, from[1] + delta[1] * t])?;
                        }
                    }
                }
                self.remainder = (self.remainder + length).rem_euclid(step);
            }
        } else {
            self.stamp(doc, to)?;
        }
        self.last = Some(to);
        Ok(())
    }
    pub fn finish_sample(&mut self, doc: &Document) -> Result<(), String> {
        if let Some(last) = self.last
            && self
                .last_stamp
                .is_none_or(|p| (last[0] - p[0]).hypot(last[1] - p[1]) > 1e-7)
        {
            self.stamp(doc, last)?;
        }
        Ok(())
    }
    pub fn commit(mut self, doc: &mut Document) -> Result<bool, String> {
        self.check(doc)?;
        self.finish_sample(doc)?;
        let Self { layer, changes, .. } = self;
        doc.commit_pixel_changes(layer, changes)
    }
    pub fn pixel(
        &self,
        doc: &Document,
        layer: usize,
        raster: &RasterLayer,
        x: u32,
        y: u32,
    ) -> Option<[u8; 4]> {
        if layer != self.layer || doc.id != self.document || doc.revision != self.revision {
            return None;
        }
        let offset = raster.pixel_index(doc.width, doc.height, x as i64, y as i64)?;
        self.lookup.get(&offset).map(|n| self.changes[*n].after)
    }
    pub fn preview_key(&self) -> (u64, u64) {
        (self.nonce, self.preview_revision)
    }
    pub fn style(&self) -> Style {
        self.style
    }
    pub fn trace(&self) -> Trace {
        self.trace.clone()
    }
    pub fn changed_pixels(&self) -> usize {
        self.changes.len()
    }
    fn stamp(&mut self, doc: &Document, center: Point) -> Result<(), String> {
        self.last_stamp = Some(center);
        let shape = self.style.shape;
        let radius = shape.size * 0.5;
        let minor = radius * shape.aspect;
        let (sin, cos) = shape.angle.to_radians().sin_cos();
        let extent = [
            radius * (cos * cos + shape.aspect * shape.aspect * sin * sin).sqrt() + 1.0,
            radius * (sin * sin + shape.aspect * shape.aspect * cos * cos).sqrt() + 1.0,
        ];
        let layer = &doc.layers[self.layer];
        let storage = layer.extent(doc.width, doc.height);
        let low = [
            (center[0] - extent[0])
                .floor()
                .max(0.0)
                .max(storage.offset[0] as f64) as i64,
            (center[1] - extent[1])
                .floor()
                .max(0.0)
                .max(storage.offset[1] as f64) as i64,
        ];
        let high = [
            (center[0] + extent[0])
                .ceil()
                .min(doc.width as f64 - 1.0)
                .min((storage.offset[0] + storage.width as i64 - 1) as f64) as i64,
            (center[1] + extent[1])
                .ceil()
                .min(doc.height as f64 - 1.0)
                .min((storage.offset[1] + storage.height as i64 - 1) as f64) as i64,
        ];
        let mut changed = false;
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                let mask = doc
                    .scene
                    .selection_mask
                    .as_ref()
                    .map_or(255, |m| m.coverage(point));
                if mask == 0 {
                    continue;
                }
                let dx = point[0] - center[0];
                let dy = point[1] - center[1];
                let distance =
                    ((dx * cos + dy * sin) / radius).hypot((-dx * sin + dy * cos) / minor);
                let coverage = ((1.0 - distance) * minor + 0.5).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }
                let amount = match self.style.tip {
                    BrushTip::Pen | BrushTip::Eraser => coverage,
                    BrushTip::Pencil => {
                        let grain = ((x.rem_euclid(23) * 17
                            + y.rem_euclid(23) * 31
                            + x.rem_euclid(23) * y.rem_euclid(23) * 7)
                            .rem_euclid(23)) as f64
                            / 22.0;
                        coverage * (0.4 + 0.6 * grain)
                    }
                    BrushTip::Wash => {
                        coverage
                            * (1.0 - (distance - 0.5 / minor).max(0.0).powi(2))
                                .max(0.0)
                                .powi(3)
                            * 0.65
                    }
                };
                let offset = layer.pixel_index(doc.width, doc.height, x, y).unwrap();
                let old = self.lookup.get(&offset).copied();
                let original = old.map_or(layer.pixels[offset], |n| self.changes[n].before);
                let unmasked = old.map_or(original, |n| self.unmasked[n]);
                let next = if self.style.tip == BrushTip::Eraser {
                    [
                        unmasked[0],
                        unmasked[1],
                        unmasked[2],
                        (unmasked[3] as f64 * (1.0 - amount)).round() as u8,
                    ]
                } else {
                    over(unmasked, self.style.color, amount)
                };
                let result = mix(original, next, mask as f64 / 255.0);
                if old.is_none() && result == original {
                    continue;
                }
                let index = if let Some(n) = old {
                    n
                } else {
                    if self.changes.len() >= self.limit {
                        return Err(
                            "Stroke exceeds available memory; reduce brush size or free history"
                                .into(),
                        );
                    }
                    self.changes.try_reserve(1).map_err(|e| e.to_string())?;
                    self.unmasked.try_reserve(1).map_err(|e| e.to_string())?;
                    self.lookup.try_reserve(1).map_err(|e| e.to_string())?;
                    let n = self.changes.len();
                    self.changes.push(PixelChange {
                        offset,
                        before: original,
                        after: original,
                    });
                    self.unmasked.push(original);
                    self.lookup.insert(offset, n);
                    n
                };
                changed |= self.changes[index].after != result;
                self.changes[index].after = result;
                self.unmasked[index] = next;
            }
        }
        if changed {
            self.preview_revision += 1;
        }
        Ok(())
    }
}
fn mix(a: [u8; 4], b: [u8; 4], weight: f64) -> [u8; 4] {
    if weight == 1.0 {
        return b;
    }
    let aa = a[3] as f64 / 255.0;
    let ba = b[3] as f64 / 255.0;
    let alpha = aa * (1.0 - weight) + ba * weight;
    if alpha <= 0.0 {
        return [a[0], a[1], a[2], 0];
    }
    let mut out = [0; 4];
    for c in 0..3 {
        out[c] =
            ((a[c] as f64 * aa * (1.0 - weight) + b[c] as f64 * ba * weight) / alpha).round() as u8;
    }
    out[3] = (alpha * 255.0).round() as u8;
    out
}
fn over(before: [u8; 4], color: [u8; 4], amount: f64) -> [u8; 4] {
    let a = color[3] as f64 / 255.0 * amount;
    let b = before[3] as f64 / 255.0;
    let alpha = a + b * (1.0 - a);
    if alpha <= 0.0 {
        return before;
    }
    let mut out = [0; 4];
    for c in 0..3 {
        out[c] = ((color[c] as f64 * a + before[c] as f64 * b * (1.0 - a)) / alpha).round() as u8;
    }
    out[3] = (alpha * 255.0).round() as u8;
    out
}
fn clip(from: Point, to: Point, min: Point, max: Point) -> Option<[f64; 2]> {
    let mut start: f64 = 0.0;
    let mut end: f64 = 1.0;
    for axis in 0..2 {
        let d = to[axis] - from[axis];
        if d.abs() < 1e-12 {
            if from[axis] < min[axis] || from[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - from[axis]) / d;
            let b = (max[axis] - from[axis]) / d;
            start = start.max(a.min(b));
            end = end.min(a.max(b));
            if start > end {
                return None;
            }
        }
    }
    Some([start, end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        render::Renderer,
        selection::{Mask, Run},
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 32,
                height: 12,
                dpi: 300.0,
                background: Background::Transparent,
            },
            4096,
        )
        .unwrap()
    }
    fn style(tip: BrushTip) -> Style {
        Style {
            shape: Shape {
                size: 1.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip,
            color: [30, 60, 90, 200],
            smoothing: 0.0,
        }
    }
    #[test]
    fn point_and_fast_drag_preview_commit_one_history_and_exact_undo_redo() {
        let mut d = doc();
        d.mark_saved("point.drawer".into());
        let before = d.snapshot();
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.5, 4.5]).unwrap();
        assert_eq!(d.snapshot(), before);
        assert_eq!(d.history_counts(), (0, 0));
        let preview = Renderer::preview(&d, Some(&s)).pixel([2.5, 4.5]);
        assert!(preview[3] > 0);
        assert!(s.commit(&mut d).unwrap());
        assert_eq!(d.composite_pixel(2, 4), preview);
        assert_eq!(d.history_counts(), (1, 0));
        assert!(d.dirty);
        let after = d.snapshot();
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
        assert!(!d.dirty);
        assert!(d.redo());
        assert_eq!(d.snapshot(), after);
        let mut a = doc();
        let mut b = doc();
        let mut fast = Stroke::begin(&a, 0, style(BrushTip::Pencil)).unwrap();
        fast.sample(&a, [1.5, 4.5]).unwrap();
        fast.sample(&a, [30.5, 4.5]).unwrap();
        fast.commit(&mut a).unwrap();
        let mut dense = Stroke::begin(&b, 0, style(BrushTip::Pencil)).unwrap();
        for x in 1..=30 {
            dense.sample(&b, [x as f64 + 0.5, 4.5]).unwrap();
        }
        dense.commit(&mut b).unwrap();
        assert_eq!(a.snapshot(), b.snapshot());
        assert_eq!(a.history_counts(), (1, 0));
        assert!((1..=30).all(|x| a.composite_pixel(x, 4)[3] > 0));
    }
    #[test]
    fn eraser_selection_feather_outside_clipping_cancel_limits_and_stale_input() {
        let mut d = doc();
        d.layers[0].pixels.fill([80, 90, 100, 255]);
        d.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 10,
                y: 4,
                length: 10,
                alpha: 128,
            }],
        });
        let before = d.snapshot();
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Eraser)).unwrap();
        s.sample(&d, [-1e12, 4.5]).unwrap();
        s.sample(&d, [1e12, 4.5]).unwrap();
        assert_eq!(d.snapshot(), before);
        s.commit(&mut d).unwrap();
        assert_eq!(d.history_counts(), (1, 0));
        for x in 0..32 {
            assert_eq!(
                d.layers[0].pixels[4 * 32 + x][3],
                if (10..20).contains(&x) { 127 } else { 255 }
            );
        }
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
        d.scene.selection_mask = Some(Mask::default());
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.0, 2.0]).unwrap();
        assert!(!s.commit(&mut d).unwrap());
        d.scene.selection_mask = None;
        let before = d.snapshot();
        let mut s = Stroke::with_budget(&d, 0, style(BrushTip::Pen), 512).unwrap();
        assert!(s.sample(&d, [f64::NAN, 0.0]).is_err());
        s.sample(&d, [2.5, 4.5]).unwrap();
        assert!(s.sample(&d, [30.5, 4.5]).is_err());
        drop(s);
        assert_eq!(d.snapshot(), before);
        let mut s = Stroke::begin(&d, 0, style(BrushTip::Pen)).unwrap();
        s.sample(&d, [2.5, 4.5]).unwrap();
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        assert!(s.commit(&mut d).is_err());
        assert_eq!(d.layers[0].pixels[0], [1, 2, 3, 255]);
    }
}
