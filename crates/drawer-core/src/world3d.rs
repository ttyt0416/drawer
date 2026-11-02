//! Editable rigid/articulated world objects and a document camera. Mesh vertices
//! are world-space sources; accumulated frames retain object axes and joint pivots.
pub use crate::world_math::{Matrix, V};
use crate::world_math::{add, mul, sub};
use crate::{
    document::Document,
    layer_search::Key,
    scene::{Camera, Point, Rect, Scene, WorldMesh},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Projection {
    Orthographic,
    Perspective,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    pub projection: Projection,
    pub angles: V,
    pub fov: f64,
    pub near: f64,
    pub far: f64,
}
impl Default for Lens {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective,
            angles: [0.; 3],
            fov: 45.,
            near: 0.01,
            far: 10000.,
        }
    }
}
pub(crate) fn camera_valid(c: &Camera) -> bool {
    c.page_bounds.valid()
        && (0..2).all(|i| c.page_bounds.min[i] < c.page_bounds.max[i])
        && c.page_bounds
            .min
            .into_iter()
            .chain(c.page_bounds.max)
            .all(|v| v.abs() <= 1e9)
        && c.position.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
        && c.pixels_per_unit.is_finite()
        && (1e-6..=1e6).contains(&c.pixels_per_unit)
}
impl Lens {
    pub fn valid(self) -> bool {
        self.angles
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= 36000.)
            && self.fov.is_finite()
            && (1. ..=179.).contains(&self.fov)
            && self.near.is_finite()
            && self.far.is_finite()
            && self.near > 0.
            && self.far > self.near
            && self.far <= 1e9
    }
    fn view(self, c: &Camera, p: V) -> V {
        Matrix::euler(self.angles)
            .inverse()
            .vector(sub(p, c.position))
    }
    fn project_view(self, c: &Camera, p: V) -> Point {
        let center =
            std::array::from_fn::<_, 2, _>(|i| (c.page_bounds.min[i] + c.page_bounds.max[i]) / 2.);
        let scale = match self.projection {
            Projection::Orthographic => c.pixels_per_unit,
            Projection::Perspective => {
                (c.page_bounds.max[1] - c.page_bounds.min[1])
                    / 2.
                    / (self.fov.to_radians() / 2.).tan()
                    / -p[2]
            }
        };
        [center[0] + p[0] * scale, center[1] - p[1] * scale]
    }
    pub fn project(self, c: &Camera, p: V) -> Option<Point> {
        let p = self.view(c, p);
        let z = -p[2];
        (z >= self.near && z <= self.far).then(|| self.project_view(c, p))
    }
    pub fn segment(self, c: &Camera, a: V, b: V) -> Option<[Point; 2]> {
        let a = self.view(c, a);
        let b = self.view(c, b);
        let za = -a[2];
        let dz = -b[2] - za;
        let (mut lo, mut hi) = (0f64, 1f64);
        if dz.abs() < 1e-15 {
            if za < self.near || za > self.far {
                return None;
            }
        } else {
            let x = (self.near - za) / dz;
            let y = (self.far - za) / dz;
            lo = lo.max(x.min(y));
            hi = hi.min(x.max(y));
            if lo > hi {
                return None;
            }
        }
        let delta = sub(b, a);
        Some([
            self.project_view(c, add(a, mul(delta, lo))),
            self.project_view(c, add(a, mul(delta, hi))),
        ])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub frame: Matrix,
    pub parent: Option<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub nodes: BTreeMap<u64, Node>,
    pub lens: Option<Lens>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub surfaces: BTreeMap<u64, crate::model_mesh::Surface>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rigs: BTreeMap<u64, crate::human_rig::Rig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<crate::lighting::Settings>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub colliders: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<crate::environment::Data>,
    #[serde(
        default,
        skip_serializing_if = "crate::pose_sequence::Data::is_default"
    )]
    pub sequences: crate::pose_sequence::Data,
}
fn index(s: &Scene, id: u64) -> Option<usize> {
    s.catalog.worlds.index(id, s.worlds.len())
}
pub fn node(s: &Scene, id: u64) -> Option<Node> {
    let mesh = &s.worlds[index(s, id)?];
    Some(s.world3d.nodes.get(&id).copied().unwrap_or_else(|| Node {
        frame: Matrix::translation(center(mesh)),
        parent: None,
    }))
}
pub fn center(m: &WorldMesh) -> V {
    if m.vertices.is_empty() {
        return [0.; 3];
    }
    std::array::from_fn(|i| {
        let (lo, hi) = m
            .vertices
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v[i]), hi.max(v[i]))
            });
        (lo + hi) / 2.
    })
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn bytes(&self) -> usize {
        self.sequences.bytes()
            + self.environment.as_ref().map_or(0, |e| e.bytes())
            + self.surfaces.values().map(|s| s.bytes()).sum::<usize>()
            + self.rigs.values().map(|r| r.bytes()).sum::<usize>()
            + self.lighting.as_ref().map_or(0, |s| s.bytes())
            + self.colliders.len() * 32
            + self.nodes.len() * std::mem::size_of::<(u64, Node)>()
            + usize::from(self.lens.is_some()) * std::mem::size_of::<Lens>()
    }
    pub fn valid(&self, s: &Scene) -> bool {
        if !self.sequences.valid(s) {
            return false;
        }
        if self
            .environment
            .as_ref()
            .is_some_and(|e| e.pixels().is_err() || self.lens.is_none())
        {
            return false;
        }
        if self.lighting.as_ref().is_some_and(|v| {
            !v.valid()
                || self.lens.is_none()
                || self
                    .surfaces
                    .values()
                    .map(|s| s.triangles.len())
                    .sum::<usize>()
                    > 262144
        }) {
            return false;
        }
        if self.colliders.len() > 256 || self.colliders.iter().any(|id| index(s, *id).is_none()) {
            return false;
        }
        if self.nodes.len() > 256
            || self
                .lens
                .is_some_and(|l| !l.valid() || s.camera.as_ref().is_none_or(|c| !camera_valid(c)))
            || (!self.is_default()
                && s.worlds
                    .iter()
                    .map(|m| m.vertices.len() + m.edges.len())
                    .sum::<usize>()
                    > 524288)
        {
            return false;
        }
        if self.surfaces.len() > 256
            || self
                .surfaces
                .iter()
                .any(|(id, surface)| index(s, *id).is_none_or(|i| !surface.valid(&s.worlds[i])))
        {
            return false;
        }
        if self.rigs.len() > 32
            || self.rigs.iter().any(|(id, r)| {
                index(s, *id).is_none_or(|i| {
                    self.nodes.get(id).is_none_or(|n| {
                        !self.surfaces.contains_key(id)
                            || !crate::human_rig::binding_valid(r, &s.worlds[i], n.frame)
                    })
                })
            })
        {
            return false;
        }
        for (&id, n) in &self.nodes {
            if index(s, id).is_none() || !n.frame.valid() {
                return false;
            }
            let mut seen = BTreeSet::from([id]);
            let mut parent = n.parent;
            let mut depth = 0;
            while let Some(p) = parent {
                depth += 1;
                if depth > 16 || index(s, p).is_none() || !seen.insert(p) {
                    return false;
                }
                parent = self.nodes.get(&p).and_then(|n| n.parent);
            }
        }
        true
    }
}
/// Removing a parent detaches surviving children in place: sources and frames
/// are already world-space, so this never changes their visible pose.
pub(crate) fn reconcile(s: &mut Scene) {
    let ids = (0..s.worlds.len())
        .map(|i| s.catalog.worlds.id(i))
        .collect::<BTreeSet<_>>();
    s.world3d.nodes.retain(|id, _| ids.contains(id));
    s.world3d.surfaces.retain(|id, _| ids.contains(id));
    s.world3d.rigs.retain(|id, _| ids.contains(id));
    s.world3d.colliders.retain(|id| ids.contains(id));
    crate::pose_sequence::prune(s);
    for n in s.world3d.nodes.values_mut() {
        if n.parent.is_some_and(|p| !ids.contains(&p)) {
            n.parent = None;
        }
    }
    if s.camera.is_none() {
        s.world3d.lens = None;
        s.world3d.lighting = None;
    }
}
pub(crate) fn duplicate(
    s: &mut Scene,
    original: &Scene,
    map: &BTreeMap<Key, Key>,
) -> Result<(), String> {
    for (old, new) in map {
        if let (Key::World(old), Key::World(new)) = (old, new)
            && original.world3d.colliders.contains(old)
        {
            s.world3d.colliders.insert(*new);
        }
    }
    for (old, new) in map {
        if let (Key::World(old), Key::World(new)) = (old, new)
            && let Some(r) = original.world3d.rigs.get(old)
        {
            s.world3d.rigs.insert(*new, r.clone());
        }
    }
    for (old, new) in map {
        if let (Key::World(old), Key::World(new)) = (old, new)
            && let Some(surface) = original.world3d.surfaces.get(old)
        {
            s.world3d.surfaces.insert(*new, surface.clone());
        }
    }
    for (old, new) in map {
        if let (Key::World(old), Key::World(new)) = (old, new)
            && let Some(mut node) = original.world3d.nodes.get(old).copied()
        {
            node.parent = node.parent.map(|p| match map.get(&Key::World(p)) {
                Some(Key::World(new)) => *new,
                _ => p,
            });
            s.world3d.nodes.insert(*new, node);
        }
    }
    crate::pose_sequence::duplicate(s, original, map)?;
    Ok(())
}
/// Page-layer movement is parallel to the camera image plane at the joint's
/// depth. It also carries the retained frame, without moving unselected layers.
pub(crate) fn page_delta(s: &Scene, id: u64, delta: Point) -> Result<V, String> {
    let c = s.camera.as_ref().ok_or("Missing 3D camera")?;
    let Some(lens) = s.world3d.lens else {
        return Ok([
            delta[0] / c.pixels_per_unit,
            -delta[1] / c.pixels_per_unit,
            0.,
        ]);
    };
    let pivot = node(s, id).ok_or("Missing 3D object")?.frame.origin();
    let depth = -lens.view(c, pivot)[2];
    if depth < lens.near || depth > lens.far {
        return Err("3D pivot is outside camera depth; move it in the 3D editor".into());
    }
    let scale = match lens.projection {
        Projection::Orthographic => c.pixels_per_unit,
        Projection::Perspective => {
            (c.page_bounds.max[1] - c.page_bounds.min[1])
                / 2.
                / (lens.fov.to_radians() / 2.).tan()
                / depth
        }
    };
    Ok(Matrix::euler(lens.angles).vector([delta[0] / scale, -delta[1] / scale, 0.]))
}
pub(crate) fn validate_edit(d: &Document, s: &Scene) -> Result<(), String> {
    for i in 0..d.scene.worlds.len() {
        let id = d.scene.catalog.worlds.id(i);
        // Source deletion follows the existing layer deletion policy. Retained
        // children still have their own parent-link / frame lock checks below.
        if index(s, id).is_none() {
            continue;
        }
        let p = crate::layer_catalog::properties(d, Key::World(id));
        let a = node(&d.scene, id);
        let b = node(s, id);
        let changed = b != a;
        if (d.scene.world3d.surfaces.get(&id) != s.world3d.surfaces.get(&id)
            || d.scene.world3d.rigs.get(&id) != s.world3d.rigs.get(&id))
            && (p.locked || p.hidden || p.pixels_locked)
        {
            return Err("3D surface is hidden / locked".into());
        }
        if changed && (p.locked || p.hidden || p.position_locked || p.pixels_locked) {
            return Err(format!("3D object{id} or a descendant is hidden / locked"));
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axes {
    #[default]
    Floor,
    Object,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pivot {
    #[default]
    Individual,
    SelectionCenter,
    FirstSelected,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Move,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub axes: Axes,
    pub pivot: Pivot,
    pub mode: Mode,
    pub step: f64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            axes: Axes::Floor,
            pivot: Pivot::Individual,
            mode: Mode::Move,
            step: 15.,
        }
    }
}
impl Preferences {
    pub fn valid(self) -> bool {
        self.version == 1 && self.step.is_finite() && (0. ..=180.).contains(&self.step)
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local 3D preferences".into());
        }
        let p = root.join("settings/world3d.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let p: Self = serde_json::from_slice(&crate::book_io::read_local(&p, 4096)?)
            .map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid 3D preferences; file preserved".into());
        }
        Ok(p)
    }
    pub fn save(self, root: &Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid local 3D preferences".into());
        }
        let p = root.join("settings");
        std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
        let mut stage = crate::animation_export::Stage::new(&p)?;
        let mut f = stage.create("world3d.json")?;
        f.write_all(&serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(stage.directory.join("world3d.json"), p.join("world3d.json"))
            .map_err(|e| e.to_string())
    }
}
pub fn pivot(s: &Scene, ids: &[u64], mode: Pivot) -> Result<V, String> {
    let first = *ids.first().ok_or("Select a 3D object")?;
    if mode != Pivot::SelectionCenter {
        return node(s, first)
            .map(|n| n.frame.origin())
            .ok_or_else(|| "3D selection removed".into());
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &id in ids {
        let m = &s.worlds[index(s, id).ok_or("3D selection removed")?];
        for p in &m.vertices {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
    }
    if !min[0].is_finite() {
        return Err("Selected models have no vertices".into());
    }
    Ok(std::array::from_fn(|i| (min[i] + max[i]) / 2.))
}
pub enum Edit {
    Reference,
    Transform {
        ids: Vec<u64>,
        options: Preferences,
        value: V,
    },
    Parent {
        ids: Vec<u64>,
        parent: Option<u64>,
    },
    Camera {
        camera: Camera,
        lens: Lens,
    },
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    pub summary: String,
    pub affected: Vec<u64>,
    pub warnings: Vec<String>,
    history_label: Option<String>,
}
impl Prepared {
    pub fn scene(&self) -> &Scene {
        &self.state.scene
    }
    pub fn matches(&self, d: &Document) -> bool {
        d.id == self.id && d.revision == self.revision
    }
    pub(crate) fn named(mut self, label: &str) -> Self {
        self.history_label = Some(label.into());
        self
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("3D preview is stale; prepare again".into());
        }
        if let Some(label) = self.history_label {
            d.commit_state_named(self.state, &label)
        } else {
            d.commit_state(self.state)
        }
    }
}
fn camera(d: &Document) -> Camera {
    d.scene.camera.clone().unwrap_or_else(|| {
        let inv = d.scene.transform.inverse();
        let corners = [
            [0., 0.],
            [d.width as f64, 0.],
            [d.width as f64, d.height as f64],
            [0., d.height as f64],
        ]
        .map(|p| inv.map(p));
        let bounds = Rect {
            min: std::array::from_fn(|i| {
                corners.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min)
            }),
            max: std::array::from_fn(|i| {
                corners
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        };
        Camera {
            pixels_per_unit: (bounds.max[0] - bounds.min[0]).min(bounds.max[1] - bounds.min[1])
                / 5.,
            page_bounds: bounds,
            position: [0., 0., 6.],
        }
    })
}

pub fn default_camera(d: &Document) -> (Camera, Lens) {
    (camera(d), d.scene.world3d.lens.unwrap_or_default())
}
fn descendants(s: &Scene, root: u64) -> BTreeSet<u64> {
    let mut set = BTreeSet::from([root]);
    loop {
        let n = set.len();
        for (&id, node) in &s.world3d.nodes {
            if node.parent.is_some_and(|p| set.contains(&p)) {
                set.insert(id);
            }
        }
        if n == set.len() {
            return set;
        }
    }
}
fn ensure_nodes(s: &mut Scene) {
    let entries = (0..s.worlds.len())
        .map(|i| {
            let id = s.catalog.worlds.id(i);
            (id, node(s, id).unwrap())
        })
        .collect::<Vec<_>>();
    for (id, n) in entries {
        s.world3d.nodes.entry(id).or_insert(n);
    }
}
fn reference(s: &mut Scene) -> Result<Vec<u64>, String> {
    if s.worlds.len() > 253 {
        return Err("3D object limit256".into());
    }
    s.catalog.worlds.initialize(s.worlds.len())?;
    let first = s.worlds.len();
    // Original articulated reference: a base and two hinged rectangular links.
    let mut parent = None;
    let mut ids = vec![];
    for (n, (lo, hi, joint)) in [
        ([-0.8, -1., -0.35], [0.8, -0.3, 0.35], [0., -0.65, 0.]),
        ([-0.18, -0.3, -0.18], [0.18, 0.7, 0.18], [0., -0.3, 0.]),
        ([-0.14, 0.7, -0.14], [0.14, 1.5, 0.14], [0., 0.7, 0.]),
    ]
    .into_iter()
    .enumerate()
    {
        let vertices = (0..8)
            .map(|k| std::array::from_fn(|i| if k & (1 << i) == 0 { lo[i] } else { hi[i] }))
            .collect();
        let edges = (0usize..8)
            .flat_map(|k| {
                (0..3).filter_map(move |i| (k & (1 << i) == 0).then_some([k, k | (1 << i)]))
            })
            .collect();
        s.worlds.push(WorldMesh {
            vertices,
            edges,
            color: [180, 210, 245, 255],
        });
        s.catalog.worlds.append(s.worlds.len())?;
        let id = s.catalog.worlds.id(first + n);
        s.world3d.nodes.insert(
            id,
            Node {
                frame: Matrix::translation(joint),
                parent,
            },
        );
        s.catalog.world_info.entry(id).or_default().name = Some(format!("Reference {}", n + 1));
        ids.push(id);
        parent = Some(id);
    }
    Ok(ids)
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes().saturating_mul(3) > budget
        || d.scene
            .worlds
            .iter()
            .map(|m| m.vertices.len() + m.edges.len())
            .sum::<usize>()
            > 524288
    {
        return Err("3D editing snapshot / geometry budget exceeded".into());
    }
    let mut state = d.snapshot();
    let s = &mut state.scene;
    ensure_nodes(s);
    let mut affected = vec![];
    let summary = match edit {
        Edit::Reference => {
            s.camera = Some(camera(d));
            s.world3d.lens = Some(d.scene.world3d.lens.unwrap_or_default());
            affected = reference(s)?;
            "Place original articulated reference model".into()
        }
        Edit::Camera { camera, lens } => {
            if !lens.valid() || !camera_valid(&camera) {
                return Err("Invalid 3D camera, lens or depth range".into());
            }
            s.camera = Some(camera);
            s.world3d.lens = Some(lens);
            "Edit document 3D camera; one Undo restores projection".into()
        }
        Edit::Parent { ids, parent } => {
            if ids.is_empty()
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                || parent.is_some_and(|p| index(s, p).is_none())
            {
                return Err("Select existing 3D objects and parent".into());
            }
            for id in ids {
                let n = s
                    .world3d
                    .nodes
                    .get_mut(&id)
                    .ok_or("Selected object removed")?;
                n.parent = parent;
                affected.push(id);
            }
            if !s.world3d.valid(s) {
                return Err("Parent relation would cycle or exceed16 levels".into());
            }
            "Set articulated parent links while preserving world pose".into()
        }
        Edit::Transform {
            ids,
            options,
            value,
        } => {
            if !options.valid()
                || ids.is_empty()
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                || value.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
                || ids.iter().any(|id| index(s, *id).is_none())
            {
                return Err("Choose unique 3D objects and finite transform values".into());
            }
            if options.mode == Mode::Scale
                && value.iter().any(|v| v.abs() < 0.0001 || v.abs() > 10000.)
            {
                return Err("Scale may mirror but must be nonzero (absolute0.0001..10000)".into());
            }
            let common = pivot(s, &ids, options.pivot)?;
            let roots = ids
                .iter()
                .copied()
                .filter(|id| {
                    !ids.iter()
                        .any(|other| other != id && descendants(s, *other).contains(id))
                })
                .collect::<Vec<_>>();
            for id in roots {
                let frame = s.world3d.nodes[&id].frame;
                let basis = if options.axes == Axes::Object {
                    frame.basis()
                } else {
                    Matrix::default()
                };
                if !basis.valid() {
                    return Err("Object axes are ill-conditioned".into());
                }
                let p = if options.pivot == Pivot::Individual {
                    frame.origin()
                } else {
                    common
                };
                let delta = match options.mode {
                    Mode::Move => Matrix::translation(basis.vector(value)),
                    Mode::Rotate => {
                        let v = if options.step > 0. {
                            value.map(|v| (v / options.step).round() * options.step)
                        } else {
                            value
                        };
                        (0..3)
                            .fold(Matrix::default(), |m, i| {
                                m.then(Matrix::rotation(basis.axis(i), v[i]))
                            })
                            .around(p)
                    }
                    Mode::Scale => basis
                        .inverse()
                        .then(Matrix::scale(value))
                        .then(basis)
                        .around(p),
                };
                for child in descendants(s, id) {
                    crate::book_io::check_cancel(cancel)?;
                    let i = index(s, child).ok_or("3D child removed")?;
                    for v in &mut s.worlds[i].vertices {
                        *v = delta.map(*v);
                    }
                    let n = s.world3d.nodes.get_mut(&child).unwrap();
                    n.frame = n.frame.then(delta);
                    affected.push(child);
                }
            }
            format!(
                "{:?} {} objects using {:?} axes / {:?} pivot (parents carry descendants once)",
                options.mode,
                affected.len(),
                options.axes,
                options.pivot
            )
        }
    };
    crate::book_io::check_cancel(cancel)?;
    prepared(d, state, summary, affected, vec![])
}
pub(crate) fn prepared(
    d: &Document,
    mut state: DocumentState,
    summary: String,
    affected: Vec<u64>,
    warnings: Vec<String>,
) -> Result<Prepared, String> {
    if !state.scene.world3d.valid(&state.scene) {
        return Err("3D transform / surface is singular, cyclic or outside limits".into());
    }
    validate_edit(d, &state.scene)?;
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::layer_groups::finish(d, &state)?;
    Ok(Prepared {
        state,
        id: d.id,
        revision: d.revision,
        summary,
        affected,
        warnings,
        history_label: None,
    })
}

pub fn segments(s: &Scene, index: usize) -> Vec<[Point; 2]> {
    let Some(mesh) = s.worlds.get(index) else {
        return vec![];
    };
    segments_mesh(s, mesh)
}
pub(crate) fn segments_mesh(s: &Scene, mesh: &WorldMesh) -> Vec<[Point; 2]> {
    let (Some(c), Some(l)) = (&s.camera, s.world3d.lens) else {
        return vec![];
    };
    mesh.edges
        .iter()
        .filter_map(|e| l.segment(c, *mesh.vertices.get(e[0])?, *mesh.vertices.get(e[1])?))
        .collect()
}
