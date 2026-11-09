//! Original bounded mannequin deformation and explicit pose assistance.
use crate::{
    document::Document,
    human_rig::{self, Kind, Rig, Shape},
    world_math::{add, cross, dot, mul, sub, unit},
    world3d::{self, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    pub parts: Vec<V>,
    /// Local direction toward the viewer. Frozen; moving the camera never edits geometry.
    pub toward: V,
    pub strength: f64,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            parts: vec![],
            toward: [0., 0., 1.],
            strength: 0.,
        }
    }
}
impl Form {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self, count: usize) -> bool {
        (self.parts.is_empty() || self.parts.len() == count)
            && self
                .parts
                .iter()
                .flatten()
                .all(|x| x.is_finite() && (0.5..=2.).contains(x))
            && self.toward.iter().all(|x| x.is_finite())
            && (dot(self.toward, self.toward) - 1.).abs() < 1e-9
            && self.strength.is_finite()
            && (0. ..=2.).contains(&self.strength)
    }
    pub fn point(&self, p: V, height: f64) -> V {
        if self.strength == 0. {
            return p;
        }
        let depth = dot(p, self.toward);
        let scale = (self.strength * depth / height)
            .clamp(-4f64.ln(), 4f64.ln())
            .exp();
        add(
            mul(sub(p, mul(self.toward, depth)), scale),
            mul(self.toward, depth),
        )
    }
    pub fn frame(&self, m: Matrix, height: f64) -> Matrix {
        if self.strength == 0. {
            return m;
        }
        let p = m.origin();
        let q = self.point(p, height);
        let t = dot(p, self.toward) * self.strength / height;
        let scale = t.clamp(-4f64.ln(), 4f64.ln()).exp();
        let mut out = Matrix::translation(q);
        for i in 0..3 {
            let a = m.0.map(|r| r[i]);
            let depth = dot(a, self.toward);
            let derivative = if t.abs() < 4f64.ln() {
                scale * self.strength / height * depth
            } else {
                0.
            };
            let b = add(
                add(
                    mul(sub(a, mul(self.toward, depth)), scale),
                    mul(self.toward, depth),
                ),
                mul(sub(p, mul(self.toward, dot(p, self.toward))), derivative),
            );
            for (j, value) in b.into_iter().enumerate() {
                out.0[j][i] = value;
            }
        }
        out
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Body,
    Head,
    Hand,
}
pub fn blend(source: Shape, target: Shape, weight: f64, region: Region) -> Result<Shape, String> {
    if !source.valid() || !target.valid() || !weight.is_finite() || !(0. ..=1.).contains(&weight) {
        return Err("Blend requires valid shapes and weight0..1".into());
    }
    let mut s = source;
    let mix = |a: f64, b: f64| {
        if weight == 0. {
            a
        } else if weight == 1. {
            b
        } else {
            a + (b - a) * weight
        }
    };
    if region == Region::Body {
        s.height = mix(s.height, target.height);
        s.shoulders = mix(s.shoulders, target.shoulders);
        s.hips = mix(s.hips, target.hips);
        s.bulk = mix(s.bulk, target.bulk);
    }
    if region != Region::Hand {
        s.head = mix(s.head, target.head);
        s.face = mix(s.face, target.face);
        s.jaw = mix(s.jaw, target.jaw);
    }
    if region != Region::Head {
        s.hands = mix(s.hands, target.hands);
        s.palm = mix(s.palm, target.palm);
        s.fingers = mix(s.fingers, target.fingers);
    }
    Ok(s)
}
#[derive(Clone, Debug)]
pub enum Change {
    Form(Form),
    Blend {
        target: Shape,
        weight: f64,
        region: Region,
    },
    Assist {
        collision: bool,
        upright: bool,
        ground: Option<f64>,
    },
}
#[derive(Clone, Copy)]
struct Capsule {
    a: V,
    b: V,
    r: f64,
    group: usize,
    upper_leg: bool,
}
fn segment(a: V, b: V, c: V, d: V) -> f64 {
    let u = sub(b, a);
    let v = sub(d, c);
    let w = sub(a, c);
    let aa = dot(u, u);
    let bb = dot(u, v);
    let cc = dot(v, v);
    let dd = dot(u, w);
    let ee = dot(v, w);
    let den = aa * cc - bb * bb;
    let mut s = if den > 1e-20 {
        ((bb * ee - cc * dd) / den).clamp(0., 1.)
    } else {
        0.
    };
    let t = if cc > 1e-20 {
        ((bb * s + ee) / cc).clamp(0., 1.)
    } else {
        0.
    };
    if aa > 1e-20 {
        s = ((bb * t - dd) / aa).clamp(0., 1.);
    }
    let delta = sub(add(a, mul(u, s)), add(c, mul(v, t)));
    dot(delta, delta).sqrt()
}
/// Approximate limb capsules, independent of mesh triangle count. Adjacent links are excluded.
pub fn penetration(r: &Rig) -> Result<f64, String> {
    if r.kind != Kind::Body || !r.valid() {
        return Err("Limb assistance requires a valid original Body".into());
    }
    let j = r.joints();
    let f = r.frames();
    let index = |name: &str| {
        j.iter()
            .position(|j| j.name == name)
            .ok_or("Missing original bone")
    };
    let mut c = vec![];
    for (group, prefix) in [(1, "R/"), (2, "L/"), (3, "R/"), (4, "L/")] {
        let names = if group <= 2 {
            [("UpperArm", "Elbow", 0.045), ("Elbow", "Wrist", 0.035)]
        } else {
            [("Hip", "Knee", 0.07), ("Knee", "Ankle", 0.05)]
        };
        for (part, (a, b, radius)) in names.into_iter().enumerate() {
            let i = index(&format!("{prefix}{a}"))?;
            let k = index(&format!("{prefix}{b}"))?;
            let a = f[i].origin();
            let b = f[k].origin();
            let v = sub(b, a);
            let radius = radius * r.shape.height / 1.8 * r.shape.bulk;
            let stretch = (0..3)
                .map(|axis| {
                    let v = f[i].0.map(|row| row[axis]);
                    dot(v, v).sqrt()
                })
                .fold(0., f64::max);
            c.push(Capsule {
                a: add(a, mul(v, 0.18)),
                b: add(a, mul(v, 0.82)),
                r: radius * stretch,
                group,
                upper_leg: group >= 3 && part == 0,
            });
        }
    }
    c.push(Capsule {
        a: f[0].origin(),
        b: f[index("Chest")?].origin(),
        r: 0.09 * r.shape.bulk * r.shape.height / 1.8,
        group: 0,
        upper_leg: false,
    });
    let mut score = 0.;
    for (i, a) in c.iter().enumerate() {
        for b in &c[i + 1..] {
            if a.group == b.group || (a.group == 0 && b.upper_leg) || (b.group == 0 && a.upper_leg)
            {
                continue;
            }
            let overlap = (a.r + b.r - segment(a.a, a.b, b.a, b.b)).max(0.);
            score += overlap * overlap;
        }
    }
    Ok(score)
}
pub fn separate(r: &Rig, cancel: &AtomicBool) -> Result<Rig, String> {
    crate::book_io::check_cancel(cancel)?;
    let mut out = r.clone();
    let mut score = penetration(&out)?;
    if score <= 1e-10 {
        return Ok(out);
    }
    let controls = crate::model_edit::controls(r);
    let joints = r.joints();
    for step in [30., 15., 5., 1.] {
        for _ in 0..3 {
            let mut improved = false;
            for (i, j) in joints.iter().enumerate() {
                if !["Shoulder", "UpperArm", "Elbow", "Hip", "Knee"]
                    .iter()
                    .any(|name| j.name.ends_with(name))
                {
                    continue;
                }
                for axis in 0..3 {
                    crate::book_io::check_cancel(cancel)?;
                    if controls[i].locked[axis] {
                        continue;
                    }
                    let old = out.angles[i][axis];
                    let mut best = old;
                    for delta in [-step, step] {
                        let value =
                            (old + delta).clamp(controls[i].min[axis], controls[i].max[axis]);
                        out.angles[i][axis] = value;
                        let next = penetration(&out)?;
                        if next + 1e-14 < score {
                            score = next;
                            best = value;
                            improved = true;
                        }
                    }
                    out.angles[i][axis] = best;
                }
            }
            if score <= 1e-10 {
                return Ok(out);
            }
            if !improved {
                break;
            }
        }
    }
    Err(format!(
        "Limb capsules remain intersecting ({:.6}); original pose retained. Unlock joints or adjust shape/pose and retry",
        score.sqrt()
    ))
}
pub fn prepare(
    d: &Document,
    id: u64,
    change: Change,
    base: bool,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Mannequin assist exceeds memory budget".into());
    }
    let mut r = d
        .scene
        .world3d
        .rigs
        .get(&id)
        .cloned()
        .ok_or("Choose an original mannequin")?;
    let mut ground = None;
    let mut upright = false;
    match change {
        Change::Form(form) => r.form = form,
        Change::Blend {
            target,
            weight,
            region,
        } => {
            if (r.kind == Kind::Head && region != Region::Head)
                || (r.kind == Kind::Hand && region != Region::Hand)
            {
                return Err("Choose a region present on the mannequin".into());
            }
            r.shape = blend(r.shape, target, weight, region)?;
        }
        Change::Assist {
            collision,
            upright: stand,
            ground: floor,
        } => {
            if r.kind != Kind::Body {
                return Err("Ground, upright and limb assistance require original Body".into());
            }
            if !collision && !stand && floor.is_none() {
                return Err("Choose an explicit assistance operation".into());
            }
            if floor.is_some_and(|v| !v.is_finite() || v.abs() > 1e6) {
                return Err("Invalid world floor height".into());
            }
            ground = floor;
            upright = stand;
            if stand {
                let controls = crate::model_edit::controls(&r);
                for (i, j) in r.joints().iter().enumerate() {
                    if !j.name.contains('/')
                        || ["Hip", "Knee", "Ankle", "Toe"]
                            .iter()
                            .any(|n| j.name.ends_with(n))
                    {
                        for axis in 0..3 {
                            if !controls[i].locked[axis] {
                                r.angles[i][axis] =
                                    0f64.clamp(controls[i].min[axis], controls[i].max[axis]);
                            }
                        }
                    }
                }
            }
            if collision {
                r = separate(&r, cancel)?;
            }
        }
    }
    if !r.valid() {
        return Err("Invalid mannequin proportions or perspective".into());
    }
    let inner = human_rig::prepare(
        d,
        if base {
            human_rig::Edit::ReplaceBase { id, rig: r.clone() }
        } else {
            human_rig::Edit::Replace { id, rig: r.clone() }
        },
        budget,
        cancel,
    )?;
    let mut state = d.snapshot();
    state.scene = inner.scene().clone();
    let ids = crate::model_library::tree(&state.scene, id);
    if upright || ground.is_some() {
        crate::model_library::editable(d, &ids)?;
    }
    if upright {
        let frame = world3d::node(&state.scene, id)
            .ok_or("Missing model frame")?
            .frame;
        let y = frame.axis(1);
        let up = [0., 1., 0.];
        let angle = dot(y, up).clamp(-1., 1.).acos().to_degrees();
        if angle > 1e-9 {
            let axis = cross(y, up);
            let axis = if dot(axis, axis) < 1e-18 {
                [1., 0., 0.]
            } else {
                unit(axis)
            };
            let delta = Matrix::rotation(axis, angle).around(frame.origin());
            for child in &ids {
                let i = state
                    .scene
                    .catalog
                    .worlds
                    .index(*child, state.scene.worlds.len())
                    .ok_or("Missing child")?;
                for v in &mut state.scene.worlds[i].vertices {
                    *v = delta.map(*v);
                }
                let n = state
                    .scene
                    .world3d
                    .nodes
                    .get_mut(child)
                    .ok_or("Missing model axes")?;
                n.frame = n.frame.then(delta);
            }
        }
    }
    if let Some(floor) = ground {
        let frame = world3d::node(&state.scene, id)
            .ok_or("Missing model frame")?
            .frame;
        let min = human_rig::feet(&r, cancel)?
            .into_iter()
            .map(|v| frame.map(v)[1])
            .fold(f64::INFINITY, f64::min);
        if !min.is_finite() {
            return Err("Missing foot contact geometry".into());
        }
        crate::model_library::shift(&mut state.scene, &ids, [0., floor - min, 0.])?;
    }
    crate::model_edit::sync(d, &mut state.scene)?;
    let preview = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&preview).map_err(|e| e.to_string())?;
    crate::book_io::check_cancel(cancel)?;
    world3d::prepared(
        d,
        state,
        if base {
            "Base mannequin form / explicit assistance; clip poses unchanged"
        } else {
            "Mannequin form / explicit pose assistance"
        }
        .into(),
        ids.into_iter().collect(),
        vec![],
    )
}
