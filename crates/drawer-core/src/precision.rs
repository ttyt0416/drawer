//! Integer document-space movement, recomputed from the original on each repeat.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, MAX_DIMENSION, PlacedRaster},
    sizing::DocumentState,
};
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    Boundary,
    Pixels,
    #[default]
    Layer,
}
impl Target {
    pub const ALL: [Self; 3] = [Self::Boundary, Self::Pixels, Self::Layer];
    pub fn label(self) -> &'static str {
        match self {
            Self::Boundary => "Selection boundary",
            Self::Pixels => "Selected pixels",
            Self::Layer => "Raster layer",
        }
    }
}
pub struct Nudge {
    document: u64,
    revision: u64,
    layer: usize,
    target: Target,
    base: DocumentState,
    total: [i64; 2],
    group: u64,
}
impl Nudge {
    pub fn begin(doc: &Document, layer: usize, target: Target) -> Result<Self, String> {
        let raster = doc.layers.get(layer).ok_or("Select a raster layer")?;
        if target != Target::Boundary && raster.locked {
            return Err("The raster layer is locked".into());
        }
        if target != Target::Layer && doc.scene.selection_empty() {
            return Err("Create a nonempty selection first".into());
        }
        if !doc.scene.valid() || !raster.valid(doc.width, doc.height) {
            return Err("Invalid movement source".into());
        }
        if doc
            .memory_bytes()
            .saturating_mul(3)
            .saturating_add(doc.resident_bytes())
            > DEFAULT_DOCUMENT_BUDGET
            || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        {
            return Err("Movement exceeds the document/undo memory budget".into());
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            document: doc.id,
            revision: doc.revision,
            layer,
            target,
            base: doc.snapshot(),
            total: [0; 2],
            group: NEXT.fetch_add(1, Ordering::Relaxed),
        })
    }
    pub fn matches(&self, doc: &Document, layer: usize, target: Target) -> bool {
        self.document == doc.id
            && self.revision == doc.revision
            && self.layer == layer
            && self.target == target
    }
    pub fn step(&mut self, doc: &mut Document, delta: [i64; 2]) -> Result<bool, String> {
        if !self.matches(doc, self.layer, self.target) {
            return Err("The movement target changed; start a fresh key press".into());
        }
        if self.target != Target::Boundary && doc.layers[self.layer].locked {
            return Err("The raster layer is locked".into());
        }
        let total = [
            self.total[0].checked_add(delta[0]),
            self.total[1].checked_add(delta[1]),
        ];
        let [Some(x), Some(y)] = total else {
            return Err("Movement coordinate limit".into());
        };
        if x.unsigned_abs() > 1_000_000 || y.unsigned_abs() > 1_000_000 {
            return Err("Movement coordinate limit".into());
        }
        let total = [x, y];
        let mut after = self.base.clone();
        match self.target {
            Target::Layer => {
                let mut extent = after.layers[self.layer].extent(after.width, after.height);
                extent.offset = [extent.offset[0] + x, extent.offset[1] + y];
                if !extent.valid() {
                    return Err("Layer position limit".into());
                }
                let inverse = after.scene.transform.inverse();
                let delta = inverse.map([x as f64, y as f64]);
                let origin = inverse.map([0.0; 2]);
                after
                    .scene
                    .paper
                    .shift_layer(self.layer, [delta[0] - origin[0], delta[1] - origin[1]]);
                after.layers[self.layer].placement = if total == [0; 2] {
                    self.base.layers[self.layer].placement
                } else {
                    Some(extent)
                };
            }
            Target::Boundary | Target::Pixels => {
                after.scene.selection_translation = [
                    self.base.scene.selection_translation[0] + x as f64,
                    self.base.scene.selection_translation[1] + y as f64,
                ];
                if self.target == Target::Pixels && total != [0; 2] {
                    move_pixels(&self.base, &mut after, self.layer, total)?;
                }
            }
        }
        let changed = doc.commit_state_group(after, Some(self.group))?;
        self.total = total;
        self.revision = doc.revision;
        Ok(changed)
    }
}
fn move_pixels(
    base: &DocumentState,
    after: &mut DocumentState,
    layer: usize,
    delta: [i64; 2],
) -> Result<(), String> {
    let source = &base.layers[layer];
    let e = source.extent(base.width, base.height);
    let offset = [e.offset[0] + delta[0].min(0), e.offset[1] + delta[1].min(0)];
    let width = e.width as u64 + delta[0].unsigned_abs();
    let height = e.height as u64 + delta[1].unsigned_abs();
    if width > MAX_DIMENSION as u64 || height > MAX_DIMENSION as u64 {
        return Err("Moved raster exceeds dimension limit".into());
    }
    let count = (width * height) as usize;
    if count.saturating_mul(4).saturating_add(base.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4 {
        return Err("Moved pixels exceed undo budget".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| "Not enough movement memory")?;
    pixels.resize(count, [0; 4]);
    let destination = PlacedRaster {
        width: width as u32,
        height: height as u32,
        offset,
    };
    if !destination.valid() {
        return Err("Moved raster position limit".into());
    }
    let index = |x: i64, y: i64| ((y - offset[1]) * width as i64 + x - offset[0]) as usize;
    // Cut from the original first, then composite the moved component. Repeats
    // never resample a partially moved result, including feathered masks.
    for y in 0..e.height as i64 {
        for x in 0..e.width as i64 {
            let p = source.pixels[(y * e.width as i64 + x) as usize];
            let coverage = base.scene.selection_coverage([
                (e.offset[0] + x) as f64 + 0.5,
                (e.offset[1] + y) as f64 + 0.5,
            ]);
            let selected = (p[3] as u32 * coverage as u32 + 127) / 255;
            pixels[index(e.offset[0] + x, e.offset[1] + y)] =
                [p[0], p[1], p[2], p[3] - selected as u8];
        }
    }
    for y in 0..e.height as i64 {
        for x in 0..e.width as i64 {
            let mut p = source.pixels[(y * e.width as i64 + x) as usize];
            let coverage = base.scene.selection_coverage([
                (e.offset[0] + x) as f64 + 0.5,
                (e.offset[1] + y) as f64 + 0.5,
            ]);
            p[3] = ((p[3] as u32 * coverage as u32 + 127) / 255) as u8;
            if p[3] == 0 {
                continue;
            }
            let dest = &mut pixels[index(e.offset[0] + x + delta[0], e.offset[1] + y + delta[1])];
            let a = p[3] as f64 / 255.0;
            let b = dest[3] as f64 / 255.0;
            let out = a + b * (1.0 - a);
            for n in 0..3 {
                dest[n] = ((p[n] as f64 * a + dest[n] as f64 * b * (1.0 - a)) / out).round() as u8;
            }
            dest[3] = (out * 255.0).round() as u8;
        }
    }
    after.layers[layer].pixels = pixels;
    after.layers[layer].placement = Some(destination);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        scene::{Affine, Rect},
        selection::{Mask, Run},
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 8,
                height: 8,
                dpi: 300.0,
                background: Background::Transparent,
            },
            256,
        )
        .unwrap()
    }
    #[test]
    fn document_pixel_movement_is_view_independent_grouped_reversible_and_outside_safe() {
        for zoom in [0.01, 1.0, 64.0] {
            for flipped in [false, true] {
                let mut d = doc();
                d.layers[0].pixels[0] = [10, 20, 30, 255];
                d.view.zoom = zoom;
                d.view.angle = 37.0;
                d.view.flipped = flipped;
                d.mark_saved("test.drawer".into());
                let before = d.snapshot();
                let view = d.view;
                let mut n = Nudge::begin(&d, 0, Target::Layer).unwrap();
                n.step(&mut d, [1, 0]).unwrap();
                n.step(&mut d, [0, 10]).unwrap();
                assert_eq!(d.layers[0].extent(d.width, d.height).offset, [1, 10]);
                assert_eq!(d.layers[0].pixels, before.layers[0].pixels);
                assert_eq!(d.history_counts(), (1, 0));
                assert_eq!(d.view, view);
                assert!(d.dirty);
                let after = d.snapshot();
                d.undo();
                assert!(d.snapshot() == before);
                assert!(!d.dirty);
                d.redo();
                assert!(d.snapshot() == after);
                assert_eq!(d.view, view);
            }
        }
    }
    #[test]
    fn net_zero_new_press_saved_boundary_and_stale_revision_do_not_merge_unrelated_edits() {
        let mut d = doc();
        d.mark_saved("test.drawer".into());
        let before = d.snapshot();
        let mut n = Nudge::begin(&d, 0, Target::Layer).unwrap();
        n.step(&mut d, [1, 0]).unwrap();
        n.step(&mut d, [-1, 0]).unwrap();
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        assert!(!d.dirty);
        n.step(&mut d, [1, 0]).unwrap();
        d.mark_saved("test.drawer".into());
        n.step(&mut d, [1, 0]).unwrap();
        assert_eq!(d.history_counts(), (2, 0));
        d.undo();
        assert!(!d.dirty);
        let before = d.snapshot();
        assert!(n.step(&mut d, [1, 0]).is_err());
        assert!(d.snapshot() == before);
        let mut fresh = Nudge::begin(&d, 0, Target::Layer).unwrap();
        fresh.step(&mut d, [0, 1]).unwrap();
        assert_eq!(d.history_counts(), (2, 0));
    }
    #[test]
    fn selection_border_uses_document_translation_after_page_transform() {
        let mut d = doc();
        d.scene.selection = Some(Rect {
            min: [1.0, 2.0],
            max: [3.0, 4.0],
        });
        d.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 8.0, 0.0]);
        let before = d.snapshot();
        let p = d.scene.selection_point([1.5, 2.5]);
        assert_eq!(d.scene.selection_coverage(p), 255);
        let mut n = Nudge::begin(&d, 0, Target::Boundary).unwrap();
        n.step(&mut d, [1, 10]).unwrap();
        assert_eq!(d.scene.selection_translation, [1.0, 10.0]);
        assert_eq!(d.scene.selection_coverage([p[0] + 1.0, p[1] + 10.0]), 255);
        assert_eq!(d.layers, before.layers);
        d.undo();
        assert!(d.snapshot() == before);
    }
    #[test]
    fn selected_feathered_pixels_move_from_original_and_keep_one_history_entry() {
        let mut d = doc();
        d.layers[0].pixels[0] = [255, 0, 0, 200];
        d.layers[0].pixels[1] = [0, 0, 255, 255];
        d.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 0,
                y: 0,
                length: 1,
                alpha: 128,
            }],
        });
        let before = d.snapshot();
        let mut n = Nudge::begin(&d, 0, Target::Pixels).unwrap();
        n.step(&mut d, [1, 0]).unwrap();
        assert_eq!(d.layers[0].pixel(d.width, d.height, 0, 0), [255, 0, 0, 100]);
        assert_eq!(
            d.layers[0].pixel(d.width, d.height, 1, 0),
            [100, 0, 155, 255]
        );
        n.step(&mut d, [1, 0]).unwrap();
        assert_eq!(d.history_counts(), (1, 0));
        assert_eq!(d.layers[0].pixel(d.width, d.height, 1, 0), [0, 0, 255, 255]);
        assert_eq!(d.layers[0].pixel(d.width, d.height, 2, 0), [255, 0, 0, 100]);
        assert_eq!(d.scene.selection_coverage([2.5, 0.5]), 128);
        let after = d.snapshot();
        d.undo();
        assert!(d.snapshot() == before);
        d.redo();
        assert!(d.snapshot() == after);
    }
    #[test]
    fn locks_empty_selection_invalid_deltas_and_missing_layers_preserve_document() {
        let mut d = doc();
        assert!(Nudge::begin(&d, 99, Target::Layer).is_err());
        d.scene.selection_mask = Some(Mask::default());
        assert!(Nudge::begin(&d, 0, Target::Pixels).is_err());
        assert!(Nudge::begin(&d, 0, Target::Boundary).is_err());
        d.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 0,
                y: 0,
                length: 1,
                alpha: 255,
            }],
        });
        d.layers[0].locked = true;
        assert!(Nudge::begin(&d, 0, Target::Layer).is_err());
        assert!(Nudge::begin(&d, 0, Target::Pixels).is_err());
        assert!(Nudge::begin(&d, 0, Target::Boundary).is_ok());
        d.layers[0].locked = false;
        let before = d.snapshot();
        let mut n = Nudge::begin(&d, 0, Target::Layer).unwrap();
        assert!(n.step(&mut d, [i64::MAX, 0]).is_err());
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
    }
}
