//! Original parametric drawing mannequins. Each rigid skin piece follows one joint.
use crate::{
    document::Document,
    model_mesh::Surface,
    scene::WorldMesh,
    world3d::{self, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Body,
    Head,
    Hand,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub height: f64,
    pub shoulders: f64,
    pub hips: f64,
    pub bulk: f64,
    pub head: f64,
    pub hands: f64,
    pub face: f64,
    pub jaw: f64,
    pub palm: f64,
    pub fingers: f64,
}
impl Default for Shape {
    fn default() -> Self {
        Self {
            height: 1.8,
            shoulders: 1.,
            hips: 1.,
            bulk: 1.,
            head: 1.,
            hands: 1.,
            face: 1.,
            jaw: 1.,
            palm: 1.,
            fingers: 1.,
        }
    }
}
impl Shape {
    pub fn valid(self) -> bool {
        (0.5..=3.).contains(&self.height)
            && [
                self.shoulders,
                self.hips,
                self.bulk,
                self.head,
                self.hands,
                self.face,
                self.jaw,
                self.palm,
                self.fingers,
            ]
            .into_iter()
            .all(|v| (0.5..=2.).contains(&v))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    pub version: u32,
    pub kind: Kind,
    pub shape: Shape,
    pub angles: Vec<V>,
    pub mirrored: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<crate::model_edit::Control>,
    #[serde(default, skip_serializing_if = "zero_expression")]
    pub expressions: [f64; 3],
}
fn zero_expression(v: &[f64; 3]) -> bool {
    *v == [0.; 3]
}
#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub bind: Matrix,
    pub min: V,
    pub max: V,
}
struct Piece {
    joint: usize,
    round: bool,
    transform: Matrix,
}
struct Definition {
    joints: Vec<Joint>,
    pieces: Vec<Piece>,
}
fn reflected(m: Matrix) -> Matrix {
    let f = Matrix::scale([-1., 1., 1.]);
    f.then(m).then(f)
}
impl Definition {
    // Explicit bind, limits and handedness keep the procedural skeleton table readable.
    #[allow(clippy::too_many_arguments)]
    fn joint(
        &mut self,
        name: String,
        parent: Option<usize>,
        position: V,
        z: f64,
        min: V,
        max: V,
        left: bool,
    ) -> usize {
        let bind = Matrix::euler([0., 0., z]).then(Matrix::translation(position));
        let i = self.joints.len();
        self.joints.push(Joint {
            name,
            parent,
            bind: if left { reflected(bind) } else { bind },
            min,
            max,
        });
        i
    }
    fn piece(&mut self, joint: usize, round: bool, size: V, center: V, left: bool) {
        let mut t = Matrix::scale(size).then(Matrix::translation(center));
        if left {
            t = t.then(Matrix::scale([-1., 1., 1.]));
        }
        self.pieces.push(Piece {
            joint,
            round,
            transform: t,
        });
    }
    fn fingers(
        &mut self,
        parent: usize,
        prefix: &str,
        size: f64,
        palm: f64,
        length: f64,
        left: bool,
    ) {
        self.piece(
            parent,
            false,
            [0.045 * palm * size, 0.065 * size, 0.018 * size],
            [0., 0.06 * size, 0.],
            left,
        );
        for (finger, name) in ["Thumb", "Index", "Middle", "Ring", "Little"]
            .into_iter()
            .enumerate()
        {
            let mut parent = parent;
            let mut last = 0.;
            for segment in 0..3 {
                let len = size
                    * length
                    * [0.035, 0.05, 0.056, 0.05, 0.04][finger]
                    * [1., 0.7, 0.5][segment];
                let pos = if segment == 0 {
                    [
                        ((finger as f64) - 2.) * 0.021 * palm * size,
                        if finger == 0 {
                            0.035 * size
                        } else {
                            0.12 * size
                        },
                        0.,
                    ]
                } else {
                    [0., last, 0.]
                };
                let j = self.joint(
                    format!("{prefix}{name}{}", segment + 1),
                    Some(parent),
                    pos,
                    if finger == 0 && segment == 0 { 40. } else { 0. },
                    [0., -15., -15.],
                    [100., 15., 15.],
                    left,
                );
                self.piece(
                    j,
                    false,
                    [0.008 * size, len / 2., 0.009 * size],
                    [0., len / 2., 0.],
                    left,
                );
                parent = j;
                last = len;
            }
        }
    }
}
fn definition(r: &Rig) -> Definition {
    let s = r.shape;
    let h = s.height / 1.8;
    let mut d = Definition {
        joints: vec![],
        pieces: vec![],
    };
    let root = d.joint(
        match r.kind {
            Kind::Body => "Pelvis",
            Kind::Head => "Neck",
            Kind::Hand => "Wrist",
        }
        .into(),
        None,
        [0.; 3],
        0.,
        [-60., -180., -60.],
        [60., 180., 60.],
        false,
    );
    match r.kind {
        Kind::Body => {
            d.piece(
                root,
                true,
                [0.16 * s.hips * h, 0.12 * h, 0.1 * s.bulk * h],
                [0.; 3],
                false,
            );
            let spine = d.joint(
                "Spine".into(),
                Some(root),
                [0., 0.1 * h, 0.],
                0.,
                [-40., -50., -35.],
                [40., 50., 35.],
                false,
            );
            d.piece(
                spine,
                true,
                [0.13 * h, 0.12 * h, 0.09 * s.bulk * h],
                [0., 0.1 * h, 0.],
                false,
            );
            let chest = d.joint(
                "Chest".into(),
                Some(spine),
                [0., 0.18 * h, 0.],
                0.,
                [-40., -50., -35.],
                [40., 50., 35.],
                false,
            );
            d.piece(
                chest,
                true,
                [0.2 * s.shoulders * h, 0.15 * h, 0.1 * s.bulk * h],
                [0., 0.1 * h, 0.],
                false,
            );
            let neck = d.joint(
                "Neck".into(),
                Some(chest),
                [0., 0.26 * h, 0.],
                0.,
                [-60., -80., -40.],
                [60., 80., 40.],
                false,
            );
            d.piece(
                neck,
                true,
                [0.045 * h, 0.055 * h, 0.045 * h],
                [0., 0.035 * h, 0.],
                false,
            );
            head(&mut d, neck, s, h, r.expressions);
            for (prefix, left) in [("R/", false), ("L/", true)] {
                let shoulder = d.joint(
                    format!("{prefix}Shoulder"),
                    Some(chest),
                    [0.2 * s.shoulders * h, 0.2 * h, 0.],
                    -90.,
                    [-60., -90., -60.],
                    [60., 90., 60.],
                    left,
                );
                let arm = d.joint(
                    format!("{prefix}UpperArm"),
                    Some(shoulder),
                    [0.; 3],
                    0.,
                    [-120., -90., -90.],
                    [120., 90., 90.],
                    left,
                );
                d.piece(
                    arm,
                    true,
                    [0.045 * s.bulk * h, 0.14 * h, 0.045 * s.bulk * h],
                    [0., 0.14 * h, 0.],
                    left,
                );
                let elbow = d.joint(
                    format!("{prefix}Elbow"),
                    Some(arm),
                    [0., 0.28 * h, 0.],
                    0.,
                    [0., -80., 0.],
                    [150., 80., 0.],
                    left,
                );
                d.piece(
                    elbow,
                    true,
                    [0.035 * s.bulk * h, 0.12 * h, 0.035 * s.bulk * h],
                    [0., 0.12 * h, 0.],
                    left,
                );
                let wrist = d.joint(
                    format!("{prefix}Wrist"),
                    Some(elbow),
                    [0., 0.24 * h, 0.],
                    0.,
                    [-80., -45., -40.],
                    [80., 45., 40.],
                    left,
                );
                d.fingers(wrist, prefix, h * s.hands, s.palm, s.fingers, left);
                let hip = d.joint(
                    format!("{prefix}Hip"),
                    Some(root),
                    [0.09 * s.hips * h, -0.08 * h, 0.],
                    180.,
                    [-90., -60., -50.],
                    [140., 60., 50.],
                    left,
                );
                d.piece(
                    hip,
                    true,
                    [0.07 * s.bulk * h, 0.22 * h, 0.07 * s.bulk * h],
                    [0., 0.22 * h, 0.],
                    left,
                );
                let knee = d.joint(
                    format!("{prefix}Knee"),
                    Some(hip),
                    [0., 0.44 * h, 0.],
                    0.,
                    [0., 0., 0.],
                    [150., 0., 0.],
                    left,
                );
                d.piece(
                    knee,
                    true,
                    [0.05 * s.bulk * h, 0.2 * h, 0.05 * s.bulk * h],
                    [0., 0.2 * h, 0.],
                    left,
                );
                let ankle = d.joint(
                    format!("{prefix}Ankle"),
                    Some(knee),
                    [0., 0.4 * h, 0.],
                    0.,
                    [-60., -30., -30.],
                    [60., 30., 30.],
                    left,
                );
                d.piece(
                    ankle,
                    false,
                    [0.055 * h, 0.035 * h, 0.09 * h],
                    [0., 0.025 * h, -0.05 * h],
                    left,
                );
                let toe = d.joint(
                    format!("{prefix}Toe"),
                    Some(ankle),
                    [0., 0.025 * h, -0.13 * h],
                    0.,
                    [0., 0., 0.],
                    [80., 0., 0.],
                    left,
                );
                d.piece(
                    toe,
                    false,
                    [0.055 * h, 0.025 * h, 0.045 * h],
                    [0., 0., -0.035 * h],
                    left,
                );
            }
        }
        Kind::Head => {
            d.piece(
                root,
                true,
                [0.05 * h, 0.07 * h, 0.05 * h],
                [0., 0.04 * h, 0.],
                false,
            );
            head(&mut d, root, s, h, r.expressions);
        }
        Kind::Hand => d.fingers(root, "", h * s.hands, s.palm, s.fingers, false),
    }
    if r.kind == Kind::Hand && r.mirrored {
        for j in &mut d.joints {
            j.bind = reflected(j.bind);
        }
        for p in &mut d.pieces {
            p.transform = p.transform.then(Matrix::scale([-1., 1., 1.]));
        }
    }
    d
}
fn head(d: &mut Definition, parent: usize, s: Shape, h: f64, e: [f64; 3]) {
    let k = h * s.head;
    let j = d.joint(
        "Head".into(),
        Some(parent),
        [0., 0.06 * h, 0.],
        0.,
        [-70., -90., -45.],
        [70., 90., 45.],
        false,
    );
    d.piece(
        j,
        true,
        [0.09 * k * s.face * (1. + 0.12 * e[2]), 0.12 * k, 0.095 * k],
        [0., 0.11 * k, 0.],
        false,
    );
    // Original facial landmarks make direction and jaw movement visible.
    for x in [-1., 1.] {
        d.piece(
            j,
            true,
            [
                0.013 * k,
                0.009 * k * (1. - 0.9 * e[0]) * (1. + 0.6 * e[1]),
                0.012 * k,
            ],
            [x * 0.035 * k * s.face, 0.13 * k, 0.086 * k],
            false,
        );
    }
    d.piece(
        j,
        false,
        [0.012 * k, 0.025 * k, 0.025 * k],
        [0., 0.105 * k, 0.095 * k],
        false,
    );
    let jaw = d.joint(
        "Jaw".into(),
        Some(j),
        [0., 0.05 * k, 0.],
        0.,
        [0.; 3],
        [50., 0., 0.],
        false,
    );
    d.piece(
        jaw,
        true,
        [
            0.07 * k * s.jaw * (1. + 0.15 * e[2]),
            0.04 * k * (1. + 0.3 * e[1]),
            0.075 * k,
        ],
        [0., -0.005 * k - 0.025 * k * e[1], 0.025 * k],
        false,
    );
}
impl Rig {
    pub fn new(kind: Kind) -> Self {
        let mut r = Self {
            version: 1,
            kind,
            shape: Shape::default(),
            angles: vec![],
            mirrored: false,
            controls: vec![],
            expressions: [0.; 3],
        };
        r.angles = vec![[0.; 3]; definition(&r).joints.len()];
        r
    }
    pub fn joints(&self) -> Vec<Joint> {
        definition(self).joints
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.shape.valid()
            && (self.kind == Kind::Hand || !self.mirrored)
            && self
                .expressions
                .into_iter()
                .all(|w| w.is_finite() && (0. ..=1.).contains(&w))
            && (self.kind != Kind::Hand || self.expressions == [0.; 3])
            && crate::model_edit::valid_controls(self)
            && {
                let j = self.joints();
                j.len() == self.angles.len()
                    && j.iter().zip(&self.angles).all(|(j, a)| {
                        (0..3).all(|i| a[i].is_finite() && a[i] >= j.min[i] && a[i] <= j.max[i])
                    })
            }
    }
    pub fn reset_pose(&mut self) {
        self.angles.fill([0.; 3]);
    }
    pub fn mirror(&mut self) {
        let old = self.angles.clone();
        let j = self.joints();
        for (i, b) in j.iter().enumerate() {
            let opposite = if let Some(n) = b.name.strip_prefix("L/") {
                format!("R/{n}")
            } else if let Some(n) = b.name.strip_prefix("R/") {
                format!("L/{n}")
            } else {
                b.name.clone()
            };
            let k = j.iter().position(|b| b.name == opposite).unwrap();
            let a = old[k];
            self.angles[i] = [a[0], -a[1], -a[2]];
        }
        if self.kind == Kind::Hand {
            self.mirrored = !self.mirrored;
        }
    }
    pub fn frames(&self) -> Vec<Matrix> {
        let mut out = Vec::new();
        for (j, a) in self.joints().iter().zip(&self.angles) {
            let m = Matrix::euler(*a).then(j.bind);
            out.push(j.parent.map_or(m, |p| m.then(out[p])));
        }
        out
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.angles.len() * std::mem::size_of::<V>()
            + self.controls.len() * std::mem::size_of::<crate::model_edit::Control>()
    }
}
pub(crate) fn generate(r: &Rig, cancel: &AtomicBool) -> Result<(WorldMesh, Surface), String> {
    use crate::model_library::{Shape, primitive};
    let sphere = primitive(Shape::Sphere, 6, cancel)?.models.remove(0);
    let cube = primitive(Shape::Box, 6, cancel)?.models.remove(0);
    let mut mesh = WorldMesh {
        vertices: vec![],
        edges: vec![],
        color: [115, 153, 198, 255],
    };
    let mut surface = Surface {
        source: format!("Original {:?} mannequin / rigid joint skin v1", r.kind),
        ..Default::default()
    };
    let frames = r.frames();
    for p in definition(r).pieces {
        crate::book_io::check_cancel(cancel)?;
        let source = if p.round { &sphere } else { &cube };
        let m = p.transform.then(frames[p.joint]);
        let offset = mesh.vertices.len();
        mesh.vertices
            .extend(source.mesh.vertices.iter().map(|v| m.map(*v)));
        mesh.edges
            .extend(source.mesh.edges.iter().map(|e| e.map(|i| i + offset)));
        for t in &source.surface.triangles {
            let mut t = t.clone();
            t.v = t.v.map(|i| i + offset as u32);
            if m.determinant() < 0. {
                t.v.swap(1, 2);
            }
            t.n = None;
            t.t = None;
            surface.triangles.push(t);
        }
    }
    Ok((mesh, surface))
}
pub(crate) fn binding_valid(r: &Rig, mesh: &WorldMesh, frame: Matrix) -> bool {
    if !r.valid() {
        return false;
    }
    let Ok((generated, _)) = generate(r, &AtomicBool::new(false)) else {
        return false;
    };
    generated.edges == mesh.edges
        && generated.vertices.len() == mesh.vertices.len()
        && generated.vertices.iter().zip(&mesh.vertices).all(|(a, b)| {
            frame
                .map(*a)
                .into_iter()
                .zip(b)
                .all(|(a, b)| (a - b).abs() <= 1e-6 + 1e-12 * b.abs())
        })
}
pub enum Edit {
    Create { rig: Rig, position: V },
    Replace { id: u64, rig: Rig },
    ReplaceBase { id: u64, rig: Rig },
    Detach { id: u64 },
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    let base_scope = matches!(&edit, Edit::ReplaceBase { .. });
    crate::book_io::check_cancel(cancel)?;
    if let Edit::Replace { id, .. } = &edit
        && crate::pose_sequence::active(&d.scene, *id).is_some()
    {
        return Err(
            "Pose sequence bound: explicitly edit base scene, or edit the selected sequence pose"
                .into(),
        );
    }
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(2 * 1024 * 1024)
        > budget
    {
        return Err("Rig preparation exceeds available memory".into());
    }
    if let Edit::Replace { id, .. } | Edit::ReplaceBase { id, .. } | Edit::Detach { id } = &edit {
        let p = crate::layer_catalog::properties(d, crate::layer_search::Key::World(*id));
        if p.hidden || p.locked || p.position_locked || p.pixels_locked {
            return Err("Mannequin is hidden / locked".into());
        }
        if d.scene
            .file_objects
            .get(crate::layer_search::Key::World(*id))
            .is_some()
        {
            return Err("Detach file-object replacement before posing".into());
        }
    }
    let mut state = d.snapshot();
    let s = &mut state.scene;
    let (id, rig) = match edit {
        Edit::Create { rig, position } => {
            if !rig.valid()
                || position
                    .into_iter()
                    .any(|v| !v.is_finite() || v.abs() > 1e6)
            {
                return Err("Invalid rig / position".into());
            }
            if s.worlds.len() >= 256 || s.world3d.rigs.len() >= 32 {
                return Err("Model / mannequin limit reached".into());
            }
            s.catalog.worlds.initialize(s.worlds.len())?;
            let i = s.worlds.len();
            s.worlds.push(WorldMesh {
                vertices: vec![],
                edges: vec![],
                color: [115, 153, 198, 255],
            });
            s.catalog.worlds.append(s.worlds.len())?;
            let id = s.catalog.worlds.id(i);
            // Fit the neutral default once: shape/pose edits in a new preview
            // must not be cancelled by fitting that edited geometry again.
            let (mesh, _) = generate(&Rig::new(rig.kind), cancel)?;
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for p in mesh.vertices {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
            let longest = (0..3).map(|i| hi[i] - lo[i]).fold(0., f64::max);
            let scale = 2. / longest;
            s.world3d.nodes.insert(
                id,
                world3d::Node {
                    frame: Matrix::scale([scale; 3]).then(Matrix::translation(position)),
                    parent: None,
                },
            );
            s.catalog.world_info.entry(id).or_default().name =
                Some(format!("Original {:?} mannequin", rig.kind));
            let (camera, lens) = world3d::default_camera(d);
            s.camera = Some(camera);
            s.world3d.lens = Some(lens);
            (id, Some(rig))
        }
        Edit::Replace { id, rig } | Edit::ReplaceBase { id, rig } => {
            if !s.world3d.rigs.contains_key(&id) {
                return Err("This model has no rig; joint pose editing is unavailable".into());
            }
            if !rig.valid() || s.world3d.rigs[&id].kind != rig.kind {
                return Err("Invalid joint limits, body shape or skeleton".into());
            }
            (id, Some(rig))
        }
        Edit::Detach { id } => {
            s.world3d.sequences.bindings.retain(|b| b.rig != id);
            if s.world3d.rigs.remove(&id).is_none() {
                return Err("This model has no rig".into());
            }
            (id, None)
        }
    };
    if let Some(rig) = rig {
        let i = s
            .catalog
            .worlds
            .index(id, s.worlds.len())
            .ok_or("Missing rig model")?;
        let flip = s
            .world3d
            .rigs
            .get(&id)
            .is_some_and(|old| old.mirrored != rig.mirrored);
        let (mut mesh, mut surface) = generate(&rig, cancel)?;
        let frame = s.world3d.nodes[&id].frame;
        for v in &mut mesh.vertices {
            *v = frame.map(*v);
        }
        if let Some(old) = s.world3d.surfaces.get(&id) {
            surface.paint = old.paint.clone();
            surface.materials = old.materials.clone();
            surface.texcoords = old.texcoords.clone();
            surface.references = old.references.clone();
            for (t, old) in surface.triangles.iter_mut().zip(&old.triangles) {
                t.material = old.material;
                t.smoothing = old.smoothing;
                t.t = old.t.map(|mut uv| {
                    if flip {
                        uv.swap(1, 2);
                    }
                    uv
                });
            }
            mesh.color = s.worlds[i].color;
        }
        s.worlds[i] = mesh;
        s.world3d.surfaces.insert(id, surface);
        s.world3d.rigs.insert(id, rig);
    }
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Rig result exceeds available memory".into());
    }
    crate::book_io::check_cancel(cancel)?;
    world3d::prepared(
        d,
        state,
        "Mannequin shape / joint pose (rigid skin)".into(),
        vec![id],
        vec![],
    )
    .map(|p| {
        if base_scope {
            p.named("Edit base scene pose / shape; sequence poses unchanged")
        } else {
            p
        }
    })
}
