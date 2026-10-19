//! Document-space shape selection, sampled into the preserved page grid.
use crate::{
    document::Document,
    scene::{Point, Scene},
    selection_edit::{Combine, Grid},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rectangle,
    Ellipse,
    Polygon,
}

pub fn prepare(
    doc: &Document,
    kind: Kind,
    points: &[Point],
    combine: Combine,
) -> Result<Scene, String> {
    if points
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Selection needs finite document coordinates".into());
    }
    match kind {
        Kind::Rectangle | Kind::Ellipse => {
            if points.len() != 2 || points[0][0] == points[1][0] || points[0][1] == points[1][1] {
                return Err("Shape needs nonzero width and height".into());
            }
        }
        Kind::Polygon => {
            if !(3..=4096).contains(&points.len()) {
                return Err("Polygon needs 3–4096 vertices".into());
            }
            let a = points[0];
            let b = points
                .iter()
                .find(|p| **p != a)
                .ok_or("Polygon has no area")?;
            if !points.iter().any(|p| {
                ((p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])).abs() > 1e-9
            }) {
                return Err("Polygon has no area".into());
            }
        }
    }
    let grid = Grid::new(doc)?;
    let cost = if kind == Kind::Polygon {
        points.len()
    } else {
        1
    };
    if grid.size[0]
        .saturating_mul(grid.size[1])
        .saturating_mul(16)
        .saturating_mul(cost)
        > 64 * 1024 * 1024
    {
        return Err("Shape selection exceeds sampling work budget".into());
    }
    let incoming = grid.sample(
        doc,
        true,
        |p| if contains(kind, points, p) { 255 } else { 0 },
    );
    let values = grid
        .base(doc, combine == Combine::Add)
        .into_iter()
        .zip(incoming)
        .map(|(a, b)| combine.coverage(a, b))
        .collect::<Vec<_>>();
    grid.scene(doc, &values)
}
fn contains(kind: Kind, points: &[Point], p: Point) -> bool {
    if kind != Kind::Polygon {
        let a = points[0];
        let b = points[1];
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        if kind == Kind::Rectangle {
            return p[0] >= lo[0] && p[0] < hi[0] && p[1] >= lo[1] && p[1] < hi[1];
        }
        let x = (2.0 * p[0] - lo[0] - hi[0]) / (hi[0] - lo[0]);
        let y = (2.0 * p[1] - lo[1] - hi[1]) / (hi[1] - lo[1]);
        return x * x + y * y <= 1.0;
    }
    let mut inside = false;
    for n in 0..points.len() {
        let a = points[n];
        let b = points[(n + 1) % points.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        selection::{Mask, Run},
    };
    let make = || {
        Document::create(
            CanvasSpec {
                width: 8,
                height: 8,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())
    };
    let polygon = [[1.5, 0.0], [5.5, 0.0], [5.5, 6.0], [1.5, 6.0]];
    for (op, edge, middle) in [
        (Combine::Replace, 128, 255),
        (Combine::Add, 192, 255),
        (Combine::Subtract, 64, 0),
        (Combine::Intersect, 64, 128),
    ] {
        let mut d = make()?;
        d.scene.selection_mask = Some(Mask {
            runs: vec![Run {
                x: 0,
                y: 2,
                length: 8,
                alpha: 128,
            }],
        });
        let old = d.snapshot();
        let s = prepare(&d, Kind::Polygon, &polygon, op)?;
        if d.snapshot() != old
            || s.selection_coverage([1.5, 2.5]) != edge
            || s.selection_coverage([3.5, 2.5]) != middle
        {
            return Err(format!("Polygon composition {op:?} failed"));
        }
        d.edit_scene(s)?;
        let after = d.snapshot();
        if d.history_counts() != (1, 0)
            || !d.undo()
            || d.snapshot() != old
            || !d.redo()
            || d.snapshot() != after
        {
            return Err("Shape Undo/Redo".into());
        }
        let path = root.join("geometric-selection.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != d.snapshot()
        {
            return Err("Geometric selection reopen".into());
        }
    }
    let mut d = make()?;
    let ellipse = prepare(
        &d,
        Kind::Ellipse,
        &[[1.0, 1.0], [7.0, 7.0]],
        Combine::Replace,
    )?;
    if ellipse.selection_coverage([4.5, 4.5]) != 255
        || ellipse.selection_coverage([1.5, 1.5]) != 0
        || ellipse.selection_coverage([2.5, 1.5]) != 143
    {
        return Err(format!(
            "Ellipse independent coverage: {}",
            ellipse.selection_coverage([2.5, 1.5])
        ));
    }
    d.edit_scene(ellipse)?;
    let base = d.snapshot();
    for (op, expected) in [
        (Combine::Replace, 143),
        (Combine::Add, 206),
        (Combine::Subtract, 63),
        (Combine::Intersect, 80),
    ] {
        let same = prepare(&d, Kind::Ellipse, &[[7.0, 7.0], [1.0, 1.0]], op)?;
        if same.selection_coverage([2.5, 1.5]) != expected {
            return Err("Identical soft shape composition".into());
        }
    }
    let forward = prepare(&d, Kind::Polygon, &polygon, Combine::Replace)?;
    let reversed = polygon.into_iter().rev().collect::<Vec<_>>();
    if prepare(&d, Kind::Polygon, &reversed, Combine::Replace)?.selection_mask
        != forward.selection_mask
    {
        return Err("Polygon winding changed selection".into());
    }
    let concave = prepare(
        &d,
        Kind::Polygon,
        &[
            [1.0, 1.0],
            [7.0, 1.0],
            [7.0, 3.0],
            [3.0, 3.0],
            [3.0, 7.0],
            [1.0, 7.0],
        ],
        Combine::Replace,
    )?;
    if concave.selection_coverage([2.5, 6.5]) != 255 || concave.selection_coverage([6.5, 6.5]) != 0
    {
        return Err("Concave polygon".into());
    }

    for op in Combine::ALL {
        let s = prepare(&d, Kind::Rectangle, &[[0.0, 0.0], [1.0, 1.0]], op)?;
        let expected = if matches!(op, Combine::Add | Combine::Replace) {
            255
        } else {
            0
        };
        if s.selection_coverage([0.5, 0.5]) != expected {
            return Err("Disjoint shape composition".into());
        }
        let same = prepare(&d, Kind::Rectangle, &[[0.0, 0.0], [8.0, 8.0]], op)?;
        let expected = if op == Combine::Subtract {
            0
        } else if matches!(op, Combine::Replace | Combine::Add) {
            255
        } else {
            143
        };
        if same.selection_coverage([2.5, 1.5]) != expected {
            return Err("Contained shape composition".into());
        }
    }
    for (kind, p) in [
        (Kind::Ellipse, vec![[1.0, 1.0]; 2]),
        (Kind::Polygon, vec![[0.0; 2], [1.0; 2], [2.0; 2]]),
        (Kind::Polygon, vec![[f64::NAN, 0.0]; 3]),
    ] {
        if prepare(&d, kind, &p, Combine::Replace).is_ok() || d.snapshot() != base {
            return Err("Invalid shape modified source".into());
        }
    }
    d.scene.transform = crate::scene::Affine([0.0, 1.0, -1.0, 0.0, 8.0, 0.0]);
    let s = prepare(
        &d,
        Kind::Polygon,
        &[[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]],
        Combine::Replace,
    )?;
    if s.selection_coverage([3.5, 3.5]) != 255 || s.selection_coverage([1.5, 3.5]) != 0 {
        return Err("Transformed shape coordinates".into());
    }
    d.scene.transform = Default::default();
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: -2,
            y: 1,
            length: 1,
            alpha: 128,
        }],
    });
    d.scene.selection_translation = [-1.0, 0.0];
    let s = prepare(&d, Kind::Ellipse, &[[2.0, 2.0], [6.0, 6.0]], Combine::Add)?;
    if s.selection_coverage([-2.5, 1.5]) != 128 || s.selection_translation != [0.0; 2] {
        return Err("Moved off-canvas selection was lost".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn shapes_composition_transforms_noop_errors_history_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
