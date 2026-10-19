//! Raw layer-content alpha to the existing fractional page-selection model.
use crate::{
    document::Document,
    scene::{Point, Rect, Scene},
    selection_edit::{Combine, Grid},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Raster(usize),
    Vector(u64),
    Gradient(u64),
}
pub fn sources(doc: &Document) -> Vec<Source> {
    (0..doc.layers.len())
        .map(Source::Raster)
        .chain(doc.scene.lines.layers.iter().map(|l| Source::Vector(l.id)))
        .chain(
            doc.scene
                .gradients
                .layers
                .iter()
                .map(|l| Source::Gradient(l.id)),
        )
        .collect()
}
fn corners(r: Rect) -> Vec<Point> {
    vec![r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
}
pub fn prepare(doc: &Document, source: Source, op: Combine) -> Result<Scene, String> {
    if !doc.scene.valid() {
        return Err("Invalid layer selection page".into());
    }
    let mut gradient = None;
    let mut vector = None;
    let (corners, cost) = match source {
        Source::Raster(n) => {
            let l = doc.layers.get(n).ok_or("Missing alpha source layer")?;
            if !l.valid(doc.width, doc.height) {
                return Err("Invalid alpha source raster".into());
            }
            let p = l.extent(doc.width, doc.height);
            (
                corners(Rect {
                    min: p.offset.map(|v| v as f64),
                    max: [
                        (p.offset[0] + p.width as i64) as f64,
                        (p.offset[1] + p.height as i64) as f64,
                    ],
                }),
                1,
            )
        }
        Source::Vector(id) => {
            let layer = doc
                .scene
                .lines
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Missing vector alpha source")?;
            vector = Some(layer);
            let mut corners = vec![];
            for path in &layer.paths {
                let r = path.bounds();
                corners.extend(
                    [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
                        .map(|p| doc.scene.transform.map(p)),
                );
            }
            (
                corners,
                layer
                    .paths
                    .iter()
                    .map(|p| p.points.len() * 2)
                    .sum::<usize>()
                    .max(1),
            )
        }
        Source::Gradient(id) => {
            let mut l = doc
                .scene
                .gradients
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Missing gradient alpha source")?
                .clone();
            l.hidden = false;
            let r = crate::alignment::bounds(doc, crate::alignment::Item::Gradient(id)).ok();
            let cost = (l.areas.len() * 3).max(1);
            gradient = Some(l);
            (r.map_or(vec![], corners), cost)
        }
    };
    let grid = Grid::with_bounds(doc, &corners)?;
    if grid.size[0]
        .saturating_mul(grid.size[1])
        .saturating_mul(16)
        .saturating_mul(cost)
        > 64 * 1024 * 1024
    {
        return Err("Layer alpha selection sampling work limit".into());
    }
    if (grid.size[0] * grid.size[1])
        .saturating_mul(4)
        .saturating_add(16 * 1024 * 1024)
        .saturating_add(doc.resident_bytes())
        > crate::document::DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Layer selection working memory limit".into());
    }
    let inverse = doc.scene.transform.inverse();
    let alpha = |p: Point| match source {
        Source::Raster(n) => doc.layers[n].pixel(
            doc.width,
            doc.height,
            p[0].floor() as i64,
            p[1].floor() as i64,
        )[3],
        Source::Vector(_) => {
            let p = inverse.map(p);
            let layer = vector.unwrap();
            layer.paths.iter().fold([0; 4], |color, path| {
                crate::render::over(color, path.pixel(p))
            })[3]
        }
        Source::Gradient(_) => gradient.as_ref().unwrap().pixel(inverse.map(p))[3],
    };
    let new = grid.sample(doc, false, alpha);
    let base = grid.base(doc, op == Combine::Add);
    let values: Vec<_> = base
        .into_iter()
        .zip(new)
        .map(|(a, b)| op.coverage(a, b))
        .collect();
    grid.scene(doc, &values)
}
