//! Owned raster boundary graph / vector arc following, bounded per gesture.
use crate::{document::Document, scene::Point};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reference {
    #[default]
    Current,
    Composite,
    Vector(u64),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    All,
    FreehandOnly,
    PolylineOnly,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    #[default]
    SingleStroke,
    Multiple,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Freehand,
    Polyline,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub reference: Reference,
    pub scope: Scope,
    pub input: Input,
    pub distance: f64,
    pub contrast: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            reference: Reference::Current,
            scope: Scope::All,
            input: Input::SingleStroke,
            distance: 8.0,
            contrast: 16,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.distance.is_finite()
            && (0.0..=64.0).contains(&self.distance)
            && self.contrast > 0
            && !matches!(self.reference, Reference::Vector(0))
    }
    fn applies(self, kind: Segment) -> bool {
        self.enabled
            && matches!(
                (self.scope, kind),
                (Scope::All, _)
                    | (Scope::FreehandOnly, Segment::Freehand)
                    | (Scope::PolylineOnly, Segment::Polyline)
            )
    }
}
struct Raster {
    width: usize,
    height: usize,
    edges: Vec<u8>,
}
struct Vector {
    points: Vec<Point>,
    lengths: Vec<f64>,
    closed: bool,
}
enum Geometry {
    None,
    Raster(Raster),
    Vector(Vector),
}
pub struct Trace {
    settings: Settings,
    geometry: Geometry,
    work: usize,
}
fn tick(work: &mut usize, n: usize) -> Result<(), String> {
    *work = work.saturating_add(n);
    if *work > 64 * 1024 * 1024 {
        Err("Magnetic lasso work limit; draft cancelled".into())
    } else {
        Ok(())
    }
}
fn dist(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn color(p: [u8; 4]) -> [u8; 4] {
    let a = p[3] as u16;
    [
        (p[0] as u16 * a / 255) as u8,
        (p[1] as u16 * a / 255) as u8,
        (p[2] as u16 * a / 255) as u8,
        p[3],
    ]
}
impl Raster {
    fn new(doc: &Document, layer: usize, s: Settings) -> Result<Self, String> {
        let w = doc.width as usize;
        let h = doc.height as usize;
        let count = w.saturating_mul(h);
        if count > 4 * 1024 * 1024
            || doc
                .resident_bytes()
                .saturating_add(count * 6)
                .saturating_add(16 * 1024 * 1024)
                > crate::document::DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Magnetic raster graph exceeds memory/pixel budget".into());
        }
        let sampler = crate::fill_reference::Sampler::new(
            doc,
            layer,
            crate::bucket::Settings {
                reference: if s.reference == Reference::Composite {
                    crate::bucket::Reference::Composite
                } else {
                    crate::bucket::Reference::Active
                },
                ..Default::default()
            },
        )?;
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| color(sampler.pixel(x as u32, y as u32)))
            .collect::<Vec<_>>();
        let pixel = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                [0; 4]
            } else {
                pixels[y as usize * w + x as usize]
            }
        };
        let different = |a: [u8; 4], b: [u8; 4]| (0..4).any(|i| a[i].abs_diff(b[i]) >= s.contrast);
        let mut r = Self {
            width: w + 1,
            height: h + 1,
            edges: vec![0; (w + 1) * (h + 1)],
        };
        for y in 0..h {
            for x in 0..=w {
                if different(pixel(x as i64 - 1, y as i64), pixel(x as i64, y as i64)) {
                    let i = y * r.width + x;
                    r.edges[i] |= 8;
                    r.edges[i + r.width] |= 4;
                }
            }
        }
        for y in 0..=h {
            for x in 0..w {
                if different(pixel(x as i64, y as i64 - 1), pixel(x as i64, y as i64)) {
                    let i = y * r.width + x;
                    r.edges[i] |= 2;
                    r.edges[i + 1] |= 1;
                }
            }
        }
        Ok(r)
    }
    fn point(&self, i: usize) -> Point {
        [(i % self.width) as f64, (i / self.width) as f64]
    }
    fn nearest(&self, p: Point, r: f64, work: &mut usize) -> Result<Option<usize>, String> {
        let lo = [
            (p[0] - r).ceil().max(0.0) as usize,
            (p[1] - r).ceil().max(0.0) as usize,
        ];
        let hi = [
            (p[0] + r).floor().min(self.width as f64 - 1.0),
            (p[1] + r).floor().min(self.height as f64 - 1.0),
        ];
        if hi[0] < lo[0] as f64 || hi[1] < lo[1] as f64 {
            return Ok(None);
        }
        let mut best = (r + 1e-9, None);
        for y in lo[1]..=hi[1] as usize {
            for x in lo[0]..=hi[0] as usize {
                tick(work, 1)?;
                let i = y * self.width + x;
                if self.edges[i] != 0 {
                    let d = dist(p, self.point(i));
                    if d < best.0 {
                        best = (d, Some(i));
                    }
                }
            }
        }
        Ok(best.1)
    }
    fn route(&self, a: usize, b: usize, work: &mut usize) -> Result<Option<Vec<Point>>, String> {
        let mut previous = HashMap::new();
        previous.insert(a, a);
        let mut queue = VecDeque::from([a]);
        while let Some(i) = queue.pop_front() {
            tick(work, 4)?;
            if i == b {
                let mut route = vec![self.point(b)];
                let mut at = b;
                while at != a {
                    at = previous[&at];
                    route.push(self.point(at));
                }
                route.reverse();
                if route.len() > 4096 {
                    return Err("Magnetic lasso route exceeds 4096 points".into());
                }
                return Ok(Some(route));
            }
            for (bit, delta) in [
                (4, -(self.width as isize)),
                (2, 1),
                (8, self.width as isize),
                (1, -1),
            ] {
                if self.edges[i] & bit != 0 {
                    let j = (i as isize + delta) as usize;
                    if let std::collections::hash_map::Entry::Vacant(entry) = previous.entry(j) {
                        entry.insert(i);
                        queue.push_back(j);
                    }
                }
            }
            if previous.len() > 65536 {
                return Err("Magnetic boundary search exceeds 65536 vertices".into());
            }
        }
        Ok(None)
    }
}
impl Vector {
    fn new(doc: &Document, id: u64) -> Result<Self, String> {
        let (layer, path) = doc
            .scene
            .lines
            .layers
            .iter()
            .find_map(|l| l.paths.iter().find(|p| p.id == id).map(|p| (l, p)))
            .ok_or("Magnetic vector path is missing")?;
        if layer.hidden {
            return Err("Show the vector reference before tracing".into());
        }
        let mut points = path
            .points
            .iter()
            .map(|p| doc.scene.transform.map(*p))
            .collect::<Vec<_>>();
        if path.closed {
            points.push(points[0]);
        }
        let mut lengths = vec![0.0];
        for pair in points.windows(2) {
            lengths.push(lengths.last().unwrap() + dist(pair[0], pair[1]));
        }
        if lengths.last().copied().unwrap_or(0.0) <= 1e-9 {
            return Err("Magnetic vector has no length".into());
        }
        Ok(Self {
            points,
            lengths,
            closed: path.closed,
        })
    }
    fn nearest(&self, p: Point, r: f64, work: &mut usize) -> Result<Option<(Point, f64)>, String> {
        tick(work, self.points.len())?;
        let mut best = (r + 1e-9, None);
        for (i, pair) in self.points.windows(2).enumerate() {
            let (d, q) = crate::line_tools::distance(p, pair[0], pair[1]);
            if d < best.0 {
                best = (d, Some((q, self.lengths[i] + dist(pair[0], q))));
            }
        }
        Ok(best.1)
    }
    fn route(&self, a: (Point, f64), b: (Point, f64)) -> Vec<Point> {
        let total = *self.lengths.last().unwrap();
        let mut delta = b.1 - a.1;
        if self.closed {
            if delta > total / 2.0 {
                delta -= total;
            } else if delta < -total / 2.0 {
                delta += total;
            }
        }
        let end = a.1 + delta;
        let mut vertices = vec![];
        for shift in [-total, 0.0, total] {
            for (p, t) in self.points.iter().zip(&self.lengths) {
                let t = t + shift;
                if delta >= 0.0 && t > a.1 + 1e-9 && t < end - 1e-9
                    || delta < 0.0 && t < a.1 - 1e-9 && t > end + 1e-9
                {
                    vertices.push((t, *p));
                }
            }
        }
        vertices.sort_by(|a, b| {
            if delta >= 0.0 {
                a.0.total_cmp(&b.0)
            } else {
                b.0.total_cmp(&a.0)
            }
        });
        let mut route = vec![a.0];
        for (_, p) in vertices {
            if route.last() != Some(&p) {
                route.push(p);
            }
        }
        route.push(b.0);
        route
    }
}
impl Trace {
    pub fn new(doc: &Document, layer: usize, s: Settings) -> Result<Self, String> {
        if !s.valid() {
            return Err("Invalid magnetic lasso settings".into());
        }
        let geometry = if !s.enabled {
            Geometry::None
        } else {
            match s.reference {
                Reference::Vector(id) => Geometry::Vector(Vector::new(doc, id)?),
                _ => Geometry::Raster(Raster::new(doc, layer, s)?),
            }
        };
        Ok(Self {
            settings: s,
            geometry,
            work: 0,
        })
    }
    pub fn bytes(&self) -> usize {
        match &self.geometry {
            Geometry::None => 0,
            Geometry::Raster(r) => r.edges.len(),
            Geometry::Vector(v) => v.points.len() * 24,
        }
    }
    pub fn route(
        &mut self,
        from: Option<Point>,
        to: Point,
        kind: Segment,
    ) -> Result<Vec<Point>, String> {
        if from
            .iter()
            .chain(std::iter::once(&to))
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err("Invalid magnetic lasso coordinate".into());
        }
        if !self.settings.applies(kind) {
            return Ok(vec![to]);
        }
        let r = self.settings.distance;
        match &self.geometry {
            Geometry::None => Ok(vec![to]),
            Geometry::Raster(g) => {
                let Some(b) = g.nearest(to, r, &mut self.work)? else {
                    return Ok(vec![to]);
                };
                if let Some(from) = from
                    && let Some(a) = g.nearest(from, r, &mut self.work)?
                    && let Some(route) = g.route(a, b, &mut self.work)?
                {
                    return Ok(route);
                }
                Ok(vec![g.point(b)])
            }
            Geometry::Vector(g) => {
                let Some(b) = g.nearest(to, r, &mut self.work)? else {
                    return Ok(vec![to]);
                };
                if let Some(from) = from
                    && let Some(a) = g.nearest(from, r, &mut self.work)?
                {
                    return Ok(g.route(a, b));
                }
                Ok(vec![b.0])
            }
        }
    }
}
pub fn can_delete_outside(doc: &Document, layer: usize) -> bool {
    let selected = doc.scene.selection_mask.as_ref().map_or_else(
        || {
            doc.scene
                .selection
                .is_some_and(|r| r.max[0] > r.min[0] && r.max[1] > r.min[1])
        },
        |m| !m.runs.is_empty(),
    );
    selected && doc.layers.get(layer).is_some_and(|l| !l.locked)
}
pub fn delete_outside(
    doc: &Document,
    layer: usize,
) -> Result<crate::sizing::DocumentState, String> {
    if !can_delete_outside(doc, layer) {
        return Err(
            "Delete outside needs a nonempty selection and an unlocked raster target".into(),
        );
    }
    let l = &doc.layers[layer];
    if l.pixels.len() > 4 * 1024 * 1024
        || doc.resident_bytes().saturating_add(doc.memory_bytes())
            > crate::document::DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Delete outside exceeds pixel/memory budget".into());
    }
    let extent = l.extent(doc.width, doc.height);
    let mut state = doc.snapshot();
    let mut intersects = false;
    for (i, p) in state.layers[layer].pixels.iter_mut().enumerate() {
        let at = [
            extent.offset[0] as f64 + (i % extent.width as usize) as f64 + 0.5,
            extent.offset[1] as f64 + (i / extent.width as usize) as f64 + 0.5,
        ];
        let keep = doc.scene.selection_coverage(at);
        intersects |= keep > 0;
        if keep < 255 {
            p[3] = ((p[3] as u32 * keep as u32 + 127) / 255) as u8;
            if p[3] == 0 {
                *p = [0; 4];
            }
        }
    }
    if !intersects {
        return Err("Selection misses this raster; outside deletion is disabled".into());
    }
    Ok(state)
}
