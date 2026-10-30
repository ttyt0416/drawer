//! Final camera sampling and output-independent document path preferences.
use crate::{
    document::Document,
    keyframes::{self, Edit, Owner, Pose},
    layer_search::Key,
    scene::{Affine, Point},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Guides {
    pub enabled: bool,
    pub camera: bool,
    pub layer: Option<Key>,
    pub keys_only: bool,
    pub first: u32,
    pub last: u32,
    pub colors: [[u8; 3]; 2],
    pub corners: bool,
    pub centers: bool,
}
impl Default for Guides {
    fn default() -> Self {
        Self {
            enabled: false,
            camera: true,
            layer: None,
            keys_only: true,
            first: 0,
            last: 9999,
            colors: [[40, 180, 240], [240, 90, 50]],
            corners: false,
            centers: true,
        }
    }
}
impl Guides {
    pub fn valid(self) -> bool {
        self.first <= self.last
            && self.last < 10000
            && self.layer.is_none_or(crate::layer_groups::canonical)
    }
    pub fn color(self, frame: u32) -> [u8; 3] {
        let t = if self.last == self.first {
            0.
        } else {
            frame.saturating_sub(self.first) as f64 / (self.last - self.first) as f64
        }
        .clamp(0., 1.);
        [0, 1, 2].map(|i| {
            (self.colors[0][i] as f64 * (1. - t) + self.colors[1][i] as f64 * t).round() as u8
        })
    }
}
pub fn set_guides(d: &mut Document, g: Guides) -> Result<(), String> {
    if !g.valid() {
        return Err("Guide range must be ordered within frames1..10000".into());
    }
    d.camera_guides = g;
    Ok(())
}
pub fn enable(d: &Document, on: bool) -> Result<DocumentState, String> {
    let mut s = if on && d.scene.keyframes.get(Owner::Camera).is_none() {
        let mut s = keyframes::prepare(d, Owner::Camera, Edit::Enable(true))?;
        let t = s
            .scene
            .keyframes
            .tracks
            .iter_mut()
            .find(|t| t.owner == Owner::Camera)
            .unwrap();
        let pivot = [d.width as f64 / 2., d.height as f64 / 2.];
        t.hold.pivot = pivot;
        for k in &mut t.keys {
            k.pose.pivot = pivot;
        }
        s
    } else {
        d.animation.as_ref().ok_or("Create a timeline first")?;
        d.snapshot()
    };
    s.scene.keyframes.camera_output = on;
    Ok(s)
}
/// Camera pose maps the output frame into world coordinates. A larger scale shows more world.
pub fn to_world(d: &Document, export: bool) -> Affine {
    if !d.scene.keyframes.camera_output || !export && keyframes::content_view(d) {
        return Affine::default();
    }
    d.scene
        .keyframes
        .get(Owner::Camera)
        .map_or(Affine::default(), |t| t.at(d.current_frame).matrix())
}
pub fn display_matrix(d: &Document, key: Key, export: bool) -> (Affine, f64) {
    let (m, a) = keyframes::combined(d, key, export);
    (m.then(to_world(d, export).inverse()), a)
}
// Pointer drawing uses base coordinates. Explicit content mode prevents silent camera-space edits.
pub(crate) fn validate_source_edit(d: &Document, s: &DocumentState) -> Result<(), String> {
    if !d.scene.keyframes.camera_output || keyframes::content_view(d) {
        return Ok(());
    }
    let changed = d.width != s.width
        || d.height != s.height
        || d.layers.len() != s.layers.len()
        || d.layers
            .iter()
            .zip(&s.layers)
            .any(|(a, b)| !a.same_color(b) || a.placement != b.placement)
        || d.scene
            .lines
            .layers
            .iter()
            .map(|l| (&l.id, &l.paths))
            .collect::<Vec<_>>()
            != s.scene
                .lines
                .layers
                .iter()
                .map(|l| (&l.id, &l.paths))
                .collect::<Vec<_>>()
        || d.scene.objects != s.scene.objects
        || d.scene.gradients != s.scene.gradients
        || d.scene.worlds != s.scene.worlds
        || !d.scene.cels.same_sources(&s.scene.cels)
        || d.scene.layer_effects != s.scene.layer_effects
        || d.scene.transform != s.scene.transform
        || d.scene.layer_groups.transforms != s.scene.layer_groups.transforms
        || match (&d.animation, &s.animation) {
            (Some(a), Some(b)) => {
                a.tracks.len() != b.tracks.len()
                    || a.tracks.iter().zip(&b.tracks).any(|(a, b)| {
                        a.cells.len() != b.cells.len()
                            || a.cells
                                .iter()
                                .zip(&b.cells)
                                .any(|(a, b)| !a.same_content(b))
                    })
            }
            (None, None) => false,
            _ => true,
        };
    if changed {
        Err("Enter base content editing before changing sources in camera view".into())
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub frame: u32,
    pub camera: bool,
    pub corners: [Point; 4],
    pub center: Point,
    pub color: [u8; 3],
}
fn times(d: &Document, g: Guides, owner: Owner, last: u32) -> Vec<u32> {
    if !g.keys_only {
        return (g.first..=last).collect();
    }
    let mut owners = vec![owner];
    if let Owner::Layer(k) = owner {
        owners.extend(keyframes::ancestors(d, k).into_iter().map(Owner::Layer));
        if owners
            .iter()
            .any(|o| d.scene.keyframes.get(*o).is_some_and(|t| t.parent_camera))
        {
            owners.push(Owner::Camera);
        }
    }
    if let Owner::Layer(Key::Track(id)) = owner {
        owners.extend(
            d.scene
                .keyframes
                .tracks
                .iter()
                .filter_map(|t| match t.owner {
                    Owner::Layer(Key::Cell { track, .. }) if track == id => Some(t.owner),
                    _ => None,
                }),
        );
    }
    let keys = owners
        .iter()
        .filter_map(|o| d.scene.keyframes.get(*o))
        .flat_map(|t| t.keys.iter().map(|k| k.frame))
        .collect::<BTreeSet<_>>();
    if keys.is_empty() {
        vec![g.first]
    } else {
        keys.into_iter()
            .filter(|f| *f >= g.first && *f <= last)
            .collect()
    }
}
fn cell_at(d: &Document, key: Key, frame: u32) -> Option<Key> {
    let Key::Track(id) = key else {
        return Some(key);
    };
    let a = d.animation.as_ref()?;
    let n = d.scene.catalog.tracks.index(id, a.tracks.len())?;
    let (_, i, _) = a.indexed_visible_cells(frame).find(|(t, _, _)| *t == n)?;
    Some(Key::Cell {
        track: id,
        cell: d.scene.catalog.cell_id(id, i),
    })
}
/// At most20000 quads, with base bounds computed only once per stable source key.
pub fn paths(d: &Document) -> Result<Vec<Frame>, String> {
    let g = d.camera_guides;
    if !g.valid() {
        return Err("Invalid camera path preferences".into());
    }
    if !g.enabled {
        return Ok(vec![]);
    }
    let Some(a) = d.animation.as_ref() else {
        return Ok(vec![]);
    };
    let last = g.last.min(a.frames.saturating_sub(1));
    if g.first > last {
        return Ok(vec![]);
    }
    let g = Guides { last, ..g };
    let project = to_world(d, false).inverse();
    let mut out = vec![];
    let mut bounds = BTreeMap::new();
    if g.camera {
        for frame in times(d, g, Owner::Camera, last) {
            let pose = d.scene.keyframes.get(Owner::Camera).map_or(
                Pose {
                    pivot: [d.width as f64 / 2., d.height as f64 / 2.],
                    ..Default::default()
                },
                |t| t.at(frame),
            );
            let m = pose.matrix().then(project);
            out.push(Frame {
                frame,
                camera: true,
                corners: [
                    [0., 0.],
                    [d.width as f64, 0.],
                    [d.width as f64, d.height as f64],
                    [0., d.height as f64],
                ]
                .map(|p| m.map(p)),
                center: m.map(pose.pivot),
                color: g.color(frame),
            });
        }
    }
    if let Some(key) = g.layer {
        let mut frames = times(d, g, Owner::Layer(key), last);
        // Exposure changes are also distinct source frames for an editing track.
        if g.keys_only
            && let Key::Track(id) = key
            && let Some(n) = d.scene.catalog.tracks.index(id, a.tracks.len())
        {
            frames.extend(
                a.tracks[n]
                    .exposures
                    .iter()
                    .map(|e| e.frame)
                    .filter(|f| *f >= g.first && *f <= last),
            );
            frames.sort();
            frames.dedup();
        }
        for frame in frames {
            let Some(k) = cell_at(d, key, frame) else {
                continue;
            };
            let b = *bounds
                .entry(k)
                .or_insert_with(|| crate::layer_catalog::bounds_base(d, k));
            let Some(b) = b else { continue };
            let m = keyframes::combined_at(d, k, true, frame).0.then(project);
            let center = if let Some(t) = d.scene.keyframes.get(Owner::Layer(k)) {
                m.map(t.at(frame).pivot)
            } else if let Some(t) = d.scene.keyframes.get(Owner::Layer(key)) {
                keyframes::combined_at(d, key, true, frame)
                    .0
                    .then(project)
                    .map(t.at(frame).pivot)
            } else {
                m.map([(b.min[0] + b.max[0]) / 2., (b.min[1] + b.max[1]) / 2.])
            };
            out.push(Frame {
                frame,
                camera: false,
                corners: [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
                    .map(|p| m.map(p)),
                center,
                color: g.color(frame),
            });
        }
    }
    if out
        .iter()
        .flat_map(|f| f.corners.iter().chain([&f.center]))
        .flatten()
        .any(|v| !v.is_finite())
    {
        return Err("Nonfinite camera guide geometry".into());
    }
    Ok(out)
}
