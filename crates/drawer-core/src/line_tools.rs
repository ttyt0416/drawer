//! Local editable vector paths and explicitly separated line references/erase targets.
use crate::{
    document::Document,
    scene::{Affine, Point},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    Raster(usize),
    Vector(u64),
    Track(usize),
    PageObjects,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Path {
    #[serde(
        default,
        skip_serializing_if = "crate::vector_pressure::Profile::is_default"
    )]
    pub pressure: crate::vector_pressure::Profile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 4]>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub stroke: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<crate::shapes::Definition>,
    pub id: u64,
    pub points: Vec<Point>,
    pub closed: bool,
    pub width: f64,
    pub color: [u8; 4],
}
fn yes() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
impl Path {
    pub fn segments(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.points.windows(2).map(|p| (p[0], p[1])).chain(
            self.closed
                .then(|| (*self.points.last().unwrap(), self.points[0])),
        )
    }
    pub fn bounds(&self) -> crate::scene::Rect {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let radius = self.maximum_width() / 2.;
        for p in &self.points {
            for i in 0..2 {
                min[i] = min[i].min(p[i] - radius);
                max[i] = max[i].max(p[i] + radius);
            }
        }
        crate::scene::Rect { min, max }
    }
    pub fn visible_hit(&self, p: Point) -> bool {
        self.fill.is_some_and(|c| c[3] > 0)
            && self.closed
            && crate::shapes::contains(&self.points, p)
            || self.stroke && self.color[3] > 0 && self.stroke_hit(p)
    }
    pub fn hit(&self, p: Point) -> bool {
        self.fill.is_some() && self.closed && crate::shapes::contains(&self.points, p)
            || self.stroke && self.stroke_hit(p)
    }
    pub fn pixel(&self, p: Point) -> [u8; 4] {
        let fill = if self.fill.is_some() && self.closed && crate::shapes::contains(&self.points, p)
        {
            self.fill.unwrap_or([0; 4])
        } else {
            [0; 4]
        };
        if self.stroke && self.stroke_hit(p) {
            crate::render::over(fill, self.color)
        } else {
            fill
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    #[serde(
        default,
        skip_serializing_if = "crate::fill_reference::Tags::is_default"
    )]
    pub fill: crate::fill_reference::Tags,
    pub id: u64,
    pub name: String,
    pub hidden: bool,
    pub locked: bool,
    pub paths: Vec<Path>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub clip_paint: bool,
    pub references: Vec<Target>,
    pub magnet: bool,
    pub distance: f64,
    pub intersections: Vec<u64>,
    pub erase: Vec<Target>,
    pub darkness: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            clip_paint: false,
            references: vec![],
            magnet: false,
            distance: 8.0,
            intersections: vec![],
            erase: vec![],
            darkness: 32,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(
        default,
        skip_serializing_if = "crate::vector_selection::State::is_default"
    )]
    pub selection: crate::vector_selection::State,
    pub next_id: u64,
    pub layers: Vec<Layer>,
    pub settings: Settings,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            selection: Default::default(),
            next_id: 1,
            layers: vec![],
            settings: Default::default(),
        }
    }
}
fn unique<T: Ord + Copy>(v: &[T], limit: usize) -> bool {
    v.len() <= limit && v.iter().copied().collect::<BTreeSet<_>>().len() == v.len()
}
pub fn point_valid(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
}
impl Data {
    pub fn prune_selection(&mut self) {
        self.selection.prune(&self.layers);
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        let mut paths = 0;
        let mut points = 0;
        self.selection.valid(&self.layers)
            && self.next_id > 0
            && self.layers.len() <= 32
            && self.layers.iter().all(|l| {
                ids.insert(l.id)
                    && l.id > 0
                    && l.id < self.next_id
                    && !l.name.trim().is_empty()
                    && l.name.chars().count() <= 64
                    && !l.name.chars().any(char::is_control)
                    && l.paths.iter().all(|p| {
                        paths += 1;
                        points += p.points.len();
                        ids.insert(p.id)
                            && p.id > 0
                            && p.id < self.next_id
                            && p.points.len() >= if p.closed { 3 } else { 2 }
                            && p.points.len() <= 4096
                            && p.points.iter().copied().all(point_valid)
                            && (p.stroke || p.fill.is_some())
                            && (p.fill.is_none() || p.closed)
                            && p.definition
                                .as_ref()
                                .is_none_or(crate::shapes::Definition::valid)
                            && p.definition
                                .as_ref()
                                .is_none_or(|d| d.nodes.is_none() || d.settings.closed == p.closed)
                            && (0.0..=512.0).contains(&p.width)
                            && p.pressure.valid()
                    })
            })
            && paths <= 128
            && points <= 8192
            && unique(&self.settings.references, 288)
            && unique(&self.settings.intersections, 32)
            && unique(&self.settings.erase, 288)
            && (0.0..=1024.0).contains(&self.settings.distance)
            && self.settings.darkness > 0
    }
    pub fn size_bytes(&self) -> usize {
        if self.is_default() {
            return 0;
        }
        std::mem::size_of::<Self>()
            + self.selection.bytes()
            + self
                .layers
                .iter()
                .map(|l| {
                    std::mem::size_of::<Layer>()
                        + l.name.len()
                        + l.paths
                            .iter()
                            .map(|p| {
                                std::mem::size_of::<Path>()
                                    + p.points.len() * 16
                                    + p.pressure.bytes()
                                    + p.definition
                                        .as_ref()
                                        .map_or(0, crate::shapes::Definition::bytes)
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
            + (self.settings.references.len() + self.settings.erase.len())
                * std::mem::size_of::<Target>()
            + self.settings.intersections.len() * 8
    }
    pub(crate) fn id(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or("Vector ID limit")?;
        Ok(id)
    }
    pub fn add_layer(&mut self, name: &str) -> Result<u64, String> {
        let mut next = self.clone();
        let id = next.id()?;
        next.layers.push(Layer {
            fill: Default::default(),
            id,
            name: name.trim().into(),
            hidden: false,
            locked: false,
            paths: vec![],
        });
        if !next.valid() {
            return Err("Vector layer name or capacity limit".into());
        }
        *self = next;
        Ok(id)
    }
    pub fn add_path(
        &mut self,
        layer: u64,
        points: Vec<Point>,
        closed: bool,
        width: f64,
        color: [u8; 4],
    ) -> Result<u64, String> {
        let mut next = self.clone();
        let id = next.id()?;
        let l = next
            .layers
            .iter_mut()
            .find(|l| l.id == layer)
            .ok_or("Missing vector layer")?;
        if l.hidden || l.locked {
            return Err("Vector target is hidden or locked".into());
        }
        l.paths.push(Path {
            pressure: Default::default(),
            fill: None,
            stroke: true,
            definition: None,
            id,
            points,
            closed,
            width,
            color,
        });
        if !next.valid() {
            return Err("Invalid vector path or capacity limit".into());
        }
        *self = next;
        Ok(id)
    }
}
/// Nearest projection in transformed document pixels, independent of view zoom.
pub fn magnet(doc: &Document, point: Point) -> Result<Point, String> {
    snap(&doc.scene.lines, doc.scene.transform, point)
}
pub fn snap(data: &Data, transform: Affine, point: Point) -> Result<Point, String> {
    if !point_valid(point) {
        return Err("Invalid magnet coordinate".into());
    }
    let s = &data.settings;
    if !s.magnet {
        return Ok(point);
    }
    let mut best = (s.distance, point);
    for target in &s.references {
        if let Target::Vector(id) = target {
            let layer = data
                .layers
                .iter()
                .find(|l| l.id == *id)
                .ok_or("Missing vector reference")?;
            if layer.hidden {
                continue;
            }
            for path in &layer.paths {
                for (a, b) in path.segments() {
                    let (d, q) = distance(point, transform.map(a), transform.map(b));
                    if d <= best.0 {
                        best = (d, q)
                    }
                }
            }
        }
    }
    Ok(best.1)
}
pub(crate) fn maximum_scale(t: Affine) -> f64 {
    let [a, b, c, d, _, _] = t.0;
    let sum = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt()
}
pub(crate) fn distance(p: Point, a: Point, b: Point) -> (f64, Point) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = d[0] * d[0] + d[1] * d[1];
    let t = if n == 0.0 {
        0.0
    } else {
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / n
    }
    .clamp(0.0, 1.0);
    let q = [a[0] + t * d[0], a[1] + t * d[1]];
    ((p[0] - q[0]).hypot(p[1] - q[1]), q)
}
pub fn preflight(doc: &Document) -> Vec<(Target, Option<String>)> {
    preflight_data(doc, &doc.scene.lines)
}
pub fn preflight_data(doc: &Document, data: &Data) -> Vec<(Target, Option<String>)> {
    data.settings
        .erase
        .iter()
        .map(|t| {
            let error = match *t {
                Target::Raster(n) => match doc.layers.get(n) {
                    None => Some("Missing raster layer"),
                    Some(l) if l.locked => Some("Locked raster layer"),
                    Some(_) if !doc.scene.paper.raster_visible(n, doc.layers.len()) => {
                        Some("Hidden raster layer")
                    }
                    _ => None,
                },
                Target::Vector(id) => match data.layers.iter().find(|l| l.id == id) {
                    None => Some("Missing vector layer"),
                    Some(l) if l.locked => Some("Locked vector layer"),
                    Some(l) if l.hidden => Some("Hidden vector layer"),
                    Some(_)
                        if doc.scene.selection.is_some() || doc.scene.selection_mask.is_some() =>
                    {
                        Some("Clear raster selection before vector erasing")
                    }
                    _ => None,
                },
                Target::Track(_) | Target::PageObjects => Some("Unsupported erase target"),
            };
            (*t, error.map(str::to_owned))
        })
        .collect()
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let u = [b[0] - a[0], b[1] - a[1]];
    let v = [d[0] - c[0], d[1] - c[1]];
    let det = cross(u, v);
    if det.abs() < 1e-10 {
        return None;
    }
    let w = [c[0] - a[0], c[1] - a[1]];
    let t = cross(w, v) / det;
    let s = cross(w, u) / det;
    ((-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&s))
        .then_some(t.clamp(0.0, 1.0))
}
fn at(points: &[Point], lengths: &[f64], s: f64) -> (Point, f64) {
    let i = lengths
        .partition_point(|v| *v < s)
        .saturating_sub(1)
        .min(points.len() - 2);
    let n = lengths[i + 1] - lengths[i];
    let t = if n == 0.0 { 0.0 } else { (s - lengths[i]) / n }.clamp(0.0, 1.0);
    (
        [
            points[i][0] + t * (points[i + 1][0] - points[i][0]),
            points[i][1] + t * (points[i + 1][1] - points[i][1]),
        ],
        i as f64 + t,
    )
}
fn piece(
    points: &[Point],
    lengths: &[f64],
    a: f64,
    b: f64,
    count: usize,
) -> crate::vector_erase::Part {
    let (first, t) = at(points, lengths, a);
    let mut out = vec![first];
    let mut parameters = vec![t / count as f64];
    for (i, p) in points.iter().enumerate().skip(1) {
        if lengths[i] > a + 1e-8 && lengths[i] < b - 1e-8 {
            out.push(*p);
            parameters.push(i as f64 / count as f64);
        }
    }
    let (last, t) = at(points, lengths, b);
    out.push(last);
    parameters.push(t / count as f64);
    crate::vector_erase::Part {
        points: out,
        parameters,
    }
}
pub(crate) fn cut(
    path: &Path,
    transform: Affine,
    p: Point,
    r: f64,
    refs: &[(Point, Point)],
    work: &mut usize,
) -> Result<Option<Vec<crate::vector_erase::Part>>, String> {
    let count = path.points.len() - 1;
    let mut points = path.points.clone();
    if path.closed {
        points.push(points[0])
    }
    let mut lengths = vec![0.0];
    let mut best = (f64::INFINITY, 0.0);
    let mut cuts = vec![];
    for edge in points.windows(2) {
        let a = transform.map(edge[0]);
        let b = transform.map(edge[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let start = *lengths.last().unwrap();
        let (d, q) = distance(p, a, b);
        if d < best.0 {
            best = (d, start + (q[0] - a[0]).hypot(q[1] - a[1]))
        }
        for (c, d) in refs {
            *work += 1;
            if *work > 64 * 1024 * 1024 {
                return Err("Vector intersection work limit".into());
            }
            if let Some(t) = intersection(a, b, *c, *d) {
                if cuts.len() >= 32768 {
                    return Err("Vector intersection count limit; no layers changed".into());
                }
                cuts.push(start + t * len)
            }
        }
        lengths.push(start + len);
    }
    if best.0 > r + path.maximum_width() / 2.0 * maximum_scale(transform) {
        return Ok(None);
    }
    if path.fill.is_some() {
        return Err("Intersection erasing requires an outline path; filled shapes are kept".into());
    }
    let total = *lengths.last().unwrap();
    if total < 1e-8 {
        return Ok(Some(vec![]));
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    let lower = cuts.iter().copied().rfind(|v| *v < best.1 - 1e-8);
    let upper = cuts.iter().copied().find(|v| *v > best.1 + 1e-8);
    let mut result = vec![];
    if path.closed {
        if cuts.len() < 2 {
            return Ok(Some(result));
        }
        let a = lower.unwrap_or(cuts[cuts.len() - 1] - total);
        let b = upper.unwrap_or(cuts[0] + total);
        if a >= 0.0 && b <= total {
            let mut remain = piece(&points, &lengths, b, total, count);
            let tail = piece(&points, &lengths, 0.0, a, count);
            remain.points.extend(tail.points.into_iter().skip(1));
            let period = points.len().saturating_sub(1) as f64 / count as f64;
            remain
                .parameters
                .extend(tail.parameters.into_iter().skip(1).map(|t| t + period));
            result.push(remain)
        } else {
            result.push(piece(
                &points,
                &lengths,
                b.rem_euclid(total),
                a.rem_euclid(total),
                count,
            ))
        }
    } else {
        let a = lower.unwrap_or(0.0);
        let b = upper.unwrap_or(total);
        if a > 1e-8 {
            result.push(piece(&points, &lengths, 0.0, a, count))
        }
        if b < total - 1e-8 {
            result.push(piece(&points, &lengths, b, total, count))
        }
    }
    Ok(Some(result))
}
pub fn erase(doc: &mut Document, point: Point, radius: f64) -> Result<bool, String> {
    if !point_valid(point) || !(0.1..=1024.0).contains(&radius) {
        return Err("Invalid eraser point or radius".into());
    }
    let checks = preflight(doc);
    if checks.is_empty() {
        return Err("Select erase targets first".into());
    }
    if let Some((t, Some(e))) = checks.iter().find(|(_, e)| e.is_some()) {
        return Err(format!("{t:?}: {e}; no layers changed"));
    }
    let settings = &doc.scene.lines.settings;
    let mut refs = vec![];
    for id in &settings.intersections {
        let l = doc
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == *id)
            .ok_or("Missing intersection reference")?;
        if !l.hidden {
            for path in &l.paths {
                refs.extend(
                    path.segments()
                        .map(|(a, b)| (doc.scene.transform.map(a), doc.scene.transform.map(b))),
                )
            }
        }
    }
    let mut after = doc.scene.clone();
    let mut floating_layers = vec![];
    let mut deltas = vec![];
    let mut work = 0usize;
    let mut staged = after.size_bytes().saturating_add(doc.scene.size_bytes());
    let delta_limit = (crate::document::DEFAULT_DOCUMENT_BUDGET / 4)
        .min(crate::document::DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes()));
    for target in &settings.erase {
        match *target {
            Target::Raster(n) => {
                let l = &doc.layers[n];
                if doc.color != crate::color_storage::Config::legacy() {
                    let extent = l.extent(doc.width, doc.height);
                    let count = l.pixels.len();
                    work = work.saturating_add(count);
                    if count > 4 * 1024 * 1024
                        || work > 64 * 1024 * 1024
                        || doc
                            .memory_bytes()
                            .saturating_mul(2)
                            .saturating_add(count.saturating_mul(44))
                            > delta_limit
                    {
                        return Err("Floating multi-layer erase memory/work limit".into());
                    }
                    let mut samples = Vec::with_capacity(count);
                    for offset in 0..count {
                        let p = [
                            extent.offset[0] as f64 + (offset % extent.width as usize) as f64 + 0.5,
                            extent.offset[1] as f64 + (offset / extent.width as usize) as f64 + 0.5,
                        ];
                        let before = l.sample_at(offset);
                        let factor = if p[0] >= 0.
                            && p[1] >= 0.
                            && p[0] < doc.width as f64
                            && p[1] < doc.height as f64
                            && (p[0] - point[0]).hypot(p[1] - point[1]) <= radius
                        {
                            doc.scene.selection_coverage(p) as f64 / 255.
                        } else {
                            0.
                        };
                        samples.push(before.coverage(1. - factor)?);
                    }
                    let mut raster = l.clone();
                    crate::color_storage::set_samples(&mut raster, &doc.color, extent, samples)?;
                    floating_layers.push((n, raster));
                    continue;
                }
                let mut changes = vec![];
                let low = [
                    (point[0] - radius).floor().max(0.0) as i64,
                    (point[1] - radius).floor().max(0.0) as i64,
                ];
                let high = [
                    (point[0] + radius).ceil().min(doc.width as f64) as i64,
                    (point[1] + radius).ceil().min(doc.height as f64) as i64,
                ];
                for y in low[1]..high[1] {
                    for x in low[0]..high[0] {
                        work += 1;
                        if work > 64 * 1024 * 1024 {
                            return Err("Multi-layer erase work limit".into());
                        }
                        let p = [x as f64 + 0.5, y as f64 + 0.5];
                        if (p[0] - point[0]).hypot(p[1] - point[1]) > radius {
                            continue;
                        }
                        if let Some(offset) = l.pixel_index(doc.width, doc.height, x, y) {
                            let before = l.pixels[offset];
                            let mut pixel = before;
                            pixel[3] = (pixel[3] as f64
                                * (1.0 - doc.scene.selection_coverage(p) as f64 / 255.0))
                                .round() as u8;
                            if pixel != before {
                                staged = staged.saturating_add(std::mem::size_of::<
                                    crate::history::PixelChange,
                                >());
                                if staged > delta_limit {
                                    return Err(
                                        "Multi-layer delta memory limit; no layers changed".into(),
                                    );
                                }
                                changes.push(crate::history::PixelChange {
                                    offset,
                                    before,
                                    after: pixel,
                                })
                            }
                        }
                    }
                }
                deltas.push((n, changes));
            }
            Target::Vector(id) => {
                // A partially selected vector span is not representable as an editable path.
                if doc.scene.selection.is_some() || doc.scene.selection_mask.is_some() {
                    return Err(
                        "Clear the raster selection before vector erasing; no layers changed"
                            .into(),
                    );
                }
                let i = after.lines.layers.iter().position(|l| l.id == id).unwrap();
                let mut paths = vec![];
                for path in after.lines.layers[i].paths.clone() {
                    if let Some(parts) =
                        cut(&path, doc.scene.transform, point, radius, &refs, &mut work)?
                    {
                        for part in parts {
                            let id = after.lines.id()?;
                            let pressure = path.fragment_pressure_at(&part.parameters)?;
                            paths.push(Path {
                                pressure,
                                fill: None,
                                stroke: true,
                                definition: None,
                                id,
                                points: part.points,
                                closed: false,
                                width: path.width,
                                color: path.color,
                            })
                        }
                    } else {
                        paths.push(path)
                    }
                }
                after.lines.layers[i].paths = paths;
            }
            _ => unreachable!(),
        }
    }
    after.lines.prune_selection();
    if !after.valid() {
        return Err("Erasing exceeds vector path limits; no layers changed".into());
    }
    if doc.color != crate::color_storage::Config::legacy() {
        let mut state = doc.snapshot();
        state.scene = after;
        for (n, layer) in floating_layers {
            state.layers[n] = layer;
        }
        return doc.commit_state(state);
    }
    doc.commit_multi(deltas, (after != doc.scene).then_some(after))
}
