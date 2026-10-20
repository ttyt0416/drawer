//! Atomic vector control/style edits; hardware pressure is never overwritten by editing.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    line_tools::Path,
    scene::Point,
    sizing::DocumentState,
    vector_selection::{Control, Kind, State},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Point,
    Pressure,
    Color,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier {
    #[default]
    Control,
    Alt,
    Shift,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub tool: Tool,
    pub sensitivity: f64,
    pub radius: f64,
    pub temporary: Modifier,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            tool: Tool::Point,
            sensitivity: 0.01,
            radius: 10.,
            temporary: Modifier::Control,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.sensitivity.is_finite()
            && (0.001..=1.).contains(&self.sensitivity)
            && self.radius.is_finite()
            && (2. ..=32.).contains(&self.radius)
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub controls: Vec<Control>,
    pub paths: Vec<u64>,
}
impl Selection {
    pub fn current(d: &Document, layer: u64) -> Self {
        let s = &d.scene.lines.selection;
        if s.layer != Some(layer) {
            return Self::default();
        }
        Self {
            controls: s.points.clone(),
            paths: s.paths.clone(),
        }
    }
    pub fn path_ids(&self) -> BTreeSet<u64> {
        self.paths
            .iter()
            .copied()
            .chain(self.controls.iter().map(|c| c.path))
            .collect()
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Patch {
    pub movement: Option<Point>,
    pub pressure: Option<f64>,
    pub pressure_delta: Option<f64>,
    pub width: Option<f64>,
    pub color: Option<[u8; 4]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub path: u64,
    pub control: Option<u16>,
    pub point: Point,
    pub distance: f64,
}
fn layer(d: &Document, id: u64) -> Result<&crate::line_tools::Layer, String> {
    d.scene
        .lines
        .layers
        .iter()
        .find(|l| l.id == id)
        .ok_or_else(|| "Choose a live editable vector layer".into())
}
fn writable(d: &Document, id: u64) -> Result<(), String> {
    layer(d, id)?;
    let t = Target::Vector(id);
    let f = p::inherited(d, t);
    if f.hidden
        || f.locked
        || f.pixels_locked
        || d.scene
            .file_objects
            .get(crate::layer_search::Key::Layer(t))
            .is_some()
    {
        return Err("Vector layer is hidden, locked or a read-only linked source".into());
    }
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Vector edit snapshot limit".into());
    }
    Ok(())
}
pub fn hits(d: &Document, id: u64, point: Point, radius: f64, tool: Tool) -> Vec<Hit> {
    if writable(d, id).is_err()
        || !radius.is_finite()
        || radius <= 0.
        || point.iter().any(|v| !v.is_finite())
    {
        return vec![];
    }
    let Ok(layer) = layer(d, id) else {
        return vec![];
    };
    let mut hits = vec![];
    for path in layer.paths.iter().rev() {
        let mut found = false;
        for n in 0..path.control_count() {
            if let Some(p) = path.control_point(n) {
                let p = d.scene.transform.map(p);
                let distance = (p[0] - point[0]).hypot(p[1] - point[1]);
                if distance <= radius {
                    hits.push(Hit {
                        path: path.id,
                        control: Some(n as u16),
                        point: p,
                        distance,
                    });
                    found = true;
                }
            }
        }
        if tool == Tool::Color && !found {
            let mut best = (f64::INFINITY, point);
            for (a, b) in path.segments() {
                let a = d.scene.transform.map(a);
                let b = d.scene.transform.map(b);
                let (distance, near) = crate::line_tools::distance(point, a, b);
                if distance < best.0 {
                    best = (distance, near);
                }
            }
            let scale = crate::line_tools::maximum_scale(d.scene.transform);
            if best.0 <= radius
                || path.pressure_segments().any(|(a, b, wa, wb)| {
                    crate::vector_pressure::taper_hit(
                        point,
                        d.scene.transform.map(a),
                        d.scene.transform.map(b),
                        wa * scale + 2. * radius,
                        wb * scale + 2. * radius,
                    )
                })
            {
                hits.push(Hit {
                    path: path.id,
                    control: None,
                    point: best.1,
                    distance: best.0,
                });
            }
        }
    }
    // Preserve visual/topmost order at crossings; proximity orders candidates
    // only within a practically equal visual location.
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    if tool == Tool::Color {
        let mut seen = BTreeSet::new();
        hits.retain(|h| seen.insert(h.path));
    }
    hits
}
fn selected<'a>(d: &'a Document, id: u64, s: &Selection) -> Result<Vec<&'a Path>, String> {
    let l = layer(d, id)?;
    if s.controls.len() > 8192
        || s.paths.len() > 128
        || s.controls.iter().collect::<BTreeSet<_>>().len() != s.controls.len()
        || s.paths.iter().collect::<BTreeSet<_>>().len() != s.paths.len()
    {
        return Err("Choose unique vector controls and paths".into());
    }
    let ids = s.path_ids();
    if ids.is_empty() {
        return Err("Select a point or path first".into());
    }
    let paths = l
        .paths
        .iter()
        .filter(|p| ids.contains(&p.id))
        .collect::<Vec<_>>();
    if paths.len() != ids.len()
        || s.controls.iter().any(|c| {
            paths
                .iter()
                .find(|p| p.id == c.path)
                .is_none_or(|p| c.point as usize >= p.control_count())
        })
    {
        return Err("Vector selection changed; select the current controls again".into());
    }
    Ok(paths)
}
pub struct Values {
    pub width: Option<f64>,
    pub color: Option<[u8; 4]>,
    pub pressure: Option<f64>,
    pub paths: usize,
    pub controls: usize,
}
pub fn values(d: &Document, id: u64, s: &Selection) -> Result<Values, String> {
    let paths = selected(d, id, s)?;
    let first = paths[0];
    let pressures = s
        .controls
        .iter()
        .map(|c| {
            let p = paths.iter().find(|p| p.id == c.path).unwrap();
            p.pressure
                .control_at(c.point as f64 / p.control_count().saturating_sub(1).max(1) as f64)
        })
        .collect::<Vec<_>>();
    Ok(Values {
        width: paths
            .iter()
            .all(|p| p.width == first.width)
            .then_some(first.width),
        color: paths
            .iter()
            .all(|p| p.color == first.color)
            .then_some(first.color),
        pressure: pressures
            .first()
            .copied()
            .filter(|v| pressures.iter().all(|p| (p - v).abs() < 1e-9)),
        paths: paths.len(),
        controls: s.controls.len(),
    })
}
pub fn prepare(
    d: &Document,
    id: u64,
    s: &Selection,
    patch: Patch,
) -> Result<DocumentState, String> {
    writable(d, id)?;
    let paths = selected(d, id, s)?;
    if patch.pressure.is_some() && patch.pressure_delta.is_some() {
        return Err("Choose absolute pressure or relative pressure, not both".into());
    }
    if patch
        .pressure
        .is_some_and(|v| !v.is_finite() || !(0. ..=2.).contains(&v))
        || patch.pressure_delta.is_some_and(|v| !v.is_finite())
        || patch
            .width
            .is_some_and(|v| !v.is_finite() || !(0. ..=512.).contains(&v))
        || patch
            .movement
            .is_some_and(|v| v.iter().any(|v| !v.is_finite() || v.abs() > 1e9))
    {
        return Err("Vector edit value is outside its range".into());
    }
    if (patch.movement.is_some() || patch.pressure.is_some() || patch.pressure_delta.is_some())
        && s.controls.is_empty()
    {
        return Err("Select control points for position or pressure edits".into());
    }
    if patch.movement.is_some() && p::inherited(d, Target::Vector(id)).position_locked {
        return Err("Vector positions are locked".into());
    }
    let ids = paths.iter().map(|p| p.id).collect::<BTreeSet<_>>();
    let mut state = d.snapshot();
    let l = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == id)
        .unwrap();
    let movement = patch
        .movement
        .map(|delta| d.scene.transform.inverse().vector(delta));
    for path in l.paths.iter_mut().filter(|p| ids.contains(&p.id)) {
        let controls = s
            .controls
            .iter()
            .filter(|c| c.path == path.id)
            .copied()
            .collect::<Vec<_>>();
        if let Some(delta) = movement {
            if let Some(def) = &mut path.definition {
                let shift = def.creation.vector(delta);
                for c in &controls {
                    let p = &mut def.controls[c.point as usize];
                    p[0] += shift[0];
                    p[1] += shift[1];
                }
                def.snapped_controls = true;
                def.settings.constraint = crate::shapes::Constraint::Free;
                let geometry = def.geometry()?;
                path.points = geometry
                    .points
                    .into_iter()
                    .map(|p| def.creation.inverse().map(p))
                    .collect();
                path.closed = geometry.closed;
            } else {
                for c in &controls {
                    let p = &mut path.points[c.point as usize];
                    p[0] += delta[0];
                    p[1] += delta[1];
                }
            }
        }
        if patch.pressure.is_some() || patch.pressure_delta.is_some() {
            let count = path.control_count();
            let original = path.pressure.clone();
            for c in &controls {
                let old =
                    original.control_at(c.point as f64 / count.saturating_sub(1).max(1) as f64);
                let value = patch.pressure.unwrap_or_else(|| {
                    (old + patch.pressure_delta.unwrap_or_default()).clamp(0., 2.)
                });
                path.pressure.set_control(c.point as usize, count, value)?;
            }
        }
        if let Some(width) = patch.width {
            path.width = width;
        }
        if let Some(color) = patch.color {
            path.color = color;
        }
    }
    let point_edit = patch.movement.is_some()
        || patch.pressure.is_some()
        || patch.pressure_delta.is_some()
        || (!s.controls.is_empty()
            && s.paths.is_empty()
            && patch.color.is_none()
            && patch.width.is_none());
    state.scene.lines.selection = State {
        layer: Some(id),
        kind: if point_edit {
            Kind::Points
        } else {
            Kind::Strokes
        },
        include_width: false,
        points: if point_edit {
            s.controls.clone()
        } else {
            vec![]
        },
        paths: if point_edit {
            vec![]
        } else {
            ids.into_iter().collect()
        },
    };
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Vector edit exceeds geometry/Undo limits".into());
    }
    p::validate_edit(d, &state)?;
    Ok(state)
}
