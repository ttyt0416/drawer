//! Explicit local pose assets and timeline bindings over unchanged base rigs.
use crate::{
    document::Document,
    human_rig::{Kind, Rig},
    keyframes::Curve,
    sizing::DocumentState,
    world3d::{self, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub axes: Matrix,
    pub min: V,
    pub max: V,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skeleton {
    pub version: u32,
    pub kind: Kind,
    pub bones: Vec<Bone>,
}
impl Skeleton {
    pub fn of(r: &Rig) -> Self {
        let mut skeleton_rig = r.clone();
        skeleton_rig.mirrored = false;
        Self {
            version: r.version,
            kind: r.kind,
            bones: skeleton_rig
                .joints()
                .into_iter()
                .map(|j| Bone {
                    name: j.name,
                    parent: j.parent,
                    axes: j.bind.basis(),
                    min: j.min,
                    max: j.max,
                })
                .collect(),
        }
    }
    fn valid(&self) -> bool {
        *self == Self::of(&Rig::new(self.kind))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub angles: Vec<V>,
    pub mirrored: bool,
}
impl Pose {
    pub fn of(r: &Rig) -> Self {
        Self {
            angles: r.angles.clone(),
            mirrored: r.mirrored,
        }
    }
    fn rig(&self, base: &Rig) -> Rig {
        let mut r = base.clone();
        r.angles = self.angles.clone();
        r.mirrored = self.mirrored;
        r
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub version: u32,
    pub name: String,
    pub skeleton: Skeleton,
    pub poses: Vec<Pose>,
    pub step: u32,
}
fn name(n: &str) -> bool {
    !n.trim().is_empty() && n.len() <= 256 && !n.chars().any(char::is_control)
}
impl Sequence {
    pub fn capture(r: &Rig, name: String, step: u32) -> Result<Self, String> {
        let s = Self {
            version: 1,
            name,
            skeleton: Skeleton::of(r),
            poses: vec![Pose::of(r)],
            step,
        };
        if !s.valid() {
            return Err("Invalid pose sequence name/rig/timing".into());
        }
        Ok(s)
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && name(&self.name)
            && self.skeleton.valid()
            && self.poses.len() <= 256
            && (1..=10000).contains(&self.step)
            && self
                .poses
                .iter()
                .all(|p| p.rig(&Rig::new(self.skeleton.kind)).valid())
    }
    pub fn bytes(&self) -> usize {
        self.name.len()
            + self
                .skeleton
                .bones
                .iter()
                .map(|b| b.name.len() + 192)
                .sum::<usize>()
            + self
                .poses
                .iter()
                .map(|p| p.angles.len() * 24 + 32)
                .sum::<usize>()
            + 128
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = crate::model_import::local_path(path)?;
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("drawerposes"))
        {
            return Err("Choose an original .drawerposes sequence".into());
        }
        let s: Self = serde_json::from_slice(&crate::book_io::read_local(&path, 2 * 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err(
                "Sequence version, bone ancestry/axes/limits, pose or timing is incompatible"
                    .into(),
            );
        }
        Ok(s)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub clip: Sequence,
    pub source: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub rig: u64,
    pub asset: u64,
    pub timeline: u64,
    pub start: u32,
    pub step: u32,
    pub repeats: u32,
    pub curve: Curve,
}
impl Binding {
    pub fn end(&self, s: &Sequence) -> Option<u32> {
        self.step
            .checked_mul(s.poses.len() as u32)?
            .checked_mul(self.repeats)?
            .checked_add(self.start)
    }
    fn valid(&self, s: &Sequence) -> bool {
        self.rig > 0
            && self.asset > 0
            && self.timeline > 0
            && (1..=10000).contains(&self.step)
            && (1..=1000).contains(&self.repeats)
            && self.curve.valid()
            && !s.poses.is_empty()
            && self.end(s).is_some_and(|n| n <= 10000)
            && (self.curve == Curve::Fixed
                || s.poses.windows(2).all(|p| p[0].mirrored == p[1].mirrored))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub assets: BTreeMap<u64, Asset>,
    pub bindings: Vec<Binding>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            assets: BTreeMap::new(),
            bindings: vec![],
        }
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn bytes(&self) -> usize {
        self.assets
            .values()
            .map(|a| a.clip.bytes() + a.source.as_ref().map_or(0, String::len) + 64)
            .sum::<usize>()
            + self.bindings.len() * 96
    }
    pub fn valid(&self, scene: &crate::scene::Scene) -> bool {
        self.next > 0
            && self.assets.len() <= 32
            && self.bindings.len() <= 1024
            && self.assets.iter().all(|(id, a)| {
                *id > 0
                    && *id < self.next
                    && a.clip.valid()
                    && a.source.as_ref().is_none_or(|p| {
                        p.len() <= 2048
                            && !p.chars().any(char::is_control)
                            && crate::font_library::local(Path::new(p))
                    })
            })
            && self
                .bindings
                .iter()
                .map(|b| (b.rig, b.timeline))
                .collect::<BTreeSet<_>>()
                .len()
                == self.bindings.len()
            && self.bindings.iter().all(|b| {
                self.assets.get(&b.asset).is_some_and(|a| {
                    b.valid(&a.clip)
                        && scene
                            .world3d
                            .rigs
                            .get(&b.rig)
                            .is_some_and(|r| Skeleton::of(r) == a.clip.skeleton)
                }) && scene.timelines.entries.iter().any(|e| e.id == b.timeline)
            })
    }
    fn insert(&mut self, a: Asset) -> Result<u64, String> {
        if self.assets.len() >= 32 {
            return Err("Pose sequence asset limit32".into());
        }
        let id = self.next;
        self.next = id.checked_add(1).ok_or("Pose sequence ID exhausted")?;
        self.assets.insert(id, a);
        Ok(id)
    }
}
pub fn active(scene: &crate::scene::Scene, id: u64) -> Option<&Binding> {
    scene
        .world3d
        .sequences
        .bindings
        .iter()
        .find(|b| b.rig == id && b.timeline == scene.timelines.active)
}
pub(crate) fn prune(scene: &mut crate::scene::Scene) {
    let rigs = scene.world3d.rigs.keys().copied().collect::<BTreeSet<_>>();
    let timelines = scene
        .timelines
        .entries
        .iter()
        .map(|t| t.id)
        .collect::<BTreeSet<_>>();
    let data = &mut scene.world3d.sequences;
    data.bindings.retain(|b| {
        rigs.contains(&b.rig)
            && timelines.contains(&b.timeline)
            && data.assets.contains_key(&b.asset)
    });
}
pub(crate) fn duplicate(
    scene: &mut crate::scene::Scene,
    original: &crate::scene::Scene,
    map: &BTreeMap<crate::layer_search::Key, crate::layer_search::Key>,
) -> Result<(), String> {
    let mut assets = BTreeMap::new();
    for (old, new) in map {
        if let (crate::layer_search::Key::World(old), crate::layer_search::Key::World(new)) =
            (old, new)
        {
            for b in original
                .world3d
                .sequences
                .bindings
                .iter()
                .filter(|b| b.rig == *old)
            {
                let asset = if let Some(id) = assets.get(&b.asset) {
                    *id
                } else {
                    let a = original.world3d.sequences.assets[&b.asset].clone();
                    let id = scene.world3d.sequences.insert(a)?;
                    assets.insert(b.asset, id);
                    id
                };
                let mut b = b.clone();
                b.rig = *new;
                b.asset = asset;
                scene.world3d.sequences.bindings.push(b);
            }
        }
    }
    Ok(())
}
pub fn at(s: &Sequence, frame: u32, step: u32, curve: Curve) -> Result<Pose, String> {
    if !s.valid() || s.poses.is_empty() || step == 0 || !curve.valid() {
        return Err("Choose a nonempty compatible sequence and positive timing".into());
    }
    let i = (frame / step) as usize;
    if i >= s.poses.len() - 1 {
        return Ok(s.poses.last().unwrap().clone());
    }
    let a = &s.poses[i];
    let b = &s.poses[i + 1];
    if curve != Curve::Fixed && a.mirrored != b.mirrored {
        return Err("Opposite mirrored poses require fixed interpolation".into());
    }
    let w = curve.weight((frame % step) as f64 / step as f64);
    Ok(Pose {
        angles: a
            .angles
            .iter()
            .zip(&b.angles)
            .map(|(a, b)| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * w))
            .collect(),
        mirrored: a.mirrored,
    })
}
pub fn sampled_rig(d: &Document, id: u64) -> Result<Rig, String> {
    let base = d
        .scene
        .world3d
        .rigs
        .get(&id)
        .ok_or("Choose an original rigged mannequin")?;
    let Some(b) = active(&d.scene, id).filter(|_| d.animation.is_some()) else {
        return Ok(base.clone());
    };
    let s = &d
        .scene
        .world3d
        .sequences
        .assets
        .get(&b.asset)
        .ok_or("Missing pose asset")?
        .clip;
    if !b.valid(s) || Skeleton::of(base) != s.skeleton {
        return Err("Pose skeleton/binding mismatch; no automatic name mapping".into());
    }
    let end = b.end(s).unwrap();
    let frame = if d.current_frame < b.start {
        0
    } else if d.current_frame >= end {
        b.step * s.poses.len() as u32 - 1
    } else {
        (d.current_frame - b.start) % (b.step * s.poses.len() as u32)
    };
    let r = at(s, frame, b.step, b.curve)?.rig(base);
    if !r.valid() {
        return Err("Sampled pose exceeds rig constraints".into());
    }
    Ok(r)
}
pub(crate) fn meshes(d: &Document) -> Result<BTreeMap<u64, crate::scene::WorldMesh>, String> {
    let mut out = BTreeMap::new();
    if d.animation.is_none() {
        return Ok(out);
    }
    for b in d
        .scene
        .world3d
        .sequences
        .bindings
        .iter()
        .filter(|b| b.timeline == d.scene.timelines.active)
    {
        let r = sampled_rig(d, b.rig)?;
        let (mut mesh, _) = crate::human_rig::generate(&r, &AtomicBool::new(false))?;
        let i = d
            .scene
            .catalog
            .worlds
            .index(b.rig, d.scene.worlds.len())
            .ok_or("Pose rig removed")?;
        let frame = world3d::node(&d.scene, b.rig)
            .ok_or("Pose frame missing")?
            .frame;
        for v in &mut mesh.vertices {
            *v = frame.map(*v);
        }
        mesh.color = d.scene.worlds[i].color;
        out.insert(b.rig, mesh);
    }
    Ok(out)
}
fn apply_pose(
    scene: &mut crate::scene::Scene,
    id: u64,
    pose: &Pose,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let base = scene.world3d.rigs.get(&id).ok_or("Rig removed")?;
    let flip = base.mirrored != pose.mirrored;
    let rig = pose.rig(base);
    if !rig.valid() {
        return Err("Pose exceeds target joint constraints".into());
    }
    let (mut mesh, _) = crate::human_rig::generate(&rig, cancel)?;
    let i = scene
        .catalog
        .worlds
        .index(id, scene.worlds.len())
        .ok_or("Rig model removed")?;
    let frame = world3d::node(scene, id).ok_or("Rig frame missing")?.frame;
    for v in &mut mesh.vertices {
        *v = frame.map(*v);
    }
    mesh.color = scene.worlds[i].color;
    if flip && let Some(surface) = scene.world3d.surfaces.get_mut(&id) {
        for face in &mut surface.triangles {
            face.v.swap(1, 2);
            if let Some(uv) = &mut face.t {
                uv.swap(1, 2);
            }
        }
    }
    scene.worlds[i] = mesh;
    scene.world3d.rigs.insert(id, rig);
    Ok(())
}
pub enum Edit {
    Capture {
        rig: u64,
        name: String,
        step: u32,
    },
    Load(PathBuf),
    Reload {
        asset: u64,
        path: PathBuf,
    },
    Append {
        asset: u64,
        rig: u64,
        sampled: bool,
    },
    Pose {
        asset: u64,
        index: usize,
        pose: Pose,
    },
    Order {
        asset: u64,
        from: usize,
        to: usize,
    },
    RemovePose {
        asset: u64,
        index: usize,
    },
    Timing {
        asset: u64,
        step: u32,
    },
    Rename {
        asset: u64,
        name: String,
    },
    Delete(u64),
    Bind {
        rig: u64,
        asset: u64,
        start: u32,
        step: u32,
        repeats: u32,
        curve: Curve,
    },
    Unbind(u64),
    BasePose {
        rig: u64,
        pose: Pose,
    },
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    frame: u32,
    pub summary: String,
    pub asset: Option<u64>,
    pub pose: Option<usize>,
    pub warnings: Vec<String>,
}
impl Prepared {
    pub fn matches(&self, d: &Document) -> bool {
        self.id == d.id && self.revision == d.revision && self.frame == d.current_frame
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("Pose sequence preview became stale".into());
        }
        d.commit_state_named(self.state, &self.summary)
    }
    pub fn thumbnail(
        &self,
        d: &Document,
        rig: Option<u64>,
        pose: usize,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        preview_budget(d, budget)?;
        let mut state = self.state.clone();
        if let Some(rig) = rig {
            state
                .scene
                .world3d
                .sequences
                .bindings
                .retain(|b| b.rig != rig);
        }
        if let (Some(rig), Some(asset)) = (rig, self.asset)
            && let Some(a) = state.scene.world3d.sequences.assets.get(&asset)
            && state
                .scene
                .world3d
                .rigs
                .get(&rig)
                .is_some_and(|r| Skeleton::of(r) == a.clip.skeleton)
            && let Some(p) = a
                .clip
                .poses
                .get(
                    self.pose
                        .unwrap_or(pose)
                        .min(a.clip.poses.len().saturating_sub(1)),
                )
                .cloned()
        {
            apply_pose(&mut state.scene, rig, &p, cancel)?;
        }
        let mut copy = crate::layer_merge::preview(&state, d.current_frame)?;
        copy.keyframe_content = d.keyframe_content;
        let renderer = crate::render::Renderer::new(&copy);
        if let Some(e) = renderer.error() {
            return Err(e.into());
        }
        let mut bytes = vec![];
        for y in 0..128 {
            crate::book_io::check_cancel(cancel)?;
            for x in 0..128 {
                bytes.extend(
                    renderer
                        .display_sample([
                            (x as f64 + 0.5) * d.width as f64 / 128.,
                            (y as f64 + 0.5) * d.height as f64 / 128.,
                        ])?
                        .rgba8(),
                );
            }
        }
        Ok(bytes)
    }
}
pub fn preview_pose(
    d: &Document,
    rig: u64,
    pose: &Pose,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    preview_budget(d, budget)?;
    let mut state = d.snapshot();
    state
        .scene
        .world3d
        .sequences
        .bindings
        .retain(|b| b.rig != rig);
    apply_pose(&mut state.scene, rig, pose, cancel)?;
    let mut copy = crate::layer_merge::preview(&state, d.current_frame)?;
    copy.keyframe_content = d.keyframe_content;
    let renderer = crate::render::Renderer::new(&copy);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let mut image = vec![];
    for y in 0..128 {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..128 {
            image.extend(
                renderer
                    .display_sample([
                        (x as f64 + 0.5) * d.width as f64 / 128.,
                        (y as f64 + 0.5) * d.height as f64 / 128.,
                    ])?
                    .rgba8(),
            );
        }
    }
    Ok(image)
}
fn preview_budget(d: &Document, budget: usize) -> Result<(), String> {
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(triangles.saturating_mul(512))
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Pose preview acceleration exceeds memory budget".into());
    }
    Ok(())
}
fn editable(d: &Document, ids: impl IntoIterator<Item = u64>) -> Result<(), String> {
    crate::model_library::editable(d, &ids.into_iter().collect())
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    preview_budget(d, budget)?;
    let mut state = d.snapshot();
    let mut asset = None;
    let mut pose = None;
    let mut warnings = vec![];
    let changed_asset = match &edit {
        Edit::Reload { asset, .. }
        | Edit::Append { asset, .. }
        | Edit::Pose { asset, .. }
        | Edit::Order { asset, .. }
        | Edit::RemovePose { asset, .. }
        | Edit::Timing { asset, .. }
        | Edit::Delete(asset) => Some(*asset),
        _ => None,
    };
    if let Some(id) = changed_asset {
        editable(
            d,
            d.scene
                .world3d
                .sequences
                .bindings
                .iter()
                .filter(|b| b.asset == id)
                .map(|b| b.rig),
        )?;
    }
    let summary = match edit {
        Edit::Capture { rig, name, step } => {
            let r = d
                .scene
                .world3d
                .rigs
                .get(&rig)
                .ok_or("Choose an original rig")?;
            let clip = Sequence::capture(r, name, step)?;
            asset = Some(
                state
                    .scene
                    .world3d
                    .sequences
                    .insert(Asset { clip, source: None })?,
            );
            pose = Some(0);
            "Register base-scene pose snapshot; no timeline link"
        }
        Edit::Load(path) => {
            let path = crate::model_import::local_path(&path)?;
            let clip = Sequence::load(&path)?;
            asset = Some(state.scene.world3d.sequences.insert(Asset {
                clip,
                source: Some(path.to_str().ok_or("Use UTF-8 pose path")?.into()),
            })?);
            pose = Some(0);
            "Register frozen local pose sequence; no timeline link"
        }
        Edit::Reload { asset: id, path } => {
            let path = crate::model_import::local_path(&path)?;
            let clip = Sequence::load(&path)?;
            let old = state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?;
            *old = Asset {
                clip,
                source: Some(path.to_str().ok_or("Use UTF-8 pose path")?.into()),
            };
            asset = Some(id);
            pose = Some(0);
            "Reload pose sequence explicitly from local file"
        }
        Edit::Append {
            asset: id,
            rig,
            sampled,
        } => {
            let r = if sampled {
                sampled_rig(d, rig)?
            } else {
                d.scene.world3d.rigs.get(&rig).ok_or("Rig removed")?.clone()
            };
            let a = state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?;
            if a.clip.skeleton != Skeleton::of(&r) {
                return Err(
                    "Skeleton mismatch; equal names do not imply compatible ancestry/axes".into(),
                );
            }
            a.clip.poses.push(Pose::of(&r));
            asset = Some(id);
            pose = Some(a.clip.poses.len() - 1);
            "Append an independent pose snapshot to sequence"
        }
        Edit::Pose {
            asset: id,
            index,
            pose: p,
        } => {
            let a = state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?;
            *a.clip.poses.get_mut(index).ok_or("Pose removed")? = p;
            asset = Some(id);
            pose = Some(index);
            "Edit selected sequence pose only; base scene unchanged"
        }
        Edit::Order {
            asset: id,
            from,
            to,
        } => {
            let a = state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?;
            if from >= a.clip.poses.len() || to >= a.clip.poses.len() {
                return Err("Select existing pose positions".into());
            }
            let p = a.clip.poses.remove(from);
            a.clip.poses.insert(to, p);
            asset = Some(id);
            pose = Some(to);
            "Reorder sequence pose snapshots"
        }
        Edit::RemovePose { asset: id, index } => {
            let a = state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?;
            if index >= a.clip.poses.len() {
                return Err("Pose removed".into());
            }
            a.clip.poses.remove(index);
            if a.clip.poses.is_empty() {
                state
                    .scene
                    .world3d
                    .sequences
                    .bindings
                    .retain(|b| b.asset != id);
            }
            asset = Some(id);
            pose = Some(index.saturating_sub(1));
            "Remove sequence pose; empty sequences detach placements"
        }
        Edit::Timing { asset: id, step } => {
            state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?
                .clip
                .step = step;
            asset = Some(id);
            "Edit local sequence preview timing; placed timing remains explicit"
        }
        Edit::Rename { asset: id, name } => {
            state
                .scene
                .world3d
                .sequences
                .assets
                .get_mut(&id)
                .ok_or("Sequence removed")?
                .clip
                .name = name;
            asset = Some(id);
            "Rename local pose sequence"
        }
        Edit::Delete(id) => {
            if state.scene.world3d.sequences.assets.remove(&id).is_none() {
                return Err("Sequence removed".into());
            }
            state
                .scene
                .world3d
                .sequences
                .bindings
                .retain(|b| b.asset != id);
            "Delete sequence snapshot and its explicit placements"
        }
        Edit::Bind {
            rig,
            asset: id,
            start,
            step,
            repeats,
            curve,
        } => {
            editable(d, [rig])?;
            crate::timelines::initialize(&mut state)?;
            let a = state
                .scene
                .world3d
                .sequences
                .assets
                .get(&id)
                .ok_or("Sequence removed")?;
            let b = Binding {
                rig,
                asset: id,
                timeline: state.scene.timelines.active,
                start,
                step,
                repeats,
                curve,
            };
            if !b.valid(&a.clip)
                || b.end(&a.clip).unwrap() > state.animation.as_ref().unwrap().frames
            {
                return Err("Placement needs a nonempty compatible sequence, positive timing/repeats, fixed mirror changes and a timeline containing its full range".into());
            }
            state
                .scene
                .world3d
                .sequences
                .bindings
                .retain(|p| !(p.rig == rig && p.timeline == b.timeline));
            state.scene.world3d.sequences.bindings.push(b);
            asset = Some(id);
            "Place sequence on active timeline explicitly (pose channels only)"
        }
        Edit::Unbind(rig) => {
            editable(d, [rig])?;
            let timeline = state.scene.timelines.active;
            state
                .scene
                .world3d
                .sequences
                .bindings
                .retain(|b| !(b.rig == rig && b.timeline == timeline));
            "Detach active pose placement; base scene retained"
        }
        Edit::BasePose { rig, pose: p } => {
            editable(d, [rig])?;
            apply_pose(&mut state.scene, rig, &p, cancel)?;
            "Edit base scene pose only; sequence snapshots unchanged"
        }
    };
    crate::book_io::check_cancel(cancel)?;
    if !state.scene.world3d.sequences.valid(&state.scene) {
        return Err("Sequence/bone mapping/timing mismatch or pose asset limit exceeded".into());
    }
    if state
        .scene
        .world3d
        .sequences
        .bindings
        .iter()
        .filter(|b| b.timeline == state.scene.timelines.active)
        .any(|b| {
            b.end(&state.scene.world3d.sequences.assets[&b.asset].clip)
                .is_some_and(|e| state.animation.as_ref().is_some_and(|a| e > a.frames))
        })
    {
        warnings.push("Stored placement extends beyond the current timeline; its timing is preserved when the timeline is shortened or the sequence grows".into());
    }
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Pose assets/Undo exceed memory budget".into());
    }
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::layer_groups::finish(d, &state)?;
    let frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&frozen)
        .map_err(|e| format!("Pose assets exceed native capacity: {e}"))?;
    Ok(Prepared {
        state,
        id: d.id,
        revision: d.revision,
        frame: d.current_frame,
        summary: summary.into(),
        asset,
        pose,
        warnings,
    })
}
pub fn export(d: &Document, id: u64, path: &Path) -> Result<(), String> {
    if !crate::font_library::local(path) {
        return Err("Choose a local pose output".into());
    }
    let a = d
        .scene
        .world3d
        .sequences
        .assets
        .get(&id)
        .ok_or("Choose a sequence")?;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("drawerposes"))
    {
        return Err("Use a .drawerposes output".into());
    }
    if let Some(source) = &a.source {
        let destination = if path.exists() {
            path.canonicalize().map_err(|e| e.to_string())?
        } else {
            path.parent()
                .ok_or("Output folder missing")?
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(path.file_name().ok_or("Output name missing")?)
        };
        if destination.to_string_lossy().eq_ignore_ascii_case(source) {
            return Err("Choose a new output; imported source is read-only".into());
        }
    }
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(&a.clip).map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Pose sequence export exceeds2MiB".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose pose output folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = parent.join(path.file_name().ok_or("Choose output filename")?);
    let before = crate::animation_export::existing(&path)?;
    let mut stage = crate::animation_export::Stage::new(&parent)?;
    let mut file = stage.create("poses.drawerposes")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    if crate::animation_export::existing(&path)? != before {
        return Err("Pose output changed while saving".into());
    }
    std::fs::rename(stage.directory.join("poses.drawerposes"), path).map_err(|e| e.to_string())
}
