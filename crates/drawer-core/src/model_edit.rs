//! Original shape keys, constrained own rigs and world/bone parent relations.
use crate::{
    document::Document,
    human_rig::{self, Rig},
    layer_search::Key,
    pose_sequence::Skeleton,
    scene::{Scene, WorldMesh},
    world_math::{add, sub},
    world3d::{self, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub min: V,
    pub max: V,
    pub locked: [bool; 3],
    pub hold: V,
}
pub fn controls(r: &Rig) -> Vec<Control> {
    if r.controls.is_empty() {
        r.joints()
            .iter()
            .zip(&r.angles)
            .map(|(j, a)| Control {
                min: j.min,
                max: j.max,
                locked: [false; 3],
                hold: *a,
            })
            .collect()
    } else {
        r.controls.clone()
    }
}
pub fn valid_controls(r: &Rig) -> bool {
    r.controls.is_empty()
        || r.controls.len() == r.angles.len()
            && r.joints()
                .iter()
                .zip(&r.controls)
                .zip(&r.angles)
                .all(|((j, c), a)| {
                    (0..3).all(|i| {
                        c.min[i].is_finite()
                            && c.max[i].is_finite()
                            && c.hold[i].is_finite()
                            && c.min[i] >= j.min[i]
                            && c.max[i] <= j.max[i]
                            && c.min[i] <= c.max[i]
                            && a[i] >= c.min[i]
                            && a[i] <= c.max[i]
                            && (!c.locked[i] || (a[i] - c.hold[i]).abs() < 1e-10)
                    })
                })
}
pub fn pairs(s: &Skeleton, map: &[(usize, usize)]) -> Result<(), String> {
    let expected = Skeleton::of(&Rig::new(s.kind));
    if *s != expected {
        return Err("Bone names alone are insufficient: ancestry, axes, limits and kind must match the original skeleton".into());
    }
    let mut wanted = vec![];
    for (i, b) in s.bones.iter().enumerate() {
        if let Some(stem) = b.name.strip_prefix("L/") {
            let right = s
                .bones
                .iter()
                .position(|b| b.name == format!("R/{stem}"))
                .ok_or("Missing right-side bone")?;
            wanted.push((i, right));
        }
    }
    if wanted.is_empty()
        || wanted.iter().copied().collect::<BTreeSet<_>>() != map.iter().copied().collect()
        || map.len() != wanted.len()
    {
        return Err("A complete verified left/right bone mapping is required".into());
    }
    Ok(())
}
fn own_pairs(r: &Rig) -> Result<Vec<(usize, usize)>, String> {
    let s = Skeleton::of(r);
    let map = s
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            b.name.strip_prefix("L/").and_then(|stem| {
                s.bones
                    .iter()
                    .position(|b| b.name == format!("R/{stem}"))
                    .map(|right| (i, right))
            })
        })
        .collect::<Vec<_>>();
    pairs(&s, &map)?;
    Ok(map)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    LeftToRight,
    RightToLeft,
    Swap,
}
pub fn copy_side(r: &Rig, side: Side) -> Result<Rig, String> {
    let map = own_pairs(r)?;
    let mut next = r.clone();
    for (l, rr) in map {
        if side != Side::RightToLeft {
            let a = r.angles[l];
            next.angles[rr] = [a[0], -a[1], -a[2]];
        }
        if side != Side::LeftToRight {
            let a = r.angles[rr];
            next.angles[l] = [a[0], -a[1], -a[2]];
        }
    }
    if !next.valid() {
        return Err(
            "Copied pose violates destination locks / angle limits; no partial copy applied".into(),
        );
    }
    Ok(next)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeKey {
    pub name: String,
    pub delta: Vec<V>,
    pub weight: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Morph {
    pub base: Vec<V>,
    pub keys: Vec<ShapeKey>,
    pub normals: Vec<Option<[u32; 3]>>,
}
fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}
fn points(p: &[V]) -> bool {
    !p.is_empty() && p.len() <= 65536 && p.iter().flatten().all(|v| v.is_finite() && v.abs() <= 1e9)
}
impl Morph {
    fn vertices(&self) -> Vec<V> {
        self.base
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.keys
                    .iter()
                    .fold(*p, |p, k| add(p, k.delta[i].map(|v| v * k.weight)))
            })
            .collect()
    }
    fn valid(&self) -> bool {
        self.normals.len() <= 131072
            && points(&self.base)
            && !self.keys.is_empty()
            && self.keys.len() <= 32
            && self
                .keys
                .iter()
                .map(|k| k.name.to_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                == self.keys.len()
            && self.keys.iter().all(|k| {
                label(&k.name)
                    && k.delta.len() == self.base.len()
                    && points(&k.delta)
                    && k.weight.is_finite()
                    && (0. ..=1.).contains(&k.weight)
            })
    }
    fn bytes(&self) -> usize {
        self.base.len() * 24
            + self.normals.len() * 16
            + self
                .keys
                .iter()
                .map(|k| k.delta.len() * 24 + k.name.len() + 64)
                .sum::<usize>()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub parent: u64,
    pub joint: usize,
    pub skeleton: Skeleton,
    pub local: Matrix,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub morphs: BTreeMap<u64, Morph>,
    pub attachments: BTreeMap<u64, Attachment>,
}
fn index(s: &Scene, id: u64) -> Result<usize, String> {
    s.catalog
        .worlds
        .index(id, s.worlds.len())
        .ok_or("Model removed".into())
}
fn close(a: Matrix, b: Matrix) -> bool {
    a.0.iter()
        .flatten()
        .zip(b.0.iter().flatten())
        .all(|(a, b)| (a - b).abs() <= 1e-7 + 1e-12 * b.abs())
}
fn bone(s: &Scene, a: &Attachment) -> Result<Matrix, String> {
    let r = s
        .world3d
        .rigs
        .get(&a.parent)
        .ok_or("Bone parent has no original rig")?;
    if Skeleton::of(r) != a.skeleton {
        return Err("Bone attachment skeleton mismatch".into());
    }
    let joint = r.frames().get(a.joint).copied().ok_or("Bone removed")?;
    Ok(joint.then(
        world3d::node(s, a.parent)
            .ok_or("Bone parent removed")?
            .frame,
    ))
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.morphs.is_empty() && self.attachments.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.morphs.values().map(Morph::bytes).sum::<usize>()
            + self
                .attachments
                .values()
                .map(|a| {
                    a.skeleton
                        .bones
                        .iter()
                        .map(|b| b.name.len() + 192)
                        .sum::<usize>()
                        + 160
                })
                .sum::<usize>()
    }
    pub fn valid(&self, s: &Scene) -> bool {
        self.morphs.len() <= 256
            && self.attachments.len() <= 256
            && self.morphs.iter().all(|(id, m)| {
                m.valid()
                    && s.world3d
                        .surfaces
                        .get(id)
                        .map_or(m.normals.is_empty(), |surface| {
                            m.normals.len() == surface.triangles.len()
                                && m.normals
                                    .iter()
                                    .flatten()
                                    .flatten()
                                    .all(|n| (*n as usize) < surface.normals.len())
                        })
                    && !s.world3d.rigs.contains_key(id)
                    && index(s, *id).is_ok_and(|i| {
                        let f = world3d::node(s, *id).unwrap().frame;
                        let vertices = m.vertices();
                        vertices.len() == s.worlds[i].vertices.len()
                            && vertices.iter().zip(&s.worlds[i].vertices).all(|(p, q)| {
                                f.map(*p)
                                    .iter()
                                    .zip(q)
                                    .all(|(a, b)| (a - b).abs() <= 1e-6 + 1e-12 * b.abs())
                            })
                    })
            })
            && self.attachments.iter().all(|(id, a)| {
                *id != a.parent
                    && a.local.valid()
                    && index(s, *id).is_ok()
                    && world3d::node(s, *id).is_some_and(|n| {
                        n.parent == Some(a.parent)
                            && bone(s, a).is_ok_and(|p| close(n.frame, a.local.then(p)))
                    })
            })
    }
}
pub(crate) fn prune(s: &mut Scene) {
    let live = (0..s.worlds.len())
        .map(|i| s.catalog.worlds.id(i))
        .collect::<BTreeSet<_>>();
    s.world3d.editing.morphs.retain(|id, _| live.contains(id));
    let dead = s
        .world3d
        .editing
        .attachments
        .iter()
        .filter(|(id, a)| {
            !live.contains(id)
                || !live.contains(&a.parent)
                || !s.world3d.rigs.contains_key(&a.parent)
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in dead {
        s.world3d.editing.attachments.remove(&id);
        if let Some(n) = s.world3d.nodes.get_mut(&id) {
            n.parent = None;
        }
    }
}
pub(crate) fn duplicate(s: &mut Scene, old: &Scene, map: &BTreeMap<Key, Key>) {
    for (a, b) in map {
        if let (Key::World(a), Key::World(b)) = (a, b) {
            if let Some(m) = old.world3d.editing.morphs.get(a) {
                s.world3d.editing.morphs.insert(*b, m.clone());
            }
            if let Some(a) = old.world3d.editing.attachments.get(a) {
                let mut a = a.clone();
                if let Some(Key::World(parent)) = map.get(&Key::World(a.parent)) {
                    a.parent = *parent;
                }
                s.world3d.editing.attachments.insert(*b, a);
            }
        }
    }
}
fn frames(
    s: &Scene,
    joints: &BTreeMap<u64, Vec<Matrix>>,
    before: Option<&Scene>,
) -> Result<BTreeMap<u64, Matrix>, String> {
    fn visit(
        s: &Scene,
        id: u64,
        joints: &BTreeMap<u64, Vec<Matrix>>,
        out: &mut BTreeMap<u64, Matrix>,
        seen: &mut BTreeSet<u64>,
        depth: usize,
        before: Option<&Scene>,
    ) -> Result<Matrix, String> {
        if let Some(f) = out.get(&id) {
            return Ok(*f);
        }
        if depth > 16 || !seen.insert(id) {
            return Err("Parent relation cycles / exceeds16 levels".into());
        }
        index(s, id)?;
        let n = world3d::node(s, id).ok_or("Missing model axes")?;
        let f = if let Some(a) = s.world3d.editing.attachments.get(&id) {
            if n.parent != Some(a.parent) {
                return Err("Attachment parent differs from node parent".into());
            }
            let p = visit(s, a.parent, joints, out, seen, depth + 1, before)?;
            let j = joints
                .get(&a.parent)
                .and_then(|j| j.get(a.joint))
                .ok_or("Missing attachment bone")?;
            a.local.then(j.then(p))
        } else if let Some(parent) = n.parent {
            let p = visit(s, parent, joints, out, seen, depth + 1, before)?;
            let base = world3d::node(s, parent).ok_or("Parent removed")?.frame;
            let base = before
                .and_then(|old| {
                    let child = world3d::node(old, id)?;
                    let parent_before = world3d::node(old, parent)?;
                    (child.parent == Some(parent)
                        && close(child.frame, n.frame)
                        && !close(parent_before.frame, base))
                    .then_some(parent_before.frame)
                })
                .unwrap_or(base);
            n.frame.then(base.inverse()).then(p)
        } else {
            n.frame
        };
        if !f.valid() {
            return Err("Invalid attached model transform".into());
        }
        seen.remove(&id);
        out.insert(id, f);
        Ok(f)
    }
    let mut out = BTreeMap::new();
    for i in 0..s.worlds.len() {
        visit(
            s,
            s.catalog.worlds.id(i),
            joints,
            &mut out,
            &mut BTreeSet::new(),
            0,
            before,
        )?;
    }
    Ok(out)
}
pub(crate) fn sampled_frames(d: &Document) -> Result<BTreeMap<u64, Matrix>, String> {
    let mut joints = BTreeMap::new();
    for id in d.scene.world3d.rigs.keys() {
        joints.insert(*id, crate::pose_sequence::sampled_rig(d, *id)?.frames());
    }
    frames(&d.scene, &joints, None)
}
pub(crate) fn meshes(d: &Document) -> Result<BTreeMap<u64, WorldMesh>, String> {
    if d.scene.world3d.editing.attachments.is_empty()
        && d.scene
            .world3d
            .sequences
            .bindings
            .iter()
            .all(|b| b.timeline != d.scene.timelines.active || d.animation.is_none())
    {
        return Ok(BTreeMap::new());
    }
    let frames = sampled_frames(d)?;
    let mut out = BTreeMap::new();
    for i in 0..d.scene.worlds.len() {
        let id = d.scene.catalog.worlds.id(i);
        let f = frames[&id];
        let base = world3d::node(&d.scene, id).unwrap().frame;
        if d.animation.is_some() && crate::pose_sequence::active(&d.scene, id).is_some() {
            let r = crate::pose_sequence::sampled_rig(d, id)?;
            let (mut mesh, _) = human_rig::generate(&r, &AtomicBool::new(false))?;
            for p in &mut mesh.vertices {
                *p = f.map(*p);
            }
            mesh.color = d.scene.worlds[i].color;
            out.insert(id, mesh);
        } else if !close(f, base) {
            let mut mesh = d.scene.worlds[i].clone();
            let delta = base.inverse().then(f);
            for p in &mut mesh.vertices {
                *p = delta.map(*p);
            }
            out.insert(id, mesh);
        }
    }
    Ok(out)
}
pub(crate) fn sync(d: &Document, s: &mut Scene) -> Result<(), String> {
    sync_scene(&d.scene, s)
}
pub(crate) fn sync_scene(before: &Scene, s: &mut Scene) -> Result<(), String> {
    prune(s);
    if s.world3d.editing.attachments.is_empty()
        && s.world3d.nodes.values().all(|n| n.parent.is_none())
    {
        return Ok(());
    }
    let ids = s
        .world3d
        .editing
        .attachments
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        let a = s.world3d.editing.attachments[&id].clone();
        let now = world3d::node(s, id).ok_or("Attachment removed")?.frame;
        let old = before.world3d.editing.attachments.get(&id);
        let changed = if old.is_some_and(|old| *old == a) {
            let old_child = world3d::node(before, id)
                .ok_or("Old attachment missing")?
                .frame;
            let old_parent = world3d::node(before, a.parent)
                .ok_or("Old parent missing")?
                .frame;
            let parent = world3d::node(s, a.parent).ok_or("Parent missing")?.frame;
            !close(now, old_child) && !close(now, old_child.then(old_parent.inverse()).then(parent))
        } else {
            !close(now, a.local.then(bone(s, &a)?))
        };
        if changed {
            s.world3d.editing.attachments.get_mut(&id).unwrap().local =
                now.then(bone(s, &a)?.inverse());
        }
    }
    let joints = s
        .world3d
        .rigs
        .iter()
        .map(|(id, r)| (*id, r.frames()))
        .collect();
    let result = frames(s, &joints, Some(before))?;
    for (id, f) in result {
        let old = world3d::node(s, id).unwrap().frame;
        if !close(old, f) {
            let i = index(s, id)?;
            let delta = old.inverse().then(f);
            for p in &mut s.worlds[i].vertices {
                *p = delta.map(*p);
            }
            s.world3d.nodes.insert(
                id,
                world3d::Node {
                    frame: f,
                    parent: world3d::node(s, id).and_then(|n| n.parent),
                },
            );
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    World,
    Local,
}
pub enum Edit {
    Fit {
        rig: u64,
        change: crate::mannequin_fit::Change,
        base: bool,
    },
    Expressions {
        rig: u64,
        weights: [f64; 3],
        base: bool,
    },
    Joint {
        rig: u64,
        joint: usize,
        angles: V,
        base: bool,
    },
    Control {
        rig: u64,
        joint: usize,
        value: Control,
        base: bool,
    },
    Copy {
        rig: u64,
        side: Side,
        base: bool,
    },
    AddKey {
        model: u64,
        name: String,
        scale: V,
    },
    CaptureKey {
        model: u64,
        source: u64,
        name: String,
    },
    Weight {
        model: u64,
        key: usize,
        value: f64,
    },
    DeleteKey {
        model: u64,
        key: usize,
    },
    Parent {
        child: u64,
        parent: Option<u64>,
        joint: Option<usize>,
        keep: Keep,
    },
}
pub struct Prepared {
    inner: world3d::Prepared,
    frame: u32,
    pub summary: String,
}
impl Prepared {
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if self.frame != d.current_frame {
            return Err("Model edit preview frame changed".into());
        }
        self.inner.commit(d)
    }
    pub fn thumbnail(
        &self,
        d: &Document,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        let mut state = d.snapshot();
        state.scene = self.inner.scene().clone();
        let mut frozen = crate::layer_merge::preview(&state, d.current_frame)?;
        frozen.keyframe_content = d.keyframe_content;
        let triangles = frozen
            .scene
            .world3d
            .surfaces
            .values()
            .map(|s| s.triangles.len())
            .sum::<usize>();
        if frozen
            .memory_bytes()
            .saturating_mul(3)
            .saturating_add(triangles.saturating_mul(512))
            .saturating_add(4 * 1024 * 1024)
            > budget
        {
            return Err("Model preview exceeds memory budget".into());
        }
        let renderer = crate::render::Renderer::new(&frozen);
        if let Some(e) = renderer.error() {
            return Err(e.into());
        }
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for y in 0..128 {
            crate::book_io::check_cancel(cancel)?;
            for x in 0..128 {
                pixels.extend(
                    renderer
                        .display_sample([
                            (x as f64 + 0.5) * d.width as f64 / 128.,
                            (y as f64 + 0.5) * d.height as f64 / 128.,
                        ])?
                        .rgba8(),
                );
            }
        }
        Ok(pixels)
    }
}
fn fresh_morph(s: &Scene, id: u64) -> Result<Morph, String> {
    if s.world3d.rigs.contains_key(&id) {
        return Err("Original rigs use expression morphs; detach rig before making generic vertex shape keys".into());
    }
    let i = index(s, id)?;
    let f = world3d::node(s, id).unwrap().frame.inverse();
    Ok(s.world3d
        .editing
        .morphs
        .get(&id)
        .cloned()
        .unwrap_or_else(|| Morph {
            base: s.worlds[i].vertices.iter().map(|p| f.map(*p)).collect(),
            keys: vec![],
            normals: s
                .world3d
                .surfaces
                .get(&id)
                .map_or_else(Vec::new, |s| s.triangles.iter().map(|t| t.n).collect()),
        }))
}
fn bake(s: &mut Scene, id: u64, m: Morph) -> Result<(), String> {
    if !m.valid() {
        return Err("Shape key needs matching vertices, unique name, finite delta and weight0..1 (max32 keys)".into());
    }
    let i = index(s, id)?;
    let node = world3d::node(s, id).unwrap();
    s.world3d.nodes.entry(id).or_insert(node);
    let f = node.frame;
    s.worlds[i].vertices = m.vertices().iter().map(|p| f.map(*p)).collect();
    if let Some(surface) = s.world3d.surfaces.get_mut(&id) {
        if surface.triangles.len() != m.normals.len() {
            return Err("Shape key surface topology changed".into());
        }
        let zero = m.keys.iter().all(|k| k.weight == 0.);
        for (t, n) in surface.triangles.iter_mut().zip(&m.normals) {
            t.n = if zero { *n } else { None };
        }
    }
    s.world3d.editing.morphs.insert(id, m);
    Ok(())
}
pub fn prepare(
    d: &Document,
    e: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if let Edit::Fit { rig, change, base } = e {
        let inner = crate::mannequin_fit::prepare(d, rig, change, base, budget, cancel)?;
        return Ok(Prepared { inner, frame:d.current_frame, summary:"Original mannequin proportions / stylized perspective / explicit assistance. Collision uses limb capsules, not fingers, imported models or cloth. Ground aligns the lowest foot vertex to world Y; upright retains locked axes. Pose clips require explicit base scope.".into() });
    }
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Model edit snapshot exceeds memory budget".into());
    }
    let actor = match &e {
        Edit::Expressions { rig, base, .. }
        | Edit::Joint { rig, base, .. }
        | Edit::Control { rig, base, .. }
        | Edit::Copy { rig, base, .. } => Some((*rig, *base)),
        _ => None,
    };
    let (mut state, summary, affected) = if let Some((id, base)) = actor {
        let mut r = d
            .scene
            .world3d
            .rigs
            .get(&id)
            .cloned()
            .ok_or("Choose an original rig")?;
        let summary = match e {
            Edit::Expressions { weights, .. } => {
                r.expressions = weights;
                "Edit original expression morph weights"
            }
            Edit::Joint { joint, angles, .. } => {
                *r.angles.get_mut(joint).ok_or("Joint removed")? = angles;
                "Edit constrained joint pose"
            }
            Edit::Control { joint, value, .. } => {
                let mut c = controls(&r);
                if joint >= c.len() {
                    return Err("Joint removed".into());
                }
                c[joint] = value;
                r.controls = c;
                "Edit joint locks and axis ranges"
            }
            Edit::Copy { side, .. } => {
                r = copy_side(&r, side)?;
                "Copy verified left/right pose; retain unmapped center bones"
            }
            _ => unreachable!(),
        };
        let inner = human_rig::prepare(
            d,
            if base {
                human_rig::Edit::ReplaceBase { id, rig: r }
            } else {
                human_rig::Edit::Replace { id, rig: r }
            },
            budget,
            cancel,
        )?
        .named(summary);
        let mut state = d.snapshot();
        state.scene = inner.scene().clone();
        (state, summary, vec![id])
    } else {
        let mut state = d.snapshot();
        let s = &mut state.scene;
        let (summary, affected) = match e {
            Edit::AddKey { model, name, scale } => {
                crate::model_library::editable(d, &BTreeSet::from([model]))?;
                if scale
                    .iter()
                    .any(|x| !x.is_finite() || !(0.1..=4.).contains(x))
                {
                    return Err("Shape target scale needs0.1..4 per axis".into());
                }
                let mut m = fresh_morph(s, model)?;
                let delta = m
                    .base
                    .iter()
                    .map(|p| std::array::from_fn(|i| p[i] * (scale[i] - 1.)))
                    .collect();
                m.keys.push(ShapeKey {
                    name,
                    delta,
                    weight: 0.,
                });
                bake(s, model, m)?;
                (
                    "Register independent shape target from local-axis scale",
                    vec![model],
                )
            }
            Edit::CaptureKey {
                model,
                source,
                name,
            } => {
                crate::model_library::editable(d, &BTreeSet::from([model]))?;
                if model == source {
                    return Err("Choose a different shape target model".into());
                }
                let mut m = fresh_morph(s, model)?;
                let a = index(s, model)?;
                let b = index(s, source)?;
                let topology = |id| {
                    s.world3d
                        .surfaces
                        .get(&id)
                        .map(|s| s.triangles.iter().map(|t| t.v).collect::<Vec<_>>())
                };
                if s.worlds[a].edges != s.worlds[b].edges
                    || s.worlds[b].vertices.len() != m.base.len()
                    || topology(model) != topology(source)
                {
                    return Err("Shape target requires identical indexed vertex / edge / triangle topology; no automatic remapping".into());
                }
                let f = world3d::node(s, source).unwrap().frame.inverse();
                let delta = s.worlds[b]
                    .vertices
                    .iter()
                    .zip(&m.base)
                    .map(|(p, b)| sub(f.map(*p), *b))
                    .collect();
                m.keys.push(ShapeKey {
                    name,
                    delta,
                    weight: 0.,
                });
                bake(s, model, m)?;
                (
                    "Capture frozen shape target from matching local model topology",
                    vec![model],
                )
            }
            Edit::Weight { model, key, value } => {
                crate::model_library::editable(d, &BTreeSet::from([model]))?;
                let mut m = fresh_morph(s, model)?;
                m.keys.get_mut(key).ok_or("Shape key removed")?.weight = value;
                bake(s, model, m)?;
                (
                    "Edit shape key weight; preserve original vertices",
                    vec![model],
                )
            }
            Edit::DeleteKey { model, key } => {
                crate::model_library::editable(d, &BTreeSet::from([model]))?;
                let mut m = fresh_morph(s, model)?;
                if key >= m.keys.len() {
                    return Err("Shape key removed".into());
                }
                m.keys.remove(key);
                if m.keys.is_empty() {
                    let i = index(s, model)?;
                    let f = world3d::node(s, model).unwrap().frame;
                    s.worlds[i].vertices = m.base.iter().map(|p| f.map(*p)).collect();
                    if let Some(surface) = s.world3d.surfaces.get_mut(&model) {
                        for (t, n) in surface.triangles.iter_mut().zip(&m.normals) {
                            t.n = *n;
                        }
                    }
                    s.world3d.editing.morphs.remove(&model);
                } else {
                    bake(s, model, m)?;
                }
                (
                    "Remove shape key; restore base when last key is removed",
                    vec![model],
                )
            }
            Edit::Parent {
                child,
                parent,
                joint,
                keep,
            } => {
                crate::model_library::editable(d, &BTreeSet::from([child]))?;
                index(s, child)?;
                if parent == Some(child) || joint.is_some() && parent.is_none() {
                    return Err("Choose another parent and a bone only when it has a parent".into());
                }
                let old = world3d::node(s, child).unwrap();
                let evaluated = sampled_frames(d)?;
                let current = evaluated[&child];
                let old_parent = if let Some(a) = s.world3d.editing.attachments.get(&child) {
                    crate::pose_sequence::sampled_rig(d, a.parent)?.frames()[a.joint]
                        .then(evaluated[&a.parent])
                } else if let Some(p) = old.parent {
                    evaluated[&p]
                } else {
                    Matrix::default()
                };
                let attachment = if let (Some(parent), Some(joint)) = (parent, joint) {
                    let r = s
                        .world3d
                        .rigs
                        .get(&parent)
                        .ok_or("Parent has no original skeleton")?;
                    Some(Attachment {
                        parent,
                        joint,
                        skeleton: Skeleton::of(r),
                        local: Matrix::default(),
                    })
                } else {
                    None
                };
                let new_parent = if let Some(a) = &attachment {
                    bone(s, a)?
                } else if let Some(p) = parent {
                    index(s, p)?;
                    world3d::node(s, p).unwrap().frame
                } else {
                    Matrix::default()
                };
                let new_sampled = if let Some(a) = &attachment {
                    let r = crate::pose_sequence::sampled_rig(d, a.parent)?;
                    r.frames()
                        .get(a.joint)
                        .copied()
                        .ok_or("Bone removed")?
                        .then(evaluated[&a.parent])
                } else if let Some(p) = parent {
                    *evaluated.get(&p).ok_or("Parent removed")?
                } else {
                    Matrix::default()
                };
                let desired = if keep == Keep::World {
                    current
                } else {
                    current.then(old_parent.inverse()).then(new_sampled)
                };
                let local = desired.then(new_sampled.inverse());
                let frame = local.then(new_parent);
                let delta = old.frame.inverse().then(frame);
                let descendants = crate::model_library::tree(s, child);
                crate::model_library::editable(d, &descendants)?;
                for id in &descendants {
                    let i = index(s, *id)?;
                    let n = world3d::node(s, *id).unwrap();
                    for p in &mut s.worlds[i].vertices {
                        *p = delta.map(*p);
                    }
                    s.world3d.nodes.insert(
                        *id,
                        world3d::Node {
                            frame: n.frame.then(delta),
                            parent: n.parent,
                        },
                    );
                }
                s.world3d.nodes.get_mut(&child).unwrap().parent = parent;
                s.world3d.editing.attachments.remove(&child);
                if let Some(mut a) = attachment {
                    a.local = frame.then(new_parent.inverse());
                    s.world3d.editing.attachments.insert(child, a);
                }
                let joints = s
                    .world3d
                    .rigs
                    .iter()
                    .map(|(id, r)| (*id, r.frames()))
                    .collect();
                frames(s, &joints, None)?;
                (
                    "Change model/bone parent with explicit world/local preservation",
                    descendants.into_iter().collect(),
                )
            }
            _ => unreachable!(),
        };
        (state, summary, affected)
    };
    sync(d, &mut state.scene)?;
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Shape keys / hierarchy / Undo exceed memory budget".into());
    }
    let frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&frozen).map_err(|e| e.to_string())?;
    crate::book_io::check_cancel(cancel)?;
    let inner = world3d::prepared(d, state, summary.into(), affected, vec![])?.named(summary);
    Ok(Prepared {
        inner,
        frame: d.current_frame,
        summary: summary.into(),
    })
}
