//! Local scene rendering policy, independent of camera projection and environment sources.
use crate::{
    scene::{Camera, Point},
    world_math::{add, cross, dot, mul, sub, unit},
    world3d::{Lens, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowTarget {
    Off,
    Models,
    Ground,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowQuality {
    Low,
    Medium,
    High,
}
impl ShadowQuality {
    pub fn resolution(self) -> u32 {
        match self {
            Self::Low => 256,
            Self::Medium => 512,
            Self::High => 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditQuality {
    Normal,
    Fast,
    HighSpeed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FogKind {
    Depth,
    Distance,
    HeightBelow,
    HeightAbove,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    pub kind: FogKind,
    pub start: f64,
    pub end: f64,
    pub density: f64,
    pub color: V,
}
impl Default for Fog {
    fn default() -> Self {
        Self {
            kind: FogKind::Depth,
            start: 2.,
            end: 10.,
            density: 1.,
            color: [0.7, 0.8, 0.9],
        }
    }
}
impl Fog {
    pub fn valid(&self) -> bool {
        self.start.is_finite()
            && self.end.is_finite()
            && self.start.abs() <= 1e6
            && self.end.abs() <= 1e6
            && self.end > self.start
            && self.density.is_finite()
            && (0. ..=64.).contains(&self.density)
            && self
                .color
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
    }
    pub fn coverage(&self, p: V, c: &Camera, l: Lens) -> f64 {
        let delta = sub(p, c.position);
        let v = match self.kind {
            FogKind::Depth => -Matrix::euler(l.angles).inverse().vector(delta)[2],
            FogKind::Distance => dot(delta, delta).sqrt(),
            FogKind::HeightBelow => self.start + self.end - p[1],
            FogKind::HeightAbove => p[1],
        };
        1. - (-self.density * ((v - self.start) / (self.end - self.start)).clamp(0., 1.)).exp()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Billboard {
    Off,
    AllAxes,
    OnlyHorizontal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alpha {
    Source,
    Opaque,
    Threshold,
    Blend,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub cast: bool,
    pub receive: bool,
    pub ground: bool,
    pub billboard: Billboard,
    pub alpha: Alpha,
    pub threshold: f64,
    pub alpha_scale: f64,
}
impl Default for Object {
    fn default() -> Self {
        Self {
            cast: true,
            receive: true,
            ground: false,
            billboard: Billboard::Off,
            alpha: Alpha::Source,
            threshold: 0.5,
            alpha_scale: 1.,
        }
    }
}
impl Object {
    pub fn valid(&self) -> bool {
        self.alpha_scale.is_finite()
            && (0. ..=1.).contains(&self.alpha_scale)
            && self.threshold.is_finite()
            && (0. ..=1.).contains(&self.threshold)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightSource {
    Current,
    SingleColor,
    Files,
    Material,
    Panorama,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub source: LightSource,
    pub tint: V,
    pub target: ShadowTarget,
    pub shadow: ShadowQuality,
    pub quality: EditQuality,
    pub fog: Option<Fog>,
    pub outline: u8,
    pub line: [u8; 4],
    pub objects: BTreeMap<u64, Object>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source: LightSource::Current,
            tint: [1.; 3],
            target: ShadowTarget::Both,
            shadow: ShadowQuality::High,
            quality: EditQuality::Normal,
            fog: None,
            outline: 0,
            line: [0, 0, 0, 255],
            objects: Default::default(),
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.tint
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.outline <= 4
            && self.objects.len() <= 256
            && self.objects.iter().all(|(id, o)| *id > 0 && o.valid())
            && self.fog.as_ref().is_none_or(Fog::valid)
    }
    pub fn receives(&self, o: &Object) -> bool {
        o.receive
            && match self.target {
                ShadowTarget::Off => false,
                ShadowTarget::Both => true,
                ShadowTarget::Models => !o.ground,
                ShadowTarget::Ground => o.ground,
            }
    }
    pub fn object(&self, id: u64) -> Object {
        self.objects.get(&id).cloned().unwrap_or_default()
    }
}
pub fn billboard(frame: Matrix, mode: Billboard, c: &Camera, l: Lens) -> Matrix {
    if mode == Billboard::Off {
        return frame;
    }
    let z = match mode {
        Billboard::AllAxes => Matrix::euler(l.angles).vector([0., 0., 1.]),
        Billboard::OnlyHorizontal => {
            let v = sub(c.position, frame.origin());
            if v[0].hypot(v[2]) < 1e-9 {
                let v = Matrix::euler(l.angles).vector([0., 0., 1.]);
                if v[0].hypot(v[2]) < 1e-9 {
                    [0., 0., 1.]
                } else {
                    unit([v[0], 0., v[2]])
                }
            } else {
                unit([v[0], 0., v[2]])
            }
        }
        Billboard::Off => unreachable!(),
    };
    let up = if mode == Billboard::AllAxes {
        Matrix::euler(l.angles).vector([0., 1., 0.])
    } else {
        [0., 1., 0.]
    };
    let x = unit(cross(up, z));
    let y = unit(cross(z, x));
    let mut rotation = Matrix::default();
    for (i, v) in [x, y, z].into_iter().enumerate() {
        for (j, value) in v.into_iter().enumerate() {
            rotation.0[j][i] = value;
        }
    }
    let old_x = frame.axis(0);
    let mut old_y = sub(frame.axis(1), mul(old_x, dot(frame.axis(1), old_x)));
    if dot(old_y, old_y) < 1e-24 {
        old_y = cross(frame.axis(2), old_x);
    }
    if dot(old_y, old_y) < 1e-24 {
        old_y = cross(
            if old_x[1].abs() < 0.9 {
                [0., 1., 0.]
            } else {
                [0., 0., 1.]
            },
            old_x,
        );
    }
    let old_y = unit(old_y);
    let old_z = cross(old_x, old_y);
    let mut old = Matrix::default();
    for (i, v) in [old_x, old_y, old_z].into_iter().enumerate() {
        for (j, value) in v.into_iter().enumerate() {
            old.0[j][i] = value;
        }
    }
    let mut out = frame.then(old.inverse().then(rotation).around(frame.origin()));
    for (i, row) in out.0.iter_mut().enumerate() {
        row[3] = frame.0[i][3];
    }
    out
}
/// Quantize a hard-shadow sample in a light-aligned texel grid. No preview-only randomness.
pub fn shadow_sample(p: V, dir: V, span: f64, resolution: u32) -> V {
    let z = unit(dir);
    let up = if z[1].abs() < 0.9 {
        [0., 1., 0.]
    } else {
        [1., 0., 0.]
    };
    let x = unit(cross(up, z));
    let y = cross(z, x);
    let step = span.max(1e-5) / resolution as f64;
    add(
        p,
        add(
            mul(x, ((dot(p, x) / step).round() * step) - dot(p, x)),
            mul(y, ((dot(p, y) / step).round() * step) - dot(p, y)),
        ),
    )
}
pub fn edit_point(d: &crate::document::Document, p: Point) -> Point {
    let quality = d
        .scene
        .world3d
        .lighting
        .as_ref()
        .and_then(|l| l.render.as_ref())
        .map(|r| r.quality);
    let step = match quality {
        Some(EditQuality::Fast) => 2.,
        Some(EditQuality::HighSpeed) => 4.,
        _ => 1.,
    };
    if step == 1. {
        p
    } else {
        p.map(|v| (v / step).floor() * step + step / 2.)
    }
}
pub fn viewport_reduction(d: &crate::document::Document) -> u32 {
    if d.scene
        .world3d
        .lighting
        .as_ref()
        .and_then(|l| l.render.as_ref())
        .is_some_and(|r| r.quality == EditQuality::HighSpeed)
    {
        4
    } else {
        1
    }
}
