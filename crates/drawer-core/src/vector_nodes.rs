//! Independent cubic nodes; existing sampled outlines convert without changing their vertices.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::Path,
    scene::{Affine, Point},
    shapes::Definition,
    sizing::DocumentState,
    vector_edit::{Hit, Patch, Selection},
    vector_selection::{Control, Kind, State},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Add,
    Delete,
    Corner,
    Smooth,
}
impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "Add node",
            Self::Delete => "Delete node",
            Self::Corner => "Corner node",
            Self::Smooth => "Smooth node",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub incoming: Vec<Point>,
    pub outgoing: Vec<Point>,
    pub smooth: Vec<bool>,
}
fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: Point, t: f64) -> Point {
    [a[0] * t, a[1] * t]
}
fn lerp(a: Point, b: Point, t: f64) -> Point {
    add(a, mul(sub(b, a), t))
}
fn length(a: Point) -> f64 {
    a[0].hypot(a[1])
}
fn cubic(a: Point, b: Point, c: Point, d: Point, t: f64) -> Point {
    let ab = lerp(a, b, t);
    let bc = lerp(b, c, t);
    let cd = lerp(c, d, t);
    lerp(lerp(ab, bc, t), lerp(bc, cd, t), t)
}
pub struct Sampled {
    pub points: Vec<Point>,
    pub node_indices: Vec<usize>,
    steps: Vec<usize>,
}
impl Data {
    pub fn valid(&self, controls: &[Point]) -> bool {
        let n = controls.len();
        (2..=4096).contains(&n)
            && self.incoming.len() == n
            && self.outgoing.len() == n
            && self.smooth.len() == n
            && controls.iter().copied().all(crate::line_tools::point_valid)
            && self
                .incoming
                .iter()
                .chain(&self.outgoing)
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 2e9)
            && controls
                .iter()
                .zip(self.incoming.iter().zip(&self.outgoing))
                .all(|(p, (a, b))| {
                    crate::line_tools::point_valid(add(*p, *a))
                        && crate::line_tools::point_valid(add(*p, *b))
                })
            && (0..n).all(|i| {
                !self.smooth[i] || {
                    let a = self.incoming[i];
                    let b = self.outgoing[i];
                    let scale = length(a) * length(b);
                    (a[0] * b[1] - a[1] * b[0]).abs() <= 1e-8 * scale.max(1.)
                        && a[0] * b[0] + a[1] * b[1] <= 1e-8 * scale.max(1.)
                }
            })
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + (self.incoming.len() + self.outgoing.len()) * 16
            + self.smooth.len()
    }
    fn polyline(p: &[Point], closed: bool) -> Self {
        let n = p.len();
        let mut incoming = vec![[0.; 2]; n];
        let mut outgoing = incoming.clone();
        for i in 0..n {
            if i > 0 || closed {
                incoming[i] = mul(sub(p[(i + n - 1) % n], p[i]), 1. / 3.);
            }
            if i + 1 < n || closed {
                outgoing[i] = mul(sub(p[(i + 1) % n], p[i]), 1. / 3.);
            }
        }
        Self {
            incoming,
            outgoing,
            smooth: vec![false; n],
        }
    }
    pub fn sample(&self, p: &[Point], closed: bool) -> Result<Sampled, String> {
        if !self.valid(p) || closed && p.len() < 3 {
            return Err("Invalid cubic node data".into());
        }
        let edges = if closed { p.len() } else { p.len() - 1 };
        let mut points = vec![];
        let mut node_indices = vec![];
        let mut steps = vec![];
        for i in 0..edges {
            let j = (i + 1) % p.len();
            let delta = sub(p[j], p[i]);
            let canonical = mul(delta, 1. / 3.);
            let linear = length(sub(self.outgoing[i], canonical))
                + length(add(self.incoming[j], canonical))
                <= 1e-12 * length(delta).max(1.);
            let count = if linear { 1 } else { 32 };
            if points.len() + count + usize::from(!closed) > 4096 {
                return Err("Cubic node geometry limit4096; simplify the outline first".into());
            }
            node_indices.push(points.len());
            steps.push(count);
            for n in 0..count {
                points.push(if n == 0 {
                    p[i]
                } else {
                    cubic(
                        p[i],
                        add(p[i], self.outgoing[i]),
                        add(p[j], self.incoming[j]),
                        p[j],
                        n as f64 / count as f64,
                    )
                });
            }
        }
        if !closed {
            node_indices.push(points.len());
            points.push(*p.last().unwrap());
        }
        if !points.iter().copied().all(crate::line_tools::point_valid) {
            return Err("Node geometry coordinates exceed limits".into());
        }
        Ok(Sampled {
            points,
            node_indices,
            steps,
        })
    }
}
pub fn definition(path: &Path) -> Result<Definition, String> {
    if let Some(def) = &path.definition
        && def.nodes.is_some()
    {
        return Ok(def.clone());
    }
    let controls = path.points.clone();
    Ok(Definition {
        nodes: Some(Data::polyline(&controls, path.closed)),
        controls,
        creation: Affine::default(),
        snapped_controls: true,
        settings: crate::shapes::Settings {
            kind: crate::shapes::Kind::Polyline,
            curve: crate::shapes::Curve::CubicBezier,
            closed: path.closed,
            width: path.width.clamp(0.1, 512.),
            line: path.color,
            paint: if path.fill.is_some() {
                if path.stroke {
                    crate::shapes::Paint::CreateBoth
                } else {
                    crate::shapes::Paint::CreateFill
                }
            } else {
                crate::shapes::Paint::CreateLine
            },
            fill: path.fill.unwrap_or([0; 4]),
            ..Default::default()
        },
    })
}
pub fn controls(path: &Path) -> Vec<Point> {
    if let Some(def) = &path.definition
        && def.nodes.is_some()
    {
        def.controls
            .iter()
            .map(|p| def.creation.inverse().map(*p))
            .collect()
    } else {
        path.points.clone()
    }
}
pub fn move_controls(path: &mut Path, indices: &[usize], delta: Point) -> Result<(), String> {
    transform_controls(path, indices, Affine([1., 0., 0., 1., delta[0], delta[1]]))
}
pub fn transform_controls(
    path: &mut Path,
    indices: &[usize],
    matrix: Affine,
) -> Result<(), String> {
    if !matrix.valid() {
        return Err("Invalid control transform".into());
    }
    let old = path.clone();
    let mut def = definition(path)?;
    let mut data = def.nodes.clone().unwrap();
    let sample = data.sample(&def.controls, path.closed)?;
    if sample.points.len() != old.points.len() {
        return Err("Cached node geometry changed".into());
    }
    let count = old.points.len() - 1;
    let period = if old.closed {
        old.points.len() as f64 / count as f64
    } else {
        1.
    };
    let anchors = sample
        .node_indices
        .iter()
        .map(|n| *n as f64 / count as f64)
        .collect::<Vec<_>>();
    let translation = matrix.0[..4] == [1., 0., 0., 1.];
    let local = if translation {
        let shift = def.creation.vector([matrix.0[4], matrix.0[5]]);
        Affine([1., 0., 0., 1., shift[0], shift[1]])
    } else {
        def.creation.inverse().then(matrix).then(def.creation)
    };
    for n in indices {
        let p = def.controls.get_mut(*n).ok_or("Node selection changed")?;
        *p = local.map(*p);
        if !translation {
            data.incoming[*n] = local.vector(data.incoming[*n]);
            data.outgoing[*n] = local.vector(data.outgoing[*n]);
        }
    }
    let sample = data.sample(&def.controls, path.closed)?;
    let pressure = old.fragment_pressure_at(&sources(&sample, &anchors, path.closed, period))?;
    path.points = sample
        .points
        .into_iter()
        .map(|p| def.creation.inverse().map(p))
        .collect();
    path.pressure = pressure;
    def.nodes = Some(data);
    def.snapped_controls = true;
    def.settings.constraint = crate::shapes::Constraint::Free;
    path.definition = Some(def);
    Ok(())
}
pub fn hits(d: &Document, layer: u64, point: Point, radius: f64, op: Operation) -> Vec<Hit> {
    let Some(l) = d.scene.lines.layers.iter().find(|l| l.id == layer) else {
        return vec![];
    };
    let mut hits = vec![];
    for path in l.paths.iter().rev() {
        let Ok(def) = definition(path) else {
            continue;
        };
        let nodes = def.nodes.as_ref().unwrap();
        if op == Operation::Add {
            let Ok(sample) = nodes.sample(&def.controls, path.closed) else {
                continue;
            };
            let mut start = 0;
            for (i, count) in sample.steps.iter().copied().enumerate() {
                let mut best = (f64::INFINITY, 0., point);
                for n in 0..count {
                    let a = d
                        .scene
                        .transform
                        .map(def.creation.inverse().map(sample.points[start + n]));
                    let b = d.scene.transform.map(
                        def.creation
                            .inverse()
                            .map(sample.points[(start + n + 1) % sample.points.len()]),
                    );
                    let (distance, near) = crate::line_tools::distance(point, a, b);
                    let len = length(sub(b, a));
                    let fraction = if len > 0. {
                        length(sub(near, a)) / len
                    } else {
                        0.
                    };
                    if distance < best.0 {
                        best = (distance, (n as f64 + fraction) / count as f64, near);
                    }
                }
                if best.0 <= radius && best.1 > 1e-6 && best.1 < 1. - 1e-6 {
                    hits.push(Hit {
                        path: path.id,
                        control: Some(i as u16),
                        point: best.2,
                        distance: best.0,
                        parameter: Some(best.1),
                    });
                }
                start += count;
            }
        } else {
            for (n, p) in def.controls.iter().enumerate() {
                let p = d.scene.transform.map(def.creation.inverse().map(*p));
                let distance = length(sub(p, point));
                if distance <= radius {
                    hits.push(Hit {
                        path: path.id,
                        control: Some(n as u16),
                        point: p,
                        distance,
                        parameter: None,
                    });
                }
            }
        }
    }
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits
}
fn sources(sample: &Sampled, anchors: &[f64], closed: bool, period: f64) -> Vec<f64> {
    let mut out = vec![];
    for (i, count) in sample.steps.iter().copied().enumerate() {
        let j = (i + 1) % anchors.len();
        let end = if closed && j == 0 {
            anchors[0] + period
        } else {
            anchors[j]
        };
        for n in 0..count {
            out.push(anchors[i] + (end - anchors[i]) * n as f64 / count as f64);
        }
    }
    if !closed {
        out.push(*anchors.last().unwrap());
    }
    out
}
pub fn prepare(d: &Document, layer: u64, hit: Hit, op: Operation) -> Result<DocumentState, String> {
    if crate::layer_properties::inherited(d, crate::layer_properties::Target::Vector(layer))
        .position_locked
    {
        return Err("Node positions are locked".into());
    }
    let mut state = crate::vector_edit::prepare(
        d,
        layer,
        &Selection {
            paths: vec![hit.path],
            controls: vec![],
        },
        Patch::default(),
    )?;
    let original = d
        .scene
        .lines
        .layers
        .iter()
        .find(|l| l.id == layer)
        .and_then(|l| l.paths.iter().find(|p| p.id == hit.path))
        .ok_or("Select a current vector path")?;
    let mut def = definition(original)?;
    let mut data = def.nodes.take().unwrap();
    let old_sample = data.sample(&def.controls, original.closed)?;
    if old_sample.points.len() != original.points.len() {
        return Err("Cached node geometry changed; restore the current outline first".into());
    }
    let old_count = original.points.len() - 1;
    let period = if original.closed {
        original.points.len() as f64 / old_count as f64
    } else {
        1.
    };
    let mut anchors = old_sample
        .node_indices
        .iter()
        .map(|n| *n as f64 / old_count as f64)
        .collect::<Vec<_>>();
    let i = hit.control.ok_or("Choose a current node or segment")? as usize;
    if i >= def.controls.len() {
        return Err("Node selection changed".into());
    }
    let selected = match op {
        Operation::Add => {
            let t = hit.parameter.ok_or("Choose a point inside a segment")?;
            if !t.is_finite()
                || !(1e-6..=1. - 1e-6).contains(&t)
                || !original.closed && i + 1 == def.controls.len()
                || def.controls.len() == 4096
            {
                return Err("Cannot insert at an endpoint or exceed4096 nodes".into());
            }
            let j = (i + 1) % def.controls.len();
            let a = def.controls[i];
            let b = add(a, data.outgoing[i]);
            let e = def.controls[j];
            let c = add(e, data.incoming[j]);
            let ab = lerp(a, b, t);
            let bc = lerp(b, c, t);
            let ce = lerp(c, e, t);
            let abc = lerp(ab, bc, t);
            let bce = lerp(bc, ce, t);
            let p = lerp(abc, bce, t);
            data.outgoing[i] = sub(ab, a);
            data.incoming[j] = sub(ce, e);
            data.incoming.insert(i + 1, sub(abc, p));
            data.outgoing.insert(i + 1, sub(bce, p));
            data.smooth.insert(i + 1, true);
            def.controls.insert(i + 1, p);
            let end = if original.closed && j == 0 {
                anchors[0] + period
            } else {
                anchors[j]
            };
            let source = anchors[i] + (end - anchors[i]) * t;
            anchors.insert(i + 1, source);
            i + 1
        }
        Operation::Delete => {
            if def.controls.len() <= if original.closed { 3 } else { 2 } {
                return Err("Keep at least two open or three closed nodes".into());
            }
            def.controls.remove(i);
            data.incoming.remove(i);
            data.outgoing.remove(i);
            data.smooth.remove(i);
            anchors.remove(i);
            i.min(def.controls.len() - 1)
        }
        Operation::Smooth => {
            let n = def.controls.len();
            let prev = if i > 0 {
                def.controls[i - 1]
            } else if original.closed {
                def.controls[n - 1]
            } else {
                def.controls[i]
            };
            let next = if i + 1 < n {
                def.controls[i + 1]
            } else if original.closed {
                def.controls[0]
            } else {
                def.controls[i]
            };
            let v = sub(next, prev);
            let len = length(v);
            let before = length(sub(def.controls[i], prev));
            let after = length(sub(next, def.controls[i]));
            let distance = if before == 0. {
                after / 3.
            } else if after == 0. {
                before / 3.
            } else {
                before.min(after) / 3.
            };
            let tangent = if len > 1e-12 {
                mul(v, distance / len)
            } else {
                [0.; 2]
            };
            data.incoming[i] = if i > 0 || original.closed {
                mul(tangent, -1.)
            } else {
                [0.; 2]
            };
            data.outgoing[i] = if i + 1 < n || original.closed {
                tangent
            } else {
                [0.; 2]
            };
            data.smooth[i] = true;
            i
        }
        Operation::Corner => {
            let n = def.controls.len();
            let p = def.controls[i];
            let direction = |neighbor: Point, old: Point| {
                let v = sub(neighbor, p);
                let len = length(v);
                if len > 1e-12 {
                    mul(v, length(old) / len)
                } else {
                    [0.; 2]
                }
            };
            data.incoming[i] = if i > 0 || original.closed {
                direction(def.controls[(i + n - 1) % n], data.incoming[i])
            } else {
                [0.; 2]
            };
            data.outgoing[i] = if i + 1 < n || original.closed {
                direction(def.controls[(i + 1) % n], data.outgoing[i])
            } else {
                [0.; 2]
            };
            data.smooth[i] = false;
            i
        }
    };
    let sampled = data.sample(&def.controls, original.closed)?;
    let pressure =
        original.fragment_pressure_at(&sources(&sampled, &anchors, original.closed, period))?;
    let points = sampled
        .points
        .into_iter()
        .map(|p| def.creation.inverse().map(p))
        .collect();
    def.nodes = Some(data);
    def.settings.constraint = crate::shapes::Constraint::Free;
    def.snapped_controls = true;
    let path = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .unwrap()
        .paths
        .iter_mut()
        .find(|p| p.id == hit.path)
        .unwrap();
    path.points = points;
    path.definition = Some(def);
    path.pressure = pressure;
    state.scene.lines.selection = State {
        layer: Some(layer),
        kind: Kind::Points,
        include_width: false,
        paths: vec![],
        points: vec![Control {
            path: hit.path,
            point: selected as u16,
        }],
    };
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Node edit geometry/JSON/Undo limit; nothing changed".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    Ok(state)
}
pub fn requires_v66(scene: &crate::scene::Scene) -> bool {
    scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .any(|p| p.definition.as_ref().is_some_and(|d| d.nodes.is_some()))
}
