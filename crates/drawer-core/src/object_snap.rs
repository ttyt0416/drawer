//! Finite object edges, editable control points and canvas edges in document space.
use crate::{
    alignment::{self, Item},
    document::Document,
    scene::{Point, Rect},
    snapping::Settings,
};
#[derive(Clone, Default)]
pub struct Geometry {
    segments: Vec<[Point; 2]>,
    points: Vec<Point>,
}
impl Geometry {
    pub fn new(doc: &Document, s: Settings, exclude: &[Item]) -> Result<Self, String> {
        if !s.valid() || !doc.scene.valid() {
            return Err("Invalid object snapping scene/settings".into());
        }
        let mut result = Self::default();
        if s.objects {
            for i in alignment::items(doc) {
                let visible = match i {
                    Item::Raster(n) => doc.scene.paper.raster_visible(n, doc.layers.len()),
                    Item::Vector(id) => doc
                        .scene
                        .lines
                        .layers
                        .iter()
                        .any(|l| !l.hidden && l.paths.iter().any(|p| p.id == id)),
                    Item::Gradient(id) => doc
                        .scene
                        .gradients
                        .layers
                        .iter()
                        .any(|l| !l.hidden && l.id == id),
                    _ => true,
                };
                if visible && !exclude.contains(&i) {
                    result.rectangle(alignment::bounds(doc, i)?, true);
                }
            }
        }
        if s.control_points {
            for l in doc.scene.lines.layers.iter().filter(|l| !l.hidden) {
                for p in &l.paths {
                    if exclude.contains(&Item::Vector(p.id)) {
                        continue;
                    }
                    if let Some(def) = &p.definition {
                        result.points.extend(
                            def.controls
                                .iter()
                                .map(|p| def.creation.inverse().then(doc.scene.transform).map(*p)),
                        );
                    } else {
                        result
                            .points
                            .extend(p.points.iter().map(|p| doc.scene.transform.map(*p)));
                    }
                }
            }
            for (i, o) in doc.scene.objects.iter().enumerate() {
                if exclude.contains(&Item::Object(i)) {
                    continue;
                }
                match o {
                    crate::scene::PageObject::Line { start, end, .. } => result
                        .points
                        .extend([*start, *end].map(|p| doc.scene.transform.map(p))),
                    crate::scene::PageObject::Text { baseline, .. } => {
                        result.points.push(doc.scene.transform.map(*baseline))
                    }
                    _ => {}
                }
            }
        }
        if s.canvas_edges {
            result.rectangle(
                Rect {
                    min: [0.0; 2],
                    max: [doc.width as f64, doc.height as f64],
                },
                false,
            );
        }
        if result.segments.len() + result.points.len() > 65536
            || result
                .segments
                .iter()
                .flatten()
                .chain(&result.points)
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("Object snapping target/coordinate limit".into());
        }
        Ok(result)
    }
    fn rectangle(&mut self, r: Rect, centers: bool) {
        let a = r.min;
        let b = [r.max[0], r.min[1]];
        let c = r.max;
        let d = [r.min[0], r.max[1]];
        self.segments.extend([[a, b], [b, c], [c, d], [d, a]]);
        if centers {
            let x = (a[0] + c[0]) / 2.0;
            let y = (a[1] + c[1]) / 2.0;
            self.segments
                .extend([[[x, a[1]], [x, c[1]]], [[a[0], y], [c[0], y]]]);
        }
    }
    pub fn bytes(&self) -> usize {
        self.segments.capacity() * 32 + self.points.capacity() * 16
    }
    pub fn work(&self) -> usize {
        self.segments.len() + self.points.len()
    }
    pub fn snap(&self, p: Point, distance: f64) -> Option<(Point, f64)> {
        let mut best = None;
        let mut candidate = |q: Point| {
            let d = (q[0] - p[0]).hypot(q[1] - p[1]);
            if d <= distance && best.is_none_or(|(_, old)| d < old - 1e-9) {
                best = Some((q, d));
            }
        };
        // Exact control points win equal-distance ties; stable document order breaks remaining ties.
        for q in &self.points {
            candidate(*q);
        }
        for [a, b] in &self.segments {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let t = if length > 1e-18 {
                ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length
            } else {
                0.0
            };
            let t = t.clamp(0.0, 1.0);
            candidate([a[0] + t * d[0], a[1] + t * d[1]]);
        }
        best
    }
    /// Snap a moving box by its corners and center while retaining its size.
    pub fn movement(&self, r: Rect, delta: Point, distance: f64) -> Point {
        let c = [(r.min[0] + r.max[0]) / 2.0, (r.min[1] + r.max[1]) / 2.0];
        let mut best = None;
        for p in [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]], c] {
            let p = [p[0] + delta[0], p[1] + delta[1]];
            if let Some((q, d)) = self.snap(p, distance)
                && best.is_none_or(|(_, old)| d < old - 1e-9)
            {
                best = Some(([delta[0] + q[0] - p[0], delta[1] + q[1] - p[1]], d));
            }
        }
        best.map_or(delta, |(p, _)| p)
    }
}
