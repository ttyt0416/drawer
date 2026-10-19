//! Local morphology and box feathering in current document pixel units.
use crate::{
    document::Document,
    scene::{Point, Scene},
    selection_edit::Grid,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operation {
    #[default]
    Expand,
    Shrink,
    Feather,
}
impl Operation {
    pub const ALL: [Self; 3] = [Self::Expand, Self::Shrink, Self::Feather];
}
pub fn prepare(doc: &Document, op: Operation, radius: u8) -> Result<Scene, String> {
    if radius > 64 {
        return Err("Selection radius must be 0–64 document pixels".into());
    }
    if radius == 0
        || doc
            .scene
            .selection_mask
            .as_ref()
            .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(doc.scene.clone());
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut bounds = |a: Point, b: Point| {
        for p in [a, [b[0], a[1]], b, [a[0], b[1]]] {
            let p = doc.scene.transform.map(p);
            for i in 0..2 {
                let v = p[i] + doc.scene.selection_translation[i];
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
    };
    let unrestricted = doc.scene.selection_mask.is_none() && doc.scene.selection.is_none();
    if let Some(mask) = &doc.scene.selection_mask {
        for r in &mask.runs {
            bounds(
                [r.x as f64, r.y as f64],
                [r.x as f64 + r.length as f64, r.y as f64 + 1.0],
            );
        }
    } else if let Some(r) = doc.scene.selection {
        bounds(r.min, r.max);
    } else {
        lo = [0.0; 2];
        hi = [doc.width as f64, doc.height as f64];
    }
    lo = lo.map(|v| v.floor() - radius as f64);
    hi = hi.map(|v| v.ceil() + radius as f64);
    if lo
        .iter()
        .chain(&hi)
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
    {
        return Err("Selection filter coordinate limit".into());
    }
    let w = (hi[0] - lo[0]) as usize;
    let h = (hi[1] - lo[1]) as usize;
    let count = w.saturating_mul(h);
    if w == 0
        || h == 0
        || w > 32768
        || h > 32768
        || count > 4 * 1024 * 1024
        || count.saturating_mul(2 * (2 * radius as usize + 1)) > 64 * 1024 * 1024
    {
        return Err("Selection filter work limit".into());
    }
    let mut values = vec![0u8; count];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = [
                        lo[0] + x as f64 + (sx as f64 + 0.5) / 4.0,
                        lo[1] + y as f64 + (sy as f64 + 0.5) / 4.0,
                    ];
                    if !unrestricted
                        || (p[0] >= 0.0
                            && p[1] >= 0.0
                            && p[0] < doc.width as f64
                            && p[1] < doc.height as f64)
                    {
                        sum += doc.scene.selection_coverage(p) as u32;
                    }
                }
            }
            values[y * w + x] = ((sum + 8) / 16) as u8;
        }
    }
    let first = pass(&values, w, h, radius as usize, op, false);
    let values = pass(&first, w, h, radius as usize, op, true);
    let grid = Grid::with_bounds(doc, &[lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]])?;
    let output = grid.sample(doc, false, |p| {
        let x = (p[0].floor() - lo[0]) as i64;
        let y = (p[1].floor() - lo[1]) as i64;
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            0
        } else {
            values[y as usize * w + x as usize]
        }
    });
    grid.scene(doc, &output)
}
fn pass(input: &[u8], w: usize, h: usize, r: usize, op: Operation, vertical: bool) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    let length = if vertical { h } else { w };
    for y in 0..h {
        for x in 0..w {
            let at = if vertical { y } else { x };
            let mut result = if op == Operation::Shrink { 255u32 } else { 0 };
            for k in -(r as i64)..=r as i64 {
                let n = at as i64 + k;
                let v = if n < 0 || n >= length as i64 {
                    0
                } else if vertical {
                    input[n as usize * w + x]
                } else {
                    input[y * w + n as usize]
                };
                result = match op {
                    Operation::Expand => result.max(v as u32),
                    Operation::Shrink => result.min(v as u32),
                    Operation::Feather => result + v as u32,
                };
            }
            output[y * w + x] = if op == Operation::Feather {
                ((result + r as u32) / (2 * r as u32 + 1)) as u8
            } else {
                result as u8
            };
        }
    }
    output
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        selection::{Mask, Run},
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 7,
            height: 7,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 3,
            y: 3,
            length: 1,
            alpha: 255,
        }],
    });
    let before = d.snapshot();
    for op in Operation::ALL {
        if prepare(&d, op, 0)? != d.scene {
            return Err("Zero radius changed selection representation".into());
        }
    }
    let expand = prepare(&d, Operation::Expand, 1)?;
    for y in 0..7 {
        for x in 0..7 {
            let a = expand.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]);
            if a != if (2..=4).contains(&x) && (2..=4).contains(&y) {
                255
            } else {
                0
            } {
                return Err("Dilation independent square oracle".into());
            }
        }
    }
    if !prepare(&d, Operation::Shrink, 1)?
        .selection_mask
        .unwrap()
        .runs
        .is_empty()
    {
        return Err("Narrow selection did not vanish".into());
    }
    let feather = prepare(&d, Operation::Feather, 1)?;
    if feather.selection_coverage([3.5, 3.5]) != 28
        || feather.selection_coverage([2.5, 2.5]) != 28
        || feather.selection_coverage([1.5, 3.5]) != 0
    {
        return Err("Feather integer box oracle".into());
    }
    d.edit_scene(expand)?;
    let after = d.snapshot();
    if after.layers != before.layers
        || d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Filter pixels/history".into());
    }
    let path = root.join("selection-filter.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Filter native reopen".into());
    }
    if prepare(&d, Operation::Shrink, 1)?.selection_coverage([3.5, 3.5]) != 255 {
        return Err("Erosion center".into());
    }
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 0,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    for op in Operation::ALL {
        let s = prepare(&d, op, 1)?;
        if !s.valid() {
            return Err("Canvas-edge filter invalid".into());
        }
    }
    if prepare(&d, Operation::Expand, 1)?.selection_coverage([-0.5, -0.5]) != 128 {
        return Err("Canvas-edge dilation clipped outside selection".into());
    }
    d.scene.transform = crate::scene::Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let s = prepare(&d, Operation::Expand, 2)?;
    if s.selection_coverage([3.5, 0.5]) != 128 || s.selection_coverage([4.5, 0.5]) != 0 {
        return Err("Filter radius was not in document pixels".into());
    }
    let before = d.snapshot();
    if prepare(&d, Operation::Feather, 65).is_ok() || d.snapshot() != before {
        return Err("Invalid radius changed document".into());
    }
    d.scene.selection = None;
    d.scene.selection_mask = None;
    d.scene.transform = Default::default();
    if prepare(&d, Operation::Shrink, 1)?.selection_coverage([0.5, 0.5]) != 0
        || prepare(&d, Operation::Shrink, 1)?.selection_coverage([3.5, 3.5]) != 255
    {
        return Err("Unrestricted selection canvas bounds".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn zero_morphology_feather_edges_transforms_history_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
