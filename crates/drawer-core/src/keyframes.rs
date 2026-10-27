//! Non-destructive layer animation in parent coordinates; baked source data stays editable.
use crate::{
    document::Document,
    layer_properties::{self as props, Target},
    layer_search::Key,
    scene::{Affine, Point, Scene},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Owner {
    Layer(Key),
    Camera,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub offset: Point,
    pub scale: Point,
    pub degrees: f64,
    pub pivot: Point,
    pub opacity: f64,
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            offset: [0.; 2],
            scale: [1.; 2],
            degrees: 0.,
            pivot: [0.; 2],
            opacity: 1.,
        }
    }
}
impl Pose {
    pub fn valid(self) -> bool {
        self.offset
            .into_iter()
            .chain(self.pivot)
            .all(|v| v.is_finite() && v.abs() <= 1e9)
            && self
                .scale
                .into_iter()
                .all(|v| v.is_finite() && (0.01..=1000.).contains(&v))
            && self.degrees.is_finite()
            && self.degrees.abs() <= 36000.
            && self.opacity.is_finite()
            && (0.0..=1.).contains(&self.opacity)
    }
    pub fn matrix(self) -> Affine {
        let (s, c) = self.degrees.to_radians().sin_cos();
        let [x, y] = self.pivot;
        Affine([
            self.scale[0] * c,
            self.scale[0] * s,
            -self.scale[1] * s,
            self.scale[1] * c,
            self.offset[0] + x - x * self.scale[0] * c + y * self.scale[1] * s,
            self.offset[1] + y - x * self.scale[0] * s - y * self.scale[1] * c,
        ])
    }
    fn mix(self, b: Self, w: f64) -> Self {
        let lerp = |a: f64, b: f64| a + (b - a) * w;
        let pair = |a: Point, b: Point| [lerp(a[0], b[0]), lerp(a[1], b[1])];
        Self {
            offset: pair(self.offset, b.offset),
            scale: pair(self.scale, b.scale),
            degrees: lerp(self.degrees, b.degrees),
            pivot: pair(self.pivot, b.pivot),
            opacity: lerp(self.opacity, b.opacity),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Curve {
    Fixed,
    #[default]
    Linear,
    Bezier {
        first: Point,
        second: Point,
    },
}
impl Curve {
    pub fn valid(self) -> bool {
        match self {
            Self::Bezier {
                first: a,
                second: b,
            } => {
                a.into_iter()
                    .chain(b)
                    .all(|v| v.is_finite() && (0.0..=1.).contains(&v))
                    && a[0] <= b[0]
            }
            _ => true,
        }
    }
    pub fn weight(self, x: f64) -> f64 {
        match self {
            Self::Fixed => 0.,
            Self::Linear => x,
            Self::Bezier {
                first: a,
                second: b,
            } => {
                let eval = |t: f64, i: usize| {
                    3. * (1. - t).powi(2) * t * a[i] + 3. * (1. - t) * t * t * b[i] + t * t * t
                };
                let (mut lo, mut hi) = (0., 1.);
                for _ in 0..48 {
                    let t = (lo + hi) / 2.;
                    if eval(t, 0) < x { lo = t } else { hi = t }
                }
                eval((lo + hi) / 2., 1)
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    pub frame: u32,
    pub pose: Pose,
    pub curve: Curve,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub owner: Owner,
    pub enabled: bool,
    pub parent_camera: bool,
    pub basis: Affine,
    pub basis_opacity: u8,
    pub hold: Pose,
    pub keys: Vec<Keyframe>,
}
impl Track {
    pub fn at(&self, frame: u32) -> Pose {
        if !self.enabled || self.keys.is_empty() {
            return self.hold;
        }
        let i = self.keys.partition_point(|k| k.frame <= frame);
        if i == 0 {
            return self.keys[0].pose;
        }
        let a = self.keys[i - 1];
        if i == self.keys.len() || frame == a.frame {
            return a.pose;
        }
        let b = self.keys[i];
        a.pose.mix(
            b.pose,
            a.curve
                .weight((frame - a.frame) as f64 / (b.frame - a.frame) as f64),
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub camera_output: bool,
    pub tracks: Vec<Track>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        !self.camera_output && self.tracks.is_empty()
    }
    pub fn get(&self, owner: Owner) -> Option<&Track> {
        self.tracks.iter().find(|t| t.owner == owner)
    }
    pub fn bytes(&self) -> usize {
        self.tracks.len() * std::mem::size_of::<Track>()
            + self
                .tracks
                .iter()
                .map(|t| t.keys.len() * std::mem::size_of::<Keyframe>())
                .sum::<usize>()
    }
    pub fn valid(&self) -> bool {
        let mut owners = BTreeSet::new();
        (!self.camera_output || self.get(Owner::Camera).is_some())
            && self.tracks.len() <= 1024
            && self.tracks.iter().map(|t| t.keys.len()).sum::<usize>() <= 10000
            && self.tracks.iter().all(|t| {
                owners.insert(t.owner)
                    && match t.owner {
                        Owner::Camera => !t.parent_camera,
                        Owner::Layer(k) => crate::layer_groups::canonical(k),
                    }
                    && t.basis.valid()
                    && t.basis_opacity <= 100
                    && t.hold.valid()
                    && (!t.enabled || !t.keys.is_empty())
                    && t.keys.windows(2).all(|k| k[0].frame < k[1].frame)
                    && t.keys
                        .iter()
                        .all(|k| k.frame < 10000 && k.pose.valid() && k.curve.valid())
            })
    }
}
fn exists(d: &Document, key: Key) -> bool {
    crate::layer_groups::live(d).is_ok_and(|keys| keys.contains_key(&key))
}
pub fn writable(d: &Document, owner: Owner) -> Result<(), String> {
    if let Owner::Layer(key) = owner {
        if !exists(d, key) {
            return Err("Animation owner was removed".into());
        }
        let target = crate::layer_catalog::target(d, key).ok_or("Animation owner was removed")?;
        let f = crate::file_objects::stored_flags(d, target);
        if f.locked || f.position_locked || f.pixels_locked {
            return Err("Unlock the animation owner and its parents first".into());
        }
    }
    Ok(())
}
pub fn valid_document(d: &Document) -> bool {
    d.scene.keyframes.valid()
        && (d.scene.keyframes.is_default()
            || d.animation.as_ref().is_some_and(|a| {
                d.scene.keyframes.tracks.iter().all(|t| {
                    t.keys.iter().all(|k| k.frame < a.frames)
                        && match t.owner {
                            Owner::Camera => true,
                            Owner::Layer(k) => exists(d, k),
                        }
                })
            }))
}
pub fn content_view(d: &Document) -> bool {
    d.keyframe_content.is_some_and(|o| match o {
        Owner::Camera => false,
        Owner::Layer(k) => exists(d, k),
    })
}
pub fn edit_content(d: &mut Document, owner: Option<Owner>) -> Result<(), String> {
    if let Some(o) = owner {
        writable(d, o)?;
        if !matches!(o, Owner::Layer(_)) {
            return Err("Select a content layer, rather than the camera".into());
        }
    }
    d.keyframe_content = owner;
    Ok(())
}
fn inside(d: &Document, parent: Key, child: Key) -> bool {
    parent == child
        || match (parent, child) {
            (Key::Track(a), Key::Cell { track: b, .. }) => a == b,
            (Key::Layer(Target::Folder(_)), _) => {
                crate::layer_groups::content_members(d, &[parent]).is_ok_and(|v| v.contains(&child))
            }
            _ => false,
        }
}
pub fn guard(d: &Document, key: Key) -> Result<(), String> {
    if d.scene.keyframes.camera_output && !content_view(d) {
        return Err("Enter base content editing before changing sources in camera view".into());
    }
    let allowed = d
        .keyframe_content
        .is_some_and(|o| matches!(o,Owner::Layer(k)if inside(d,k,key)));
    if !allowed
        && d.scene.keyframes.tracks.iter().any(|t| {
            (!t.keys.is_empty() || t.hold != Pose::default() || t.parent_camera)
                && matches!(t.owner,Owner::Layer(k)if inside(d,k,key))
        })
    {
        return Err("Enter base content editing before changing an animated layer".into());
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub enum Edit {
    Enable(bool),
    ParentCamera(bool),
    Put { key: Keyframe, replace: bool },
    Move { from: u32, to: u32 },
    Delete(u32),
    Clear { confirmed: bool },
}
pub fn range(d: &Document, owner: Owner) -> String {
    d.scene.keyframes.get(owner).map_or_else(
        || "No keys".into(),
        |t| {
            if let (Some(a), Some(b)) = (t.keys.first(), t.keys.last()) {
                format!(
                    "{} keys; frames {}–{}",
                    t.keys.len(),
                    a.frame + 1,
                    b.frame + 1
                )
            } else {
                "No keys".into()
            }
        },
    )
}
pub fn prepare(d: &Document, owner: Owner, edit: Edit) -> Result<DocumentState, String> {
    writable(d, owner)?;
    let a = d.animation.as_ref().ok_or("Create a timeline first")?;
    if d.current_frame >= a.frames {
        return Err("Current frame outside timeline".into());
    }
    let mut s = d.snapshot();
    if s.scene.keyframes.get(owner).is_none() {
        let (basis, basis_opacity) = match owner {
            Owner::Layer(k) => (
                crate::transform_policy::frame(d, k),
                props::properties(d, crate::layer_catalog::target(d, k).unwrap()).opacity,
            ),
            Owner::Camera => (Affine::default(), 100),
        };
        s.scene.keyframes.tracks.push(Track {
            owner,
            enabled: false,
            parent_camera: false,
            basis,
            basis_opacity,
            hold: Pose::default(),
            keys: vec![],
        });
    }
    let t = s
        .scene
        .keyframes
        .tracks
        .iter_mut()
        .find(|t| t.owner == owner)
        .unwrap();
    match edit {
        Edit::Enable(on) => {
            if on && !t.enabled {
                if t.keys.is_empty() {
                    t.keys.push(Keyframe {
                        frame: d.current_frame,
                        pose: t.hold,
                        curve: Curve::Linear,
                    });
                }
                t.enabled = true;
            } else if !on && t.enabled {
                t.hold = t.at(d.current_frame);
                t.enabled = false;
            }
        }
        Edit::ParentCamera(on) => {
            if owner == Owner::Camera {
                return Err("Camera cannot parent itself".into());
            }
            t.parent_camera = on;
        }
        Edit::Put { key, replace } => {
            if !key.pose.valid() || !key.curve.valid() || key.frame >= a.frames {
                return Err("Invalid pose, curve handles or key time".into());
            }
            if !t.enabled {
                return Err("Enable keyframes before editing keys".into());
            }
            if let Some(old) = t.keys.iter_mut().find(|k| k.frame == key.frame) {
                if !replace {
                    return Err(
                        "A key already occupies that frame; explicitly choose Replace".into(),
                    );
                }
                *old = key;
            } else {
                t.keys.push(key);
                t.keys.sort_by_key(|k| k.frame);
            }
        }
        Edit::Move { from, to } => {
            if to >= a.frames || t.keys.iter().any(|k| k.frame == to && k.frame != from) {
                return Err("Key destination outside timeline or occupied".into());
            }
            let i = t
                .keys
                .iter()
                .position(|k| k.frame == from)
                .ok_or("Select an existing key")?;
            if i > 0 && to <= t.keys[i - 1].frame
                || i + 1 < t.keys.len() && to >= t.keys[i + 1].frame
            {
                return Err("Keys cannot cross their neighbours (reversed time)".into());
            }
            t.keys[i].frame = to;
        }
        Edit::Delete(frame) => {
            let hold = t.at(d.current_frame);
            if !t.keys.iter().any(|k| k.frame == frame) {
                return Err("Key missing".into());
            }
            t.keys.retain(|k| k.frame != frame);
            if t.keys.is_empty() {
                t.hold = hold;
                t.enabled = false;
            }
        }
        Edit::Clear { confirmed } => {
            if !confirmed {
                return Err(format!("Confirm deletion: {}", range(d, owner)));
            }
            t.hold = t.at(d.current_frame);
            t.keys.clear();
            t.enabled = false;
        }
    }
    if !s.scene.keyframes.valid() {
        return Err("Animation exceeds track/key limits or has invalid data".into());
    }
    Ok(s)
}
/// Local matrix includes one explicit camera. Folder/track parents are composed by the renderer.
pub fn local(d: &Document, key: Key, export: bool) -> (Affine, f64) {
    local_at(d, key, export, d.current_frame)
}
pub fn local_at(d: &Document, key: Key, export: bool, frame: u32) -> (Affine, f64) {
    if !export && content_view(d) {
        return (Affine::default(), 1.);
    }
    let t = d.scene.keyframes.get(Owner::Layer(key));
    let pose = t.map_or(Pose::default(), |t| t.at(frame));
    let mut matrix = pose.matrix();
    let mut opacity = pose.opacity;
    if t.is_some_and(|t| t.parent_camera)
        && !ancestors(d, key).into_iter().any(|k| {
            d.scene
                .keyframes
                .get(Owner::Layer(k))
                .is_some_and(|t| t.parent_camera)
        })
        && let Some(camera) = d.scene.keyframes.get(Owner::Camera)
    {
        let p = camera.at(frame);
        matrix = matrix.then(p.matrix());
        opacity *= p.opacity;
    }
    (matrix, opacity)
}
pub fn ancestors(d: &Document, key: Key) -> Vec<Key> {
    if let Key::Cell { track, .. } = key {
        return vec![Key::Track(track)];
    }
    let Key::Layer(target) = key else {
        return vec![];
    };
    let Some(mut node) = crate::clipping::node(d, target) else {
        return vec![];
    };
    let mut out = vec![];
    for _ in 0..16 {
        let Some((i, _)) = d
            .scene
            .paper
            .folders
            .iter()
            .enumerate()
            .find(|(_, f)| f.children.contains(&node))
        else {
            break;
        };
        out.push(Key::Layer(Target::Folder(i as u16)));
        node = crate::paper::Node::Folder(i);
    }
    out
}
pub fn combined(d: &Document, key: Key, export: bool) -> (Affine, f64) {
    combined_at(d, key, export, d.current_frame)
}
pub fn combined_at(d: &Document, key: Key, export: bool, frame: u32) -> (Affine, f64) {
    let (mut m, mut a) = local_at(d, key, export, frame);
    for parent in ancestors(d, key) {
        let (p, b) = local_at(d, parent, export, frame);
        m = m.then(p);
        a *= b;
    }
    (m, a)
}
pub(crate) fn prune(s: &mut DocumentState) -> Result<(), String> {
    if s.scene.keyframes.is_default() {
        return Ok(());
    }
    let d = crate::layer_merge::preview(s, 0)?;
    s.scene.keyframes.tracks.retain(|t| match t.owner {
        Owner::Camera => s.animation.is_some(),
        Owner::Layer(k) => exists(&d, k),
    });
    if s.scene.keyframes.get(Owner::Camera).is_none() {
        s.scene.keyframes.camera_output = false;
    }
    Ok(())
}
pub(crate) fn duplicate(s: &mut Scene, old: &Scene, map: &BTreeMap<Key, Key>) {
    for t in &old.keyframes.tracks {
        if let Owner::Layer(k) = t.owner
            && let Some(new) = map.get(&k)
        {
            let mut t = t.clone();
            t.owner = Owner::Layer(*new);
            s.keyframes.tracks.push(t);
        }
    }
}
/// Compare source content, ignoring animation metadata and display names/locks.
pub(crate) fn validate_edit(d: &Document, s: &DocumentState) -> Result<(), String> {
    crate::camera2d::validate_source_edit(d, s)?;
    for old in &d.scene.keyframes.tracks {
        if s.scene
            .keyframes
            .get(old.owner)
            .is_some_and(|next| next != old)
        {
            writable(d, old.owner)?;
        }
    }
    for next in &s.scene.keyframes.tracks {
        if d.scene.keyframes.get(next.owner).is_none()
            && match next.owner {
                Owner::Camera => true,
                Owner::Layer(k) => exists(d, k),
            }
        {
            writable(d, next.owner)?;
        }
    }
    if d.scene.keyframes.is_default() {
        return Ok(());
    }
    for key in crate::layer_groups::live(d)?.into_keys() {
        if guard(d, key).is_err() && s.scene.keyframes.get(Owner::Layer(key)).is_some() {
            let mask_changed = if let Key::Layer(target) = key {
                d.scene
                    .layer_effects
                    .get(target)
                    .and_then(|e| e.mask.as_ref())
                    != s.scene
                        .layer_effects
                        .get(target)
                        .and_then(|e| e.mask.as_ref())
            } else {
                false
            };
            if mask_changed || d.scene.transform != s.scene.transform {
                return guard(d, key);
            }
        }
    }
    for (n, old) in d.layers.iter().enumerate() {
        let key = Key::Layer(props::raster(d, n));
        if guard(d, key).is_err()
            && let Some(i) =
                (0..s.layers.len()).find(|i| s.scene.layer_ids.id(*i) == d.scene.layer_ids.id(n))
            && (!old.same_color(&s.layers[i]) || old.placement != s.layers[i].placement)
        {
            return guard(d, key);
        }
    }
    for old in &d.scene.lines.layers {
        let key = Key::Layer(Target::Vector(old.id));
        if guard(d, key).is_err()
            && let Some(new) = s.scene.lines.layers.iter().find(|l| l.id == old.id)
            && old.paths != new.paths
        {
            return guard(d, key);
        }
    }
    for t in &d.scene.keyframes.tracks {
        let Owner::Layer(k) = t.owner else { continue };
        if guard(d, k).is_ok() {
            continue;
        }
        let changed = match k {
            Key::Object(id) => d
                .scene
                .catalog
                .objects
                .index(id, d.scene.objects.len())
                .zip(s.scene.catalog.objects.index(id, s.scene.objects.len()))
                .is_some_and(|(i, j)| d.scene.objects[i] != s.scene.objects[j]),
            Key::Gradient(id) => d
                .scene
                .gradients
                .layers
                .iter()
                .find(|g| g.id == id)
                .zip(s.scene.gradients.layers.iter().find(|g| g.id == id))
                .is_some_and(|(a, b)| a != b),
            Key::World(id) => d
                .scene
                .catalog
                .worlds
                .index(id, d.scene.worlds.len())
                .zip(s.scene.catalog.worlds.index(id, s.scene.worlds.len()))
                .is_some_and(|(i, j)| d.scene.worlds[i] != s.scene.worlds[j]),
            Key::Track(_) | Key::Cell { .. } => d
                .scene
                .cels
                .links
                .iter()
                .filter(|l| match k {
                    Key::Track(track) => l.track == track,
                    Key::Cell { track, cell } => l.track == track && l.cell == cell,
                    _ => false,
                })
                .any(|l| {
                    d.scene
                        .cels
                        .get(l.source)
                        .zip(s.scene.cels.get(l.source))
                        .is_some_and(|(a, b)| a != b)
                }),
            _ => false,
        };
        if changed {
            return guard(d, k);
        }
    }
    Ok(())
}

pub(crate) fn selected(owner: Owner, scope: &crate::timeline_time_edit::Scope) -> bool {
    scope.all
        || scope.keyed.contains(&owner)
        || match owner {
            Owner::Camera => scope.globals,
            Owner::Layer(Key::Track(id)) | Owner::Layer(Key::Cell { track: id, .. }) => {
                scope.tracks.contains(&id)
            }
            _ => false,
        }
}
pub(crate) fn retime(
    data: &mut Data,
    map: &impl Fn(u32) -> Result<u32, String>,
    choices: &crate::timeline_time_edit::Choices,
    issues: &mut Vec<crate::timeline_time_edit::Issue>,
    rows: &mut Vec<String>,
    frames: &mut u32,
) -> Result<(), String> {
    for t in &mut data.tracks {
        let mut groups: BTreeMap<u32, Vec<Keyframe>> = BTreeMap::new();
        for k in &t.keys {
            groups.entry(map(k.frame)?).or_default().push(*k);
        }
        let mut keys = vec![];
        for (frame, group) in groups {
            let i = if group.len() > 1 {
                crate::timeline_time_edit::issue(
                    issues,
                    choices,
                    format!("layer-key-{:?}-{frame}", t.owner),
                    format!(
                        "{} keys of {:?} collide at frame{}",
                        group.len(),
                        t.owner,
                        frame + 1
                    ),
                    group
                        .iter()
                        .map(|k| format!("Keep original frame{}", k.frame + 1))
                        .collect(),
                )?
            } else {
                0
            };
            let mut key = group[i];
            rows.push(format!(
                "{:?} key {} → {}",
                t.owner,
                key.frame + 1,
                frame + 1
            ));
            key.frame = frame;
            keys.push(key);
            *frames = (*frames).max(frame + 1);
        }
        t.keys = keys;
    }
    Ok(())
}
pub(crate) fn splice(
    data: &mut Data,
    scope: &crate::timeline_time_edit::Scope,
    at: u32,
    count: u32,
    remove: bool,
    rows: &mut Vec<String>,
    frames: &mut u32,
) -> Result<(), String> {
    for t in &mut data.tracks {
        if !selected(t.owner, scope) {
            continue;
        }
        let hold = t.at(at);
        let mut keys = vec![];
        for mut k in t.keys.clone() {
            let old = k.frame;
            if remove && (at..at + count).contains(&k.frame) {
                rows.push(format!("Remove {:?} key{}", t.owner, k.frame + 1));
                continue;
            }
            if k.frame >= at {
                k.frame = if remove {
                    k.frame.saturating_sub(count).max(at)
                } else {
                    k.frame.checked_add(count).ok_or("Key frame overflow")?
                };
            }
            if k.frame >= 10000 {
                return Err("Layer key exceeds10000 frames".into());
            }
            *frames = (*frames).max(k.frame + 1);
            rows.push(format!("{:?} key {} → {}", t.owner, old + 1, k.frame + 1));
            keys.push(k);
        }
        t.keys = keys;
        if t.keys.is_empty() {
            t.enabled = false;
            t.hold = hold;
        }
    }
    Ok(())
}
