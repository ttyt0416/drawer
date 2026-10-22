//! Separate vector intersection references, edit targets and geometric erasing modes.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::{Path, Target},
    scene::{Affine, Point},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    None,
    Touched,
    #[default]
    Intersection,
    Whole,
}
impl Mode {
    pub const ALL: [Self; 4] = [Self::None, Self::Touched, Self::Intersection, Self::Whole];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Do not erase",
            Self::Touched => "Erase touched areas",
            Self::Intersection => "Erase up to intersection",
            Self::Whole => "Whole line",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub refer_all: bool,
    pub erase_all: bool,
    pub radius: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Intersection,
            refer_all: false,
            erase_all: false,
            radius: 4.,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.radius.is_finite() && (0.1..=1024.).contains(&self.radius)
    }
}
pub(crate) struct Reference {
    pub path: u64,
    pub edge: usize,
    pub a: Point,
    pub b: Point,
}
pub(crate) struct Part {
    pub points: Vec<Point>,
    pub parameters: Vec<f64>,
}
fn period(path: &Path) -> f64 {
    if path.closed {
        path.points.len() as f64 / (path.points.len() - 1) as f64
    } else {
        1.
    }
}
fn geometry_at(path: &Path, t: f64) -> Point {
    let n = path.points.len() - 1;
    let p = period(path);
    let t = if path.closed {
        t.rem_euclid(p)
    } else {
        t.clamp(0., 1.)
    };
    let x = t * n as f64;
    let i = (x.floor() as usize).min(if path.closed { n } else { n - 1 });
    let f = (x - i as f64).clamp(0., 1.);
    let a = path.points[i];
    let b = path.points[(i + 1) % path.points.len()];
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
}
pub(crate) fn part(path: &Path, a: f64, b: f64) -> Result<Part, String> {
    if !a.is_finite() || !b.is_finite() || b <= a {
        return Err("Invalid vector fragment interval".into());
    }
    let count = (path.points.len() - 1) as f64;
    let mut parameters = vec![a];
    let mut points = vec![geometry_at(path, a)];
    let lo = (a * count).floor() as usize + 1;
    let hi = (b * count).ceil() as usize;
    for n in lo..hi {
        let t = n as f64 / count;
        if t > a + 1e-12 && t < b - 1e-12 {
            parameters.push(t);
            points.push(path.points[n % path.points.len()]);
        }
    }
    parameters.push(b);
    points.push(geometry_at(path, b));
    if parameters.len() > 4096 {
        return Err("Vector fragment vertex limit".into());
    }
    Ok(Part { points, parameters })
}
fn crossing(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let u = [b[0] - a[0], b[1] - a[1]];
    let v = [d[0] - c[0], d[1] - c[1]];
    let w = [c[0] - a[0], c[1] - a[1]];
    let det = u[0] * v[1] - u[1] * v[0];
    if det.abs() < 1e-10 {
        return None;
    }
    let t = (w[0] * v[1] - w[1] * v[0]) / det;
    let s = (w[0] * u[1] - w[1] * u[0]) / det;
    ((-1e-9..=1. + 1e-9).contains(&t) && (-1e-9..=1. + 1e-9).contains(&s))
        .then_some(t.clamp(0., 1.))
}
fn intervals(a: Point, b: Point, point: Point, r0: f64, r1: f64) -> Vec<(f64, f64)> {
    let q = [a[0] - point[0], a[1] - point[1]];
    let v = [b[0] - a[0], b[1] - a[1]];
    let dr = r1 - r0;
    let aa = v[0] * v[0] + v[1] * v[1] - dr * dr;
    let bb = 2. * (q[0] * v[0] + q[1] * v[1] - r0 * dr);
    let cc = q[0] * q[0] + q[1] * q[1] - r0 * r0;
    let mut cuts = vec![0., 1.];
    if aa.abs() < 1e-18 {
        if bb.abs() > 1e-18 {
            let t = -cc / bb;
            if t > 0. && t < 1. {
                cuts.push(t);
            }
        }
    } else {
        // Evaluate at the quadratic's center rather than subtracting two
        // enormous squared discriminant terms for long off-page segments.
        let center = -bb / (2. * aa);
        let qx = q[0] + v[0] * center;
        let qy = q[1] + v[1] * center;
        let r = r0 + dr * center;
        let square = -(qx * qx + qy * qy - r * r) / aa;
        if square >= 0. {
            for t in [center - square.sqrt(), center + square.sqrt()] {
                if t > 0. && t < 1. {
                    cuts.push(t);
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    cuts.windows(2)
        .filter_map(|w| {
            let t = (w[0] + w[1]) / 2.;
            ((q[0] + v[0] * t).hypot(q[1] + v[1] * t) <= r0 + dr * t + 1e-9).then_some((w[0], w[1]))
        })
        .collect()
}
struct EllipseFrame {
    polygon: Option<[Point; 4]>,
    cos: f64,
    sin: f64,
    major: f64,
    minor: f64,
}
impl EllipseFrame {
    fn transformed(t: Affine, tip: crate::vector_tip::Preset) -> Option<Self> {
        if let Some(vertices) = tip.polygon() {
            return Some(Self {
                polygon: Some(vertices.map(|v| t.vector(v))),
                cos: 1.,
                sin: 0.,
                major: 1.,
                minor: 1.,
            });
        }
        let t = tip.frame().then(t);
        let [a, b, c, d, _, _] = t.0;
        let xx = a * a + c * c;
        let yy = b * b + d * d;
        let xy = a * b + c * d;
        if (xx - yy).abs() + 2. * xy.abs() <= (xx + yy) * 1e-12 {
            return None;
        }
        let angle = 0.5 * (2. * xy).atan2(xx - yy);
        let (sin, cos) = angle.sin_cos();
        let major = ((xx + yy + (xx - yy).hypot(2. * xy)) / 2.).sqrt();
        Some(Self {
            polygon: None,
            cos,
            sin,
            major,
            minor: (a * d - b * c).abs() / major,
        })
    }
    fn distance(&self, q: Point, radius: f64, work: &mut usize) -> f64 {
        *work += 1;
        if *work > 64 * 1024 * 1024 {
            return f64::INFINITY;
        }
        if let Some(vertices) = self.polygon {
            let vertices = vertices.map(|v| [v[0] * radius, v[1] * radius]);
            if crate::shapes::contains(&vertices, q) {
                return 0.;
            }
            *work += 4;
            return (0..4)
                .map(|i| crate::line_tools::distance(q, vertices[i], vertices[(i + 1) % 4]).0)
                .fold(f64::INFINITY, f64::min);
        }
        let a = self.major * radius;
        let b = self.minor * radius;
        if a < 1e-12 {
            return q[0].hypot(q[1]);
        }
        let x = (q[0] * self.cos + q[1] * self.sin).abs();
        let y = (-q[0] * self.sin + q[1] * self.cos).abs();
        if (x / a).powi(2) + (y / b).powi(2) <= 1. {
            return 0.;
        }
        let mut lo = 0.;
        let mut hi = (a * x).hypot(b * y);
        let aa = a * a;
        let bb = b * b;
        for _ in 0..64 {
            let m = (lo + hi) / 2.;
            let f = (a * x / (m + aa)).powi(2) + (b * y / (m + bb)).powi(2);
            if f > 1. {
                lo = m;
            } else {
                hi = m;
            }
        }
        *work += 64;
        let lambda = (lo + hi) / 2.;
        (x - aa * x / (lambda + aa)).hypot(y - bb * y / (lambda + bb))
    }
    fn contact(
        &self,
        a: Point,
        b: Point,
        point: Point,
        radii: [f64; 2],
        radius: f64,
        work: &mut usize,
    ) -> Vec<(f64, f64)> {
        let mut distance = |t: f64| {
            self.distance(
                [
                    point[0] - a[0] - (b[0] - a[0]) * t,
                    point[1] - a[1] - (b[1] - a[1]) * t,
                ],
                radii[0] + (radii[1] - radii[0]) * t,
                work,
            )
        };
        // Distance to an affinely translated, linearly scaled convex disk has
        // convex sublevel sets. Find its feasible interval and both boundaries.
        let golden = (5_f64.sqrt() - 1.) / 2.;
        let mut lo = 0.;
        let mut hi = 1.;
        let mut c = hi - golden * (hi - lo);
        let mut d = lo + golden * (hi - lo);
        let mut fc = distance(c);
        let mut fd = distance(d);
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        for _ in 0..64 {
            if (hi - lo) * length < 1e-7 {
                break;
            }
            if fc < fd {
                hi = d;
                d = c;
                fd = fc;
                c = hi - golden * (hi - lo);
                fc = distance(c);
            } else {
                lo = c;
                c = d;
                fc = fd;
                d = lo + golden * (hi - lo);
                fd = distance(d);
            }
        }
        let mut middle = (lo + hi) / 2.;
        let mut minimum = distance(middle);
        let left = distance(0.);
        let right = distance(1.);
        if left < minimum {
            middle = 0.;
            minimum = left;
        }
        if right < minimum {
            middle = 1.;
            minimum = right;
        }
        let threshold = radius + 1e-8;
        if minimum > threshold {
            return vec![];
        }
        let start = if left <= threshold {
            0.
        } else {
            let (mut a, mut b) = (0., middle);
            for _ in 0..50 {
                let t = (a + b) / 2.;
                if distance(t) <= threshold {
                    b = t;
                } else {
                    a = t;
                }
            }
            b
        };
        let end = if right <= threshold {
            1.
        } else {
            let (mut a, mut b) = (middle, 1.);
            for _ in 0..50 {
                let t = (a + b) / 2.;
                if distance(t) <= threshold {
                    a = t;
                } else {
                    b = t;
                }
            }
            a
        };
        if end > start + 1e-12 {
            vec![(start, end)]
        } else {
            vec![]
        }
    }
}
pub(crate) fn cut(
    path: &Path,
    transform: Affine,
    point: Point,
    radius: f64,
    mode: Mode,
    refs: &[Reference],
    work: &mut usize,
) -> Result<Option<Vec<Part>>, String> {
    if mode == Mode::Whole && path.closed && path.fill_pixel(transform.inverse().map(point))[3] > 0
    {
        return Ok(Some(vec![]));
    }
    if mode == Mode::None || !path.stroke || path.color[3] == 0 || path.maximum_width() <= 0. {
        return Ok(None);
    }
    let scale = crate::line_tools::maximum_scale(path.tip.frame().then(transform));
    let ellipse = EllipseFrame::transformed(transform, path.tip);
    let mut touched = vec![];
    for (t0, t1, a, b, wa, wb) in path.parameter_segments() {
        *work += 1;
        if *work > 64 * 1024 * 1024 {
            return Err("Vector erase work limit; nothing changed".into());
        }
        if wa <= 0. && wb <= 0. {
            continue;
        }
        let a = transform.map(a);
        let b = transform.map(b);
        let spans = if let Some(ellipse) = &ellipse {
            ellipse.contact(a, b, point, [wa / 2., wb / 2.], radius, work)
        } else {
            intervals(
                a,
                b,
                point,
                radius + wa * scale / 2.,
                radius + wb * scale / 2.,
            )
        };
        if *work > 64 * 1024 * 1024 {
            return Err("Transformed eraser work limit; nothing changed".into());
        }
        for (u, v) in spans {
            touched.push((t0 + (t1 - t0) * u, t0 + (t1 - t0) * v));
        }
    }
    if touched.is_empty() {
        return Ok(None);
    }
    if mode == Mode::Whole {
        return Ok(Some(vec![]));
    }
    if path.fill.is_some() {
        return Err(
            "Touched/intersection erasing needs an outline; choose Whole line for a filled path"
                .into(),
        );
    }
    let end = period(path);
    if mode == Mode::Intersection {
        let count = path.points.len() - 1;
        let edges = if path.closed { count + 1 } else { count };
        let mut best = (f64::INFINITY, 0.);
        let mut cuts = vec![];
        for (n, (a, b)) in path.segments().enumerate() {
            let a = transform.map(a);
            let b = transform.map(b);
            let (distance, q) = crate::line_tools::distance(point, a, b);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if distance < best.0 {
                best = (
                    distance,
                    (n as f64
                        + if length > 0. {
                            (q[0] - a[0]).hypot(q[1] - a[1]) / length
                        } else {
                            0.
                        })
                        / count as f64,
                );
            }
            for r in refs {
                *work += 1;
                if *work > 64 * 1024 * 1024 {
                    return Err("Vector intersection work limit".into());
                }
                let gap = n.abs_diff(r.edge);
                if r.path == path.id && (gap <= 1 || path.closed && gap + 1 == edges) {
                    continue;
                }
                if let Some(t) = crossing(a, b, r.a, r.b) {
                    cuts.push((n as f64 + t) / count as f64);
                    if cuts.len() > 32768 {
                        return Err("Vector intersection count limit".into());
                    }
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        if path.closed {
            cuts.retain(|t| *t < end - 1e-9);
            if cuts.len() < 2 {
                return Ok(Some(vec![]));
            }
            let i = cuts.partition_point(|t| *t <= best.1 + 1e-10);
            let a = if i == 0 {
                cuts[cuts.len() - 1] - end
            } else {
                cuts[i - 1]
            };
            let b = if i == cuts.len() {
                cuts[0] + end
            } else {
                cuts[i]
            };
            let start = b.rem_euclid(end);
            let mut finish = a.rem_euclid(end);
            if finish <= start {
                finish += end;
            }
            return Ok(Some(vec![part(path, start, finish)?]));
        }
        cuts.push(0.);
        cuts.push(end);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let i = cuts
            .partition_point(|t| *t <= best.1 + 1e-10)
            .saturating_sub(1)
            .min(cuts.len() - 2);
        touched = vec![(cuts[i], cuts[i + 1])];
    }
    touched.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = vec![];
    for span in touched {
        if let Some(last) = merged.last_mut()
            && span.0 <= last.1 + 1e-10
        {
            last.1 = last.1.max(span.1);
        } else {
            merged.push(span);
        }
    }
    let mut spans = vec![];
    let mut cursor = 0.;
    for (a, b) in merged {
        if a > cursor + 1e-10 {
            spans.push((cursor, a));
        }
        cursor = cursor.max(b);
    }
    if cursor < end - 1e-10 {
        spans.push((cursor, end));
    }
    if path.closed
        && spans.len() > 1
        && spans[0].0 == 0.
        && spans.last().is_some_and(|s| (s.1 - end).abs() < 1e-10)
    {
        let first = spans.remove(0);
        let last = spans.pop().unwrap();
        spans.push((last.0, first.1 + end));
    }
    Ok(Some(
        spans
            .into_iter()
            .map(|(a, b)| part(path, a, b))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}
fn editable(d: &Document, t: Target) -> bool {
    let target = match t {
        Target::Raster(n) if n < d.layers.len() => Some(crate::layer_properties::raster(d, n)),
        Target::Vector(id) if d.scene.lines.layers.iter().any(|l| l.id == id) => {
            Some(crate::layer_properties::Target::Vector(id))
        }
        _ => None,
    };
    let Some(t) = target else {
        return false;
    };
    let flags = crate::layer_properties::inherited(d, t);
    let alpha = match t {
        crate::layer_properties::Target::Raster(id) => {
            crate::layers::index(d, id).is_some_and(|n| crate::clipping::alpha_locked(d, n))
        }
        _ => d.scene.clipping.alpha.contains(&t),
    };
    !flags.hidden
        && !flags.locked
        && !flags.pixels_locked
        && !alpha
        && d.scene
            .file_objects
            .get(crate::layer_search::Key::Layer(t))
            .is_none()
}
fn references(d: &Document, target: u64, all: bool) -> Vec<Reference> {
    let ids = &d.scene.lines.settings.intersections;
    d.scene
        .lines
        .layers
        .iter()
        .filter(|l| {
            (all || l.id == target || ids.contains(&l.id))
                && !crate::layer_properties::inherited(
                    d,
                    crate::layer_properties::Target::Vector(l.id),
                )
                .hidden
        })
        .flat_map(|l| l.paths.iter())
        .filter(|p| p.stroke && p.color[3] > 0 && p.maximum_width() > 0.)
        .flat_map(|p| {
            p.segments()
                .enumerate()
                .map(move |(edge, (a, b))| Reference {
                    path: p.id,
                    edge,
                    a: d.scene.transform.map(a),
                    b: d.scene.transform.map(b),
                })
        })
        .collect()
}
pub fn prepare(
    d: &Document,
    target: Option<Target>,
    point: Point,
    s: Settings,
) -> Result<DocumentState, String> {
    if !s.valid()
        || !crate::line_tools::point_valid(point)
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid eraser settings or snapshot budget".into());
    }
    let targets = if s.erase_all {
        (0..d.layers.len())
            .map(Target::Raster)
            .chain(d.scene.lines.layers.iter().map(|l| Target::Vector(l.id)))
            .filter(|t| editable(d, *t))
            .collect::<Vec<_>>()
    } else {
        let t = target.ok_or("Choose a raster or vector layer")?;
        if !editable(d, t) {
            return Err(
                "Erase target is hidden, locked, alpha protected or a read-only source".into(),
            );
        }
        vec![t]
    };
    let mut state = d.snapshot();
    let rasters = targets
        .iter()
        .filter(|t| matches!(t, Target::Raster(_)))
        .copied()
        .collect::<Vec<_>>();
    if !rasters.is_empty() {
        let mut preview = crate::layer_merge::preview(&state, d.current_frame)?;
        preview.scene.lines.settings.erase = rasters;
        crate::line_tools::erase(&mut preview, point, s.radius)?;
        state = preview.snapshot();
        state.scene.lines.settings = d.scene.lines.settings.clone();
    }
    let mut work = 0;
    for target in targets {
        let Target::Vector(id) = target else {
            continue;
        };
        if s.mode == Mode::None {
            continue;
        }
        if d.scene.selection.is_some() || d.scene.selection_mask.is_some() {
            return Err("Clear the raster selection before erasing vectors".into());
        }
        let refs = references(d, id, s.refer_all);
        let i = state
            .scene
            .lines
            .layers
            .iter()
            .position(|l| l.id == id)
            .ok_or("Missing vector target")?;
        let mut paths = vec![];
        for path in state.scene.lines.layers[i].paths.clone() {
            if let Some(parts) = cut(
                &path,
                d.scene.transform,
                point,
                s.radius,
                s.mode,
                &refs,
                &mut work,
            )? {
                for part in parts {
                    let pressure = path.fragment_pressure_at(&part.parameters)?;
                    let id = state.scene.lines.id()?;
                    paths.push(Path {
                        svg: path.svg.clone(),
                        tip: path.tip,
                        pressure,
                        points: part.points,
                        id,
                        closed: false,
                        width: path.width,
                        color: path.color,
                        fill: None,
                        stroke: true,
                        definition: None,
                    });
                }
            } else {
                paths.push(path);
            }
        }
        state.scene.lines.layers[i].paths = paths;
    }
    state.scene.lines.prune_selection();
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Vector erase geometry/Undo limit; nothing changed".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    Ok(state)
}
pub fn delete(
    d: &Document,
    layer: u64,
    selection: &crate::vector_edit::Selection,
) -> Result<DocumentState, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Vector deletion snapshot limit".into());
    }
    if !editable(d, Target::Vector(layer)) {
        return Err("Choose an editable vector layer".into());
    }
    let ids = selection.path_ids();
    if ids.is_empty() {
        return Err("Select whole strokes first".into());
    }
    let mut state = d.snapshot();
    let l = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .ok_or("Missing vector layer")?;
    if ids.iter().any(|id| !l.paths.iter().any(|p| p.id == *id)) {
        return Err("Selection changed; select current strokes".into());
    }
    l.paths.retain(|p| !ids.contains(&p.id));
    state.scene.lines.prune_selection();
    crate::layer_properties::validate_edit(d, &state)?;
    Ok(state)
}
