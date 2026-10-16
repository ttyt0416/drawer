//! Copy unassociated RGBA from the active raster. Selection is applied once.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    paste::ClipboardImage,
    references::LocalImage,
    scene::Rect,
};

pub fn copy(doc: &Document, layer: usize, budget: usize) -> Result<ClipboardImage, String> {
    let raster = doc
        .layers
        .get(layer)
        .ok_or("Select a raster layer to copy")?;
    if !doc.scene.valid() || !raster.valid(doc.width, doc.height) {
        return Err("Invalid selection or raster".into());
    }
    let extent = raster.extent(doc.width, doc.height);
    let mut min = extent.offset;
    let mut max = [min[0] + extent.width as i64, min[1] + extent.height as i64];
    let restricted = doc.scene.selection_mask.is_some() || doc.scene.selection.is_some();
    if restricted {
        let mut bounds = Rect {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        };
        let mut add = |a: [f64; 2], b: [f64; 2]| {
            for p in [a, [b[0], a[1]], b, [a[0], b[1]]] {
                let q = doc.scene.selection_point(p);
                for (i, v) in q.into_iter().enumerate() {
                    bounds.min[i] = bounds.min[i].min(v);
                    bounds.max[i] = bounds.max[i].max(v);
                }
            }
        };
        if let Some(mask) = &doc.scene.selection_mask {
            for r in &mask.runs {
                add(
                    [r.x as f64, r.y as f64],
                    [r.x as f64 + r.length as f64, r.y as f64 + 1.0],
                );
            }
        } else if let Some(rect) = doc.scene.selection {
            add(rect.min, rect.max);
        }
        if !bounds.min[0].is_finite() {
            return Err("The selection is empty".into());
        }
        for i in 0..2 {
            min[i] = min[i].max(bounds.min[i].floor() as i64);
            max[i] = max[i].min(bounds.max[i].ceil() as i64);
        }
    }
    if (0..2).any(|i| min[i] >= max[i]) {
        return Err("The selection does not overlap this layer".into());
    }
    let size = [(max[0] - min[0]) as u32, (max[1] - min[1]) as u32];
    let count = size[0] as usize * size[1] as usize;
    if !(PlacedRaster {
        width: size[0],
        height: size[1],
        offset: min,
    })
    .valid()
        || count.saturating_mul(4).saturating_add(36) > budget.min(DEFAULT_DOCUMENT_BUDGET / 4)
    {
        return Err("Selected pixels exceed the clipboard budget".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| "Not enough clipboard memory")?;
    let mut selected = false;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let coverage = doc
                .scene
                .selection_coverage([x as f64 + 0.5, y as f64 + 0.5]);
            let mut p = raster.pixel(doc.width, doc.height, x, y);
            if coverage == 0 {
                p = [0; 4];
            } else {
                selected = true;
                p[3] = ((p[3] as u32 * coverage as u32 + 127) / 255) as u8;
            }
            pixels.push(p);
        }
    }
    if !selected {
        return Err("The selection contains no layer pixels".into());
    }
    Ok(ClipboardImage {
        image: LocalImage {
            width: size[0],
            height: size[1],
            pixels,
        },
        origin: Some(min),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        precision::{Nudge, Target},
        selection::{Mask, Run},
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 4,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            10000,
        )
        .unwrap()
    }
    #[test]
    fn copy_preserves_hidden_rgb_scales_coverage_and_never_changes_source() {
        let mut d = doc();
        d.layers[0].pixels[..4].copy_from_slice(&[
            [20, 30, 40, 0],
            [90, 80, 70, 128],
            [7, 8, 9, 255],
            [1, 2, 3, 255],
        ]);
        d.scene.selection_mask = Some(Mask {
            runs: vec![
                Run {
                    x: 0,
                    y: 0,
                    length: 2,
                    alpha: 128,
                },
                Run {
                    x: 3,
                    y: 0,
                    length: 1,
                    alpha: 255,
                },
            ],
        });
        let before = d.snapshot();
        let history = d.history_counts();
        let dirty = d.dirty;
        let p = copy(&d, 0, 1000).unwrap();
        assert_eq!(p.origin, Some([0, 0]));
        assert_eq!(
            p.image.pixels,
            vec![[20, 30, 40, 0], [90, 80, 70, 64], [0; 4], [1, 2, 3, 255]]
        );
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), history);
        assert_eq!(d.dirty, dirty);
        let q = ClipboardImage::decode(&p.encode(1000).unwrap(), 1000).unwrap();
        assert_eq!(q.image, p.image);
        assert!(copy(&d, 0, 51).is_err());
        assert!(copy(&d, 99, 1000).is_err());
    }
    #[test]
    fn empty_outside_transformed_and_whole_offcanvas_layer() {
        let mut d = doc();
        d.layers[0].placement = Some(PlacedRaster {
            width: 4,
            height: 2,
            offset: [-2, 5],
        });
        assert_eq!(copy(&d, 0, 1000).unwrap().origin, Some([-2, 5]));
        d.scene.selection_mask = Some(Mask::default());
        assert!(copy(&d, 0, 1000).is_err());
        d.scene.selection_mask = None;
        d.scene.selection = Some(Rect {
            min: [0.0; 2],
            max: [1.0; 2],
        });
        assert!(copy(&d, 0, 1000).is_err());
        d.scene.transform = crate::scene::Affine([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        d.scene.selection_translation = [-1.0, 5.0];
        let p = copy(&d, 0, 1000).unwrap();
        assert_eq!(p.origin, Some([-2, 5]));
        assert_eq!(p.image.width, 1);
        assert_eq!(p.image.height, 1);
    }
    #[test]
    fn frozen_movement_preview_cut_outside_and_single_undo() {
        let mut d = doc();
        d.layers[0].pixels[0] = [10, 20, 30, 128];
        d.scene.selection = Some(Rect {
            min: [0.0; 2],
            max: [1.0; 2],
        });
        let before = d.snapshot();
        let n = Nudge::begin(&d, 0, Target::Pixels).unwrap();
        let preview = n.preview_total(&d, [-5, 3]).unwrap();
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        assert_eq!(
            preview.layers[0].pixel(d.width, d.height, -5, 3),
            [10, 20, 30, 128]
        );
        assert_eq!(
            preview.layers[0].pixel(d.width, d.height, 0, 0),
            [10, 20, 30, 0]
        );
        assert!(n.preview_total(&d, [0, 0]).unwrap() == before);
        assert!(n.preview_total(&d, [i64::MAX, 0]).is_err());
        d.commit_state(preview).unwrap();
        assert_eq!(d.history_counts(), (1, 0));
        d.undo();
        assert!(d.snapshot() == before);
        assert!(n.preview_total(&d, [1, 0]).is_err());
    }
    #[test]
    fn dense_coverage_binary_lookup_respects_gaps_and_boundaries() {
        let m = Mask {
            runs: (0..16000)
                .map(|x| Run {
                    x: x * 2,
                    y: 2,
                    length: 1,
                    alpha: (x % 255 + 1) as u8,
                })
                .collect(),
        };
        assert!(m.valid());
        for x in 0..32000 {
            assert_eq!(
                m.coverage([x as f64 + 0.5, 2.5]),
                if x % 2 == 0 {
                    (x / 2 % 255 + 1) as u8
                } else {
                    0
                }
            );
        }
        assert_eq!(m.coverage([f64::NAN, 2.0]), 0);
        assert_eq!(m.coverage([-0.1, 2.0]), 0);
        assert_eq!(m.coverage([0.0, 3.0]), 0);
    }
}
