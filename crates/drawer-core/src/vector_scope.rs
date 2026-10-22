//! Session selection, exact centerline/area tests and atomic pivot transforms.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::Path,
    scene::{Affine, Point, Rect},
    sizing::DocumentState,
    vector_edit::{self, Selection},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picked {
    pub layer: u64,
    pub selection: Selection,
}
pub fn validate(d: &Document, scope: &[Picked]) -> Result<(), String> {
    if scope.is_empty()
        || scope.len() > 32
        || scope.iter().map(|p| p.layer).collect::<BTreeSet<_>>().len() != scope.len()
    {
        return Err("Choose unique nonempty vector layers".into());
    }
    for p in scope {
        vector_edit::writable(d, p.layer)?;
        vector_edit::selected(d, p.layer, &p.selection)?;
    }
    Ok(())
}
pub(crate) fn checked(d: &Document, state: DocumentState) -> Result<DocumentState, String> {
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Vector operation exceeds geometry/Undo limits".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    Ok(state)
}
fn clip(a: Point, b: Point, r: Rect) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for k in 0..2 {
        let delta = b[k] - a[k];
        if delta.abs() < 1e-15 {
            if a[k] < r.min[k] - 1e-10 || a[k] > r.max[k] + 1e-10 {
                return None;
            }
        } else {
            let x = (r.min[k] - a[k]) / delta;
            let y = (r.max[k] - a[k]) / delta;
            lo = lo.max(x.min(y));
            hi = hi.min(x.max(y));
            if lo > hi + 1e-10 {
                return None;
            }
        }
    }
    Some((lo.clamp(0., 1.), hi.clamp(0., 1.)))
}
/// Inclusive boundaries. Merge the union of mask-run intervals, so a hole
/// between two inside endpoints still prevents full containment.
pub fn area(d: &Document, layers: &[u64], contained: bool) -> Result<Vec<Picked>, String> {
    if d.scene.selection.is_none() && d.scene.selection_mask.is_none() {
        return Err("Create an area selection first".into());
    }
    if layers.is_empty()
        || layers.len() > 32
        || layers.iter().collect::<BTreeSet<_>>().len() != layers.len()
    {
        return Err("Choose unique vector layers".into());
    }
    let rects: Vec<_> = if let Some(mask) = &d.scene.selection_mask {
        mask.runs
            .iter()
            .filter(|r| r.alpha > 0)
            .map(|r| Rect {
                min: [r.x as f64, r.y as f64],
                max: [r.x as f64 + r.length as f64, r.y as f64 + 1.],
            })
            .collect()
    } else {
        d.scene.selection.into_iter().collect()
    };
    let shift = d
        .scene
        .transform
        .inverse()
        .vector(d.scene.selection_translation);
    let mut work = 0usize;
    let mut result = vec![];
    for id in layers {
        let layer = d
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == *id)
            .ok_or("Missing vector selection layer")?;
        if vector_edit::writable(d, *id).is_err() {
            continue;
        }
        let mut selected = vec![];
        for path in &layer.paths {
            let mut any = false;
            let mut all = true;
            for (a, b) in path.segments() {
                let a = [a[0] - shift[0], a[1] - shift[1]];
                let b = [b[0] - shift[0], b[1] - shift[1]];
                let mut intervals = vec![];
                for r in &rects {
                    work += 1;
                    if work > 16 * 1024 * 1024 {
                        return Err("Centerline/selection work limit".into());
                    }
                    if let Some(range) = clip(a, b, *r) {
                        intervals.push(range);
                    }
                }
                intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
                any |= !intervals.is_empty();
                let mut end = 0.0_f64;
                let mut start = true;
                let mut covered = true;
                for (a, b) in intervals {
                    if start && a > 1e-10 || !start && a > end + 1e-10 {
                        covered = false;
                        break;
                    }
                    start = false;
                    end = end.max(b);
                }
                all &= covered && !start && end >= 1. - 1e-10;
                if contained && !all || !contained && any {
                    break;
                }
            }
            if if contained { all } else { any } {
                selected.push(path.id);
            }
        }
        if !selected.is_empty() {
            result.push(Picked {
                layer: *id,
                selection: Selection {
                    controls: vec![],
                    paths: selected,
                },
            });
        }
    }
    Ok(result)
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub pivot: Point,
    pub translation: Point,
    pub scale: Point,
    pub degrees: f64,
    pub duplicate: bool,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            pivot: [0.; 2],
            translation: [0.; 2],
            scale: [1.; 2],
            degrees: 0.,
            duplicate: false,
        }
    }
}
impl Transform {
    pub fn matrix(self) -> Result<Affine, String> {
        if self
            .pivot
            .iter()
            .chain(&self.translation)
            .any(|v| !v.is_finite() || v.abs() > 1e9)
            || self
                .scale
                .iter()
                .any(|v| !v.is_finite() || v.abs() < 1e-6 || v.abs() > 1e4)
            || !self.degrees.is_finite()
            || self.degrees.abs() > 36000.
        {
            return Err("Invalid pivot, translation, scale or rotation".into());
        }
        let (s, c) = self.degrees.to_radians().sin_cos();
        Ok(Affine([1., 0., 0., 1., -self.pivot[0], -self.pivot[1]])
            .then(Affine([
                c * self.scale[0],
                s * self.scale[0],
                -s * self.scale[1],
                c * self.scale[1],
                0.,
                0.,
            ]))
            .then(Affine([
                1.,
                0.,
                0.,
                1.,
                self.pivot[0] + self.translation[0],
                self.pivot[1] + self.translation[1],
            ])))
    }
}
pub(crate) fn whole(path: &mut Path, matrix: Affine) {
    for p in &mut path.points {
        *p = matrix.map(*p);
    }
    if let Some(def) = &mut path.definition {
        def.creation = matrix.inverse().then(def.creation);
    }
    if let Some(svg) = &mut path.svg {
        svg.transform(matrix);
    }
}
fn points(path: &mut Path, indices: &[usize], matrix: Affine) -> Result<(), String> {
    if path.definition.as_ref().is_some_and(|d| d.nodes.is_some()) {
        crate::vector_nodes::transform_controls(path, indices, matrix)?;
    } else if let Some(def) = &mut path.definition {
        let local = def.creation.inverse().then(matrix).then(def.creation);
        for i in indices {
            def.controls[*i] = local.map(def.controls[*i]);
        }
        def.snapped_controls = true;
        def.settings.constraint = crate::shapes::Constraint::Free;
        let g = def.geometry()?;
        path.points = g
            .points
            .into_iter()
            .map(|p| def.creation.inverse().map(p))
            .collect();
        path.closed = g.closed;
    } else {
        for i in indices {
            path.points[*i] = matrix.map(path.points[*i]);
        }
    }
    Ok(())
}
pub fn transform(
    d: &Document,
    scope: &[Picked],
    settings: Transform,
) -> Result<(DocumentState, Vec<Picked>), String> {
    validate(d, scope)?;
    let doc_matrix = settings.matrix()?;
    let matrix = d
        .scene
        .transform
        .then(doc_matrix)
        .then(d.scene.transform.inverse());
    let mut state = d.snapshot();
    let mut selected = vec![];
    for entry in scope {
        if crate::layer_properties::inherited(
            d,
            crate::layer_properties::Target::Vector(entry.layer),
        )
        .position_locked
        {
            return Err("Vector positions are locked".into());
        }
        let index = state
            .scene
            .lines
            .layers
            .iter()
            .position(|l| l.id == entry.layer)
            .unwrap();
        let mut selection = Selection::default();
        for original in vector_edit::selected(d, entry.layer, &entry.selection)? {
            let mut path = original.clone();
            if !matrix.is_identity() && entry.selection.paths.contains(&original.id) {
                whole(&mut path, matrix);
            } else if !matrix.is_identity() {
                let indices = entry
                    .selection
                    .controls
                    .iter()
                    .filter(|c| c.path == original.id)
                    .map(|c| c.point as usize)
                    .collect::<Vec<_>>();
                points(&mut path, &indices, matrix)?;
            }
            if settings.duplicate {
                path.id = state.scene.lines.id()?;
                state.scene.lines.layers[index].paths.push(path.clone());
            } else {
                *state.scene.lines.layers[index]
                    .paths
                    .iter_mut()
                    .find(|p| p.id == path.id)
                    .unwrap() = path.clone();
            }
            if entry.selection.paths.contains(&original.id) {
                selection.paths.push(path.id);
            } else {
                selection.controls.extend(
                    entry
                        .selection
                        .controls
                        .iter()
                        .filter(|c| c.path == original.id)
                        .map(|c| crate::vector_selection::Control {
                            path: path.id,
                            point: c.point,
                        }),
                );
            }
        }
        selected.push(Picked {
            layer: entry.layer,
            selection,
        });
    }
    Ok((checked(d, state)?, selected))
}
