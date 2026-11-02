//! Original primitives, local polygon assets, duplication and world alignment.
use crate::{
    document::Document,
    layer_search::Key,
    model_mesh::{Asset, Model, Surface, Triangle},
    scene::WorldMesh,
    sizing::DocumentState,
    world_math::{add, mul},
    world3d::{self, Matrix, V},
};
use std::{collections::BTreeSet, path::PathBuf, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Box,
    Plane,
    Sphere,
    Cylinder,
    Cone,
    Chair,
    Room,
}
impl Shape {
    pub const ALL: [Self; 7] = [
        Self::Box,
        Self::Plane,
        Self::Sphere,
        Self::Cylinder,
        Self::Cone,
        Self::Chair,
        Self::Room,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Plane => "Plane",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Chair => "Chair",
            Self::Room => "Room",
        }
    }
}
struct Builder {
    v: Vec<V>,
    faces: Vec<Vec<usize>>,
    edges: BTreeSet<[usize; 2]>,
}
impl Builder {
    fn new() -> Self {
        Self {
            v: vec![],
            faces: vec![],
            edges: BTreeSet::new(),
        }
    }
    fn face(&mut self, v: Vec<usize>) {
        for i in 0..v.len() {
            let a = v[i];
            let b = v[(i + 1) % v.len()];
            if a != b {
                self.edges.insert([a.min(b), a.max(b)]);
            }
        }
        self.faces.push(v);
    }
    fn cuboid(&mut self, lo: V, hi: V) {
        let start = self.v.len();
        self.v.extend(
            (0..8).map(|k| std::array::from_fn(|i| if k & (1 << i) == 0 { lo[i] } else { hi[i] })),
        );
        for f in [
            [0, 4, 6, 2],
            [1, 3, 7, 5],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 2, 3, 1],
            [4, 5, 7, 6],
        ] {
            self.face(f.map(|i| i + start).to_vec());
        }
    }
    fn model(self, label: &str, cancel: &AtomicBool) -> Result<Model, String> {
        let mut surface = Surface::default();
        for f in &self.faces {
            for t in crate::model_mesh::triangulate(&self.v, f, cancel)? {
                surface.triangles.push(Triangle {
                    v: t.map(|i| f[i] as u32),
                    t: None,
                    n: None,
                    material: 0,
                    smoothing: 0,
                });
            }
        }
        Ok(Model {
            name: label.into(),
            mesh: WorldMesh {
                vertices: self.v,
                edges: self.edges.into_iter().collect(),
                color: [180, 210, 245, 255],
            },
            surface,
        })
    }
}
pub fn primitive(shape: Shape, detail: u16, cancel: &AtomicBool) -> Result<Asset, String> {
    crate::book_io::check_cancel(cancel)?;
    if !(3..=64).contains(&detail) {
        return Err("Primitive detail must be3..64".into());
    }
    let mut b = Builder::new();
    let n = detail as usize;
    match shape {
        Shape::Box => b.cuboid([-1.; 3], [1.; 3]),
        Shape::Plane => {
            b.v = vec![[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
            b.face(vec![0, 1, 2, 3]);
        }
        Shape::Cylinder | Shape::Cone => {
            for i in 0..n {
                let a = i as f64 * std::f64::consts::TAU / n as f64;
                b.v.push([a.cos(), -1., a.sin()]);
            }
            if shape == Shape::Cylinder {
                for i in 0..n {
                    let a = i as f64 * std::f64::consts::TAU / n as f64;
                    b.v.push([a.cos(), 1., a.sin()]);
                }
                for i in 0..n {
                    let j = (i + 1) % n;
                    b.face(vec![i, i + n, j + n, j]);
                }
                b.face((n..2 * n).rev().collect());
            } else {
                let top = b.v.len();
                b.v.push([0., 1., 0.]);
                for i in 0..n {
                    b.face(vec![i, top, (i + 1) % n]);
                }
            }
            b.face((0..n).collect());
        }
        Shape::Sphere => {
            let longitude = n * 2;
            b.v.push([0., 1., 0.]);
            for row in 1..n {
                crate::book_io::check_cancel(cancel)?;
                let phi = row as f64 * std::f64::consts::PI / n as f64;
                for col in 0..longitude {
                    let a = col as f64 * std::f64::consts::TAU / longitude as f64;
                    b.v.push([phi.sin() * a.cos(), phi.cos(), phi.sin() * a.sin()]);
                }
            }
            let bottom = b.v.len();
            b.v.push([0., -1., 0.]);
            for col in 0..longitude {
                let next = (col + 1) % longitude;
                b.face(vec![0, 1 + next, 1 + col]);
                for row in 0..n - 2 {
                    let a = 1 + row * longitude + col;
                    let c = 1 + row * longitude + next;
                    b.face(vec![a, c, c + longitude, a + longitude]);
                }
                let last = 1 + (n - 2) * longitude;
                b.face(vec![last + col, last + next, bottom]);
            }
        }
        Shape::Chair => {
            b.cuboid([-0.8, 0.8, -0.7], [0.8, 1., 0.7]);
            for x in [-0.65, 0.65] {
                for z in [-0.55, 0.55] {
                    b.cuboid([x - 0.1, 0., z - 0.1], [x + 0.1, 0.8, z + 0.1]);
                }
            }
            b.cuboid([-0.8, 1., 0.5], [0.8, 2.1, 0.7]);
        }
        Shape::Room => {
            b.cuboid([-2., -0.1, -2.], [2., 0., 2.]);
            b.cuboid([-2., 0., -2.1], [2., 3., -2.]);
            b.cuboid([-2.1, 0., -2.], [-2., 3., 2.]);
        }
    }
    let mut model = b.model(shape.label(), cancel)?;
    if shape == Shape::Sphere {
        model.surface.normals = model
            .mesh
            .vertices
            .iter()
            .map(|v| crate::world_math::unit(*v))
            .collect();
        for t in &mut model.surface.triangles {
            t.n = Some(t.v);
            t.smoothing = 1;
        }
    }
    let asset = Asset {
        models: vec![model],
        warnings: vec![],
    };
    if !asset.valid() {
        return Err("Generated primitive exceeded model limits".into());
    }
    Ok(asset)
}
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub position: V,
    pub angles: V,
    pub scale: V,
    pub fit: Option<f64>,
    pub hidden: bool,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            angles: [0.; 3],
            scale: [1.; 3],
            fit: Some(2.5),
            hidden: false,
        }
    }
}
impl Placement {
    fn valid(self) -> bool {
        self.position
            .into_iter()
            .chain(self.angles)
            .all(|v| v.is_finite() && v.abs() <= 1e6)
            && self
                .scale
                .into_iter()
                .all(|n| n.is_finite() && (1e-4..=1e4).contains(&n.abs()))
            && self
                .fit
                .is_none_or(|n| n.is_finite() && (0.001..=10000.).contains(&n))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Min,
    Center,
    Max,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignTarget {
    First,
    Selection,
}
pub enum Edit {
    Primitive {
        shape: Shape,
        detail: u16,
        placement: Placement,
    },
    Import {
        path: PathBuf,
        placement: Placement,
    },
    Duplicate {
        ids: Vec<u64>,
        offset: V,
    },
    Align {
        ids: Vec<u64>,
        axis: usize,
        anchor: Anchor,
        target: AlignTarget,
    },
}
fn limits(d: &Document, budget: usize) -> Result<(), String> {
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Model preparation exceeds document memory budget".into());
    }
    Ok(())
}
fn idx(s: &crate::scene::Scene, id: u64) -> Result<usize, String> {
    s.catalog
        .worlds
        .index(id, s.worlds.len())
        .ok_or_else(|| "Model selection was removed".into())
}
fn tree(s: &crate::scene::Scene, id: u64) -> BTreeSet<u64> {
    let mut set = BTreeSet::from([id]);
    loop {
        let n = set.len();
        for (id, node) in &s.world3d.nodes {
            if node.parent.is_some_and(|p| set.contains(&p)) {
                set.insert(*id);
            }
        }
        if n == set.len() {
            return set;
        }
    }
}
fn roots(s: &crate::scene::Scene, ids: &[u64]) -> Result<Vec<u64>, String> {
    if ids.is_empty() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("Select unique 3D models".into());
    }
    for id in ids {
        idx(s, *id)?;
    }
    let roots = ids
        .iter()
        .copied()
        .filter(|id| !ids.iter().any(|p| p != id && tree(s, *p).contains(id)))
        .collect::<Vec<_>>();
    let mut order = vec![];
    for id in ids {
        if let Some(root) = roots.iter().find(|r| tree(s, **r).contains(id))
            && !order.contains(root)
        {
            order.push(*root);
        }
    }
    Ok(order)
}
fn editable(d: &Document, ids: &BTreeSet<u64>) -> Result<(), String> {
    for id in ids {
        let f = crate::layer_catalog::properties(d, Key::World(*id));
        if f.hidden || f.locked || f.position_locked || f.pixels_locked {
            return Err(format!("Model{id} or a child is hidden / locked"));
        }
        if d.scene.file_objects.get(Key::World(*id)).is_some() {
            return Err("Detach a file-object replacement before editing its3D geometry".into());
        }
    }
    Ok(())
}
fn shift(s: &mut crate::scene::Scene, ids: &BTreeSet<u64>, delta: V) -> Result<(), String> {
    for id in ids {
        let i = idx(s, *id)?;
        let node = world3d::node(s, *id).unwrap();
        for v in &mut s.worlds[i].vertices {
            *v = add(*v, delta);
        }
        s.world3d.nodes.insert(
            *id,
            world3d::Node {
                frame: node.frame.then(Matrix::translation(delta)),
                parent: node.parent,
            },
        );
    }
    Ok(())
}
fn bounds(s: &crate::scene::Scene, ids: &BTreeSet<u64>) -> Result<(V, V), String> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for id in ids {
        let i = idx(s, *id)?;
        for p in &s.worlds[i].vertices {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    if !lo[0].is_finite() {
        return Err("Selected models have no geometry".into());
    }
    Ok((lo, hi))
}
fn place(
    d: &Document,
    mut asset: Asset,
    p: Placement,
    cancel: &AtomicBool,
) -> Result<(DocumentState, Vec<u64>, Vec<String>), String> {
    if !p.valid() || !asset.valid() {
        return Err("Invalid placement or model data".into());
    }
    if d.scene.worlds.len() + asset.models.len() > 256 {
        return Err("Document model limit256".into());
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for m in &asset.models {
        for v in &m.mesh.vertices {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
    }
    let center = mul(add(lo, hi), 0.5);
    let longest = (0..3).map(|i| hi[i] - lo[i]).fold(0., f64::max);
    let fit = if let Some(size) = p.fit {
        if longest <= 1e-12 {
            return Err("Point-only geometry cannot fit to size".into());
        }
        size / longest
    } else {
        1.
    };
    if !(1e-4..=1e4).contains(&fit) {
        return Err("Model units are outside fit range; adjust source units or disable fit".into());
    }
    let basis = Matrix::scale(p.scale.map(|n| n * fit)).then(Matrix::euler(p.angles));
    let transform = Matrix::translation(mul(center, -1.))
        .then(basis)
        .then(Matrix::translation(p.position));
    let mut s = d.snapshot();
    let (camera, lens) = world3d::default_camera(d);
    s.scene.camera = Some(camera);
    s.scene.world3d.lens = Some(lens);
    s.scene.catalog.worlds.initialize(s.scene.worlds.len())?;
    let mut ids = vec![];
    for mut m in asset.models.drain(..) {
        crate::book_io::check_cancel(cancel)?;
        let pivot = transform.map(world3d::center(&m.mesh));
        for v in &mut m.mesh.vertices {
            *v = transform.map(*v);
        }
        let i = s.scene.worlds.len();
        s.scene.worlds.push(m.mesh);
        s.scene.catalog.worlds.append(s.scene.worlds.len())?;
        let id = s.scene.catalog.worlds.id(i);
        ids.push(id);
        let info = s.scene.catalog.world_info.entry(id).or_default();
        info.name = Some(m.name.chars().take(64).collect());
        info.properties.hidden = p.hidden;
        s.scene.world3d.nodes.insert(
            id,
            world3d::Node {
                frame: basis.then(Matrix::translation(pivot)),
                parent: None,
            },
        );
        s.scene.world3d.surfaces.insert(id, m.surface);
    }
    Ok((s, ids, asset.warnings))
}
fn primitive_reservation(shape: Shape, detail: u16) -> usize {
    let n = detail as usize;
    let (v, e, t) = match shape {
        Shape::Box => (8, 12, 12),
        Shape::Plane => (4, 4, 2),
        Shape::Chair => (48, 72, 72),
        Shape::Room => (24, 36, 36),
        Shape::Sphere => (2 * n * n + 2, 4 * n * n, 4 * n * n),
        Shape::Cylinder => (2 * n, 3 * n, 4 * n),
        Shape::Cone => (n + 1, 2 * n, 2 * n),
    };
    (v * 48 + e * 48 + t * (std::mem::size_of::<Triangle>() + 96)) * 2
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    limits(d, budget)?;
    if let Edit::Primitive { shape, detail, .. } = &edit
        && (!(3..=64).contains(detail)
            || d.memory_bytes()
                .saturating_mul(3)
                .saturating_add(primitive_reservation(*shape, *detail))
                > budget)
    {
        return Err("Primitive geometry / topology reservation exceeds available memory".into());
    }
    let (state, ids, warnings, summary) = match edit {
        Edit::Primitive {
            shape,
            detail,
            placement,
        } => {
            let (s, ids, w) = place(d, primitive(shape, detail, cancel)?, placement, cancel)?;
            (s, ids, w, format!("Place original {} model", shape.label()))
        }
        Edit::Import { path, placement } => {
            let asset = crate::model_obj::load(
                &path,
                budget.saturating_sub(d.memory_bytes().saturating_mul(3)),
                cancel,
            )?;
            let (s, ids, w) = place(d, asset, placement, cancel)?;
            (
                s,
                ids,
                w,
                "Place frozen local OBJ geometry / topology / attributes".into(),
            )
        }
        Edit::Duplicate { ids, offset } => {
            if offset.iter().any(|n| !n.is_finite() || n.abs() > 1e6) {
                return Err("Invalid duplicate offset".into());
            }
            let roots = roots(&d.scene, &ids)?;
            let all = roots
                .iter()
                .flat_map(|id| tree(&d.scene, *id))
                .collect::<BTreeSet<_>>();
            editable(d, &all)?;
            let keys = all.iter().map(|id| Key::World(*id)).collect::<Vec<_>>();
            let (mut s, keys) = crate::layer_move::copy_sources(d, &keys)?;
            let new = keys
                .into_iter()
                .filter_map(|k| {
                    if let Key::World(id) = k {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect::<BTreeSet<_>>();
            shift(&mut s.scene, &new, offset)?;
            (
                s,
                new.into_iter().collect(),
                vec![],
                "Duplicate selected model trees, preserving independent topology / axes / parents"
                    .into(),
            )
        }
        Edit::Align {
            ids,
            axis,
            anchor,
            target,
        } => {
            if axis >= 3 {
                return Err("Choose X, Y or Z alignment".into());
            }
            let roots = roots(&d.scene, &ids)?;
            let sets = roots
                .iter()
                .map(|id| tree(&d.scene, *id))
                .collect::<Vec<_>>();
            let all = sets.iter().flatten().copied().collect::<BTreeSet<_>>();
            editable(d, &all)?;
            let boxes = sets
                .iter()
                .map(|s| bounds(&d.scene, s))
                .collect::<Result<Vec<_>, _>>()?;
            let target_box = if target == AlignTarget::First {
                boxes[0]
            } else {
                bounds(&d.scene, &all)?
            };
            let position = |b: (V, V)| match anchor {
                Anchor::Min => b.0[axis],
                Anchor::Center => (b.0[axis] + b.1[axis]) / 2.,
                Anchor::Max => b.1[axis],
            };
            let goal = position(target_box);
            let mut s = d.snapshot();
            for (set, b) in sets.iter().zip(boxes) {
                crate::book_io::check_cancel(cancel)?;
                let mut delta = [0.; 3];
                delta[axis] = goal - position(b);
                shift(&mut s.scene, set, delta)?;
            }
            (
                s,
                all.into_iter().collect(),
                vec![],
                format!(
                    "Align model tree {:?} on axis{} to {:?}",
                    anchor,
                    axis + 1,
                    target
                ),
            )
        }
    };
    crate::book_io::check_cancel(cancel)?;
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Prepared model / attributes exceed remaining working memory".into());
    }
    world3d::prepared(d, state, summary, ids, warnings)
}
