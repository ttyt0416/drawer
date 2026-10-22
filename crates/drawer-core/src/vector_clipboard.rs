//! Bounded local vector clipboard with explicit whole-stroke / adjacent-span policy.
use crate::{
    document::Document,
    line_tools::{Data, Layer, Path},
    scene::Affine,
    sizing::DocumentState,
    vector_scope::{self, Picked},
};
use serde::{Deserialize, Serialize};
const LIMIT: usize = 4 * 1024 * 1024;
const MAGIC: &[u8] = b"DRAWVEC1\n";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub transform: Affine,
    pub paths: Vec<Path>,
}
pub fn ranges(
    path: &Path,
    selection: &crate::vector_edit::Selection,
) -> Result<Vec<(f64, f64)>, String> {
    let period = if path.closed {
        path.points.len() as f64 / (path.points.len() - 1) as f64
    } else {
        1.
    };
    if selection.paths.contains(&path.id) {
        return Ok(vec![(0., period)]);
    }
    let params = path.control_parameters()?;
    let selected = selection
        .controls
        .iter()
        .filter(|c| c.path == path.id)
        .map(|c| c.point as usize)
        .collect::<std::collections::BTreeSet<_>>();
    let mut intervals: Vec<(f64, f64)> = vec![];
    let mut covered = std::collections::BTreeSet::new();
    for n in 0..params.len() {
        let next = (n + 1) % params.len();
        if n + 1 == params.len() && !path.closed {
            break;
        }
        if selected.contains(&n) && selected.contains(&next) {
            let a = params[n];
            let b = if next == 0 { period } else { params[next] };
            if let Some(last) = intervals.last_mut()
                && (last.1 - a).abs() < 1e-12
            {
                last.1 = b;
            } else {
                intervals.push((a, b));
            }
            covered.insert(n);
            covered.insert(next);
        }
    }
    if intervals.is_empty() || covered != selected {
        return Err("Span copy requires adjacent selected controls; isolated points copy whole strokes with the standard vector copy command".into());
    }
    Ok(intervals)
}
fn fragment(path: &Path, a: f64, b: f64) -> Result<Path, String> {
    if path.fill.is_some() {
        return Err(
            "Select the whole filled shape; span copy/cut operates on stroke-only paths".into(),
        );
    }
    let part = crate::vector_erase::part(path, a, b)?;
    let mut p = path.clone();
    p.points = part.points;
    p.pressure = path.fragment_pressure_at(&part.parameters)?;
    p.closed = false;
    p.definition = None;
    p.fill = None;
    Ok(p)
}
impl Packet {
    fn valid(&self) -> bool {
        if !self.transform.valid() || self.paths.is_empty() || self.paths.len() > 128 {
            return false;
        }
        let mut paths = self.paths.clone();
        for (i, p) in paths.iter_mut().enumerate() {
            p.id = i as u64 + 2;
        }
        Data {
            layers: vec![Layer {
                id: 1,
                name: "Clipboard".into(),
                hidden: false,
                locked: false,
                fill: Default::default(),
                paths,
            }],
            next_id: self.paths.len() as u64 + 2,
            ..Default::default()
        }
        .valid()
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid vector clipboard".into());
        }
        let mut out = MAGIC.to_vec();
        out.extend(serde_json::to_vec(self).map_err(|e| e.to_string())?);
        if out.len() > LIMIT {
            return Err("Vector clipboard limit4MiB".into());
        }
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > LIMIT || !bytes.starts_with(MAGIC) {
            return Err("Invalid vector clipboard header or size".into());
        }
        let p: Self = serde_json::from_slice(&bytes[MAGIC.len()..]).map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid vector clipboard geometry".into());
        }
        Ok(p)
    }
    /// Standard raster clipboard uses the actual rendered subset bounds, even
    /// when the page area selection is much smaller than a whole copied stroke.
    pub fn raster(&self) -> Result<crate::paste::ClipboardImage, String> {
        if !self.valid() {
            return Err("Invalid vector clipboard".into());
        }
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let scale = crate::line_tools::maximum_scale(self.transform);
        for p in &self.paths {
            let radius = if p.stroke {
                p.maximum_width() * scale / 2.
                    * if p.tip == crate::vector_tip::Preset::Square {
                        std::f64::consts::SQRT_2
                    } else {
                        1.
                    }
            } else {
                0.
            };
            for point in &p.points {
                let q = self.transform.map(*point);
                for k in 0..2 {
                    min[k] = min[k].min(q[k] - radius - 1.);
                    max[k] = max[k].max(q[k] + radius + 1.);
                }
            }
        }
        let origin = min.map(|v| v.floor() as i64);
        let size = std::array::from_fn::<_, 2, _>(|k| (max[k].ceil() - origin[k] as f64) as u32);
        let pixels = size[0] as usize * size[1] as usize;
        let complexity = self
            .paths
            .iter()
            .map(|p| {
                let pressure = if p.pressure.is_default() {
                    0
                } else {
                    64usize
                        .max(p.pressure.device.len())
                        .max(p.pressure.controls.len())
                };
                p.points
                    .len()
                    .saturating_add(pressure)
                    .saturating_add(p.svg.as_ref().map_or(0, |s| s.bytes() / 8))
            })
            .sum::<usize>();
        if size.contains(&0)
            || size.iter().any(|v| *v > 32768)
            || pixels > 4 * 1024 * 1024
            || pixels.saturating_mul(complexity) > 32 * 1024 * 1024
        {
            return Err("Vector raster clipboard rendering limit".into());
        }
        let inverse = self.transform.inverse();
        let mut data = Vec::with_capacity(pixels);
        let (mut left, mut top, mut right, mut bottom) = (size[0], size[1], 0, 0);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut sum = [0_f64; 4];
                for sy in 0..4 {
                    for sx in 0..4 {
                        let point = inverse.map([
                            origin[0] as f64 + x as f64 + (sx as f64 + 0.5) / 4.,
                            origin[1] as f64 + y as f64 + (sy as f64 + 0.5) / 4.,
                        ]);
                        let color = self
                            .paths
                            .iter()
                            .fold([0; 4], |out, p| crate::render::over(out, p.pixel(point)));
                        let alpha = color[3] as f64;
                        sum[3] += alpha;
                        for k in 0..3 {
                            sum[k] += color[k] as f64 * alpha;
                        }
                    }
                }
                let pixel = if sum[3] > 0. {
                    let mut p = [0; 4];
                    for k in 0..3 {
                        p[k] = (sum[k] / sum[3]).round() as u8;
                    }
                    p[3] = (sum[3] / 16.).round() as u8;
                    p
                } else {
                    [0; 4]
                };
                if pixel[3] > 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
                data.push(pixel);
            }
        }
        if right == 0 {
            return Ok(crate::paste::ClipboardImage {
                color: None,
                origin: Some(origin),
                image: crate::references::LocalImage {
                    width: 1,
                    height: 1,
                    pixels: vec![[0; 4]],
                },
            });
        }
        let width = right - left;
        let height = bottom - top;
        let mut trimmed = Vec::with_capacity(width as usize * height as usize);
        for y in top..bottom {
            let start = (y * size[0] + left) as usize;
            trimmed.extend_from_slice(&data[start..start + width as usize]);
        }
        Ok(crate::paste::ClipboardImage {
            color: None,
            origin: Some([origin[0] + left as i64, origin[1] + top as i64]),
            image: crate::references::LocalImage {
                width,
                height,
                pixels: trimmed,
            },
        })
    }
}
pub fn copy(d: &Document, scope: &[Picked], spans: bool) -> Result<Packet, String> {
    vector_scope::validate(d, scope)?;
    let mut paths = vec![];
    // Source layer/path order, independent of the order in the selection list.
    for entry in crate::render_order::normalized(&d.scene) {
        let crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(id)) =
            entry.key
        else {
            continue;
        };
        let layer = d.scene.lines.layers.iter().find(|l| l.id == id).unwrap();
        if let Some(s) = scope.iter().find(|s| s.layer == layer.id) {
            for p in crate::vector_edit::selected(d, layer.id, &s.selection)? {
                if spans && !s.selection.paths.contains(&p.id) {
                    for (a, b) in ranges(p, &s.selection)? {
                        paths.push(fragment(p, a, b)?);
                    }
                } else {
                    paths.push(p.clone());
                }
            }
        }
    }
    let out = Packet {
        transform: d.scene.transform,
        paths,
    };
    out.encode()?;
    Ok(out)
}
pub fn cut(d: &Document, scope: &[Picked], spans: bool) -> Result<DocumentState, String> {
    copy(d, scope, spans)?;
    let mut state = d.snapshot();
    for selected in scope {
        let index = state
            .scene
            .lines
            .layers
            .iter()
            .position(|l| l.id == selected.layer)
            .unwrap();
        let mut paths = vec![];
        for p in state.scene.lines.layers[index].paths.clone() {
            if !selected.selection.path_ids().contains(&p.id) {
                paths.push(p);
                continue;
            }
            if spans && !selected.selection.paths.contains(&p.id) {
                let cuts = ranges(&p, &selected.selection)?;
                let period = if p.closed {
                    p.points.len() as f64 / (p.points.len() - 1) as f64
                } else {
                    1.
                };
                let mut cursor = 0.;
                let mut keep = vec![];
                for (a, b) in cuts {
                    if a > cursor + 1e-12 {
                        keep.push((cursor, a));
                    }
                    cursor = b;
                }
                if cursor < period - 1e-12 {
                    keep.push((cursor, period));
                }
                for (n, (a, b)) in keep.into_iter().enumerate() {
                    let mut part = fragment(&p, a, b)?;
                    if n > 0 {
                        part.id = state.scene.lines.id()?;
                    }
                    paths.push(part);
                }
            }
        }
        state.scene.lines.layers[index].paths = paths;
    }
    let affected = scope
        .iter()
        .flat_map(|p| p.selection.path_ids())
        .collect::<std::collections::BTreeSet<_>>();
    state
        .scene
        .lines
        .selection
        .points
        .retain(|c| !affected.contains(&c.path));
    state
        .scene
        .lines
        .selection
        .paths
        .retain(|id| !affected.contains(id));
    state.scene.lines.prune_selection();
    vector_scope::checked(d, state)
}
pub fn paste(
    d: &Document,
    layer: u64,
    packet: &Packet,
    offset: [f64; 2],
) -> Result<(DocumentState, Picked), String> {
    crate::vector_edit::writable(d, layer)?;
    if !packet.valid() || offset.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return Err("Invalid vector paste".into());
    }
    // A path's tip and width share the page's linear frame. Translation can be
    // normalized exactly; a different linear frame needs an explicit raster/SVG conversion.
    if packet.transform.0[..4] != d.scene.transform.0[..4] {
        return Err("Editable paste requires matching page scale/rotation frames; use the raster fallback or SVG for a different frame".into());
    }
    let matrix = packet
        .transform
        .then(Affine([1., 0., 0., 1., offset[0], offset[1]]))
        .then(d.scene.transform.inverse());
    let mut state = d.snapshot();
    let mut ids = vec![];
    for p in &packet.paths {
        let mut p = p.clone();
        vector_scope::whole(&mut p, matrix);
        p.id = state.scene.lines.id()?;
        ids.push(p.id);
        state
            .scene
            .lines
            .layers
            .iter_mut()
            .find(|l| l.id == layer)
            .unwrap()
            .paths
            .push(p);
    }
    Ok((
        vector_scope::checked(d, state)?,
        Picked {
            layer,
            selection: crate::vector_edit::Selection {
                controls: vec![],
                paths: ids,
            },
        },
    ))
}
