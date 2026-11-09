//! Local scene lighting and overlay-only camera axis guides.
use crate::{
    document::Document,
    scene::{Point, Scene},
    world_math::dot,
    world3d::{self, Matrix, V},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    Wire,
    Unlit,
    Flat,
    Smooth,
    Toon,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Directional {
        direction: V,
    },
    Point {
        position: V,
    },
    Spot {
        position: V,
        direction: V,
        angle: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    pub kind: Kind,
    pub color: V,
    pub intensity: f64,
    pub shadow: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follow_camera: bool,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            kind: Kind::Directional {
                direction: [-0.5, -1., -1.],
            },
            color: [1.; 3],
            intensity: 1.,
            shadow: true,
            follow_camera: false,
        }
    }
}
impl Light {
    pub fn valid(&self) -> bool {
        let position = |v: V| v.into_iter().all(|v| v.is_finite() && v.abs() <= 1e6);
        let direction = |v: V| position(v) && dot(v, v) > 1e-12;
        self.color
            .into_iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(&v))
            && self.intensity.is_finite()
            && (0. ..=64.).contains(&self.intensity)
            && match self.kind {
                Kind::Directional { direction: v } => direction(v),
                Kind::Point { position: p } => position(p),
                Kind::Spot {
                    position: p,
                    direction: v,
                    angle,
                } => {
                    position(p) && direction(v) && angle.is_finite() && (1. ..=89.).contains(&angle)
                }
            }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub style: Style,
    pub lights: Vec<Light>,
    pub ambient: f64,
    pub shadows: bool,
    pub bands: u8,
    pub guides: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<crate::scene_render::Settings>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            style: Style::Smooth,
            lights: vec![Light::default()],
            ambient: 0.1,
            shadows: true,
            bands: 3,
            guides: false,
            render: None,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self
                .render
                .as_ref()
                .is_none_or(crate::scene_render::Settings::valid)
            && self.lights.len() <= 8
            && self.lights.iter().all(Light::valid)
            && self.ambient.is_finite()
            && (0. ..=4.).contains(&self.ambient)
            && (2..=8).contains(&self.bands)
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.lights.len() * std::mem::size_of::<Light>()
            + self
                .render
                .as_ref()
                .map_or(0, |r| std::mem::size_of_val(r) + r.objects.len() * 128)
    }
}
pub struct Prepared {
    inner: world3d::Prepared,
    frame: u32,
}
impl Prepared {
    pub fn scene(&self) -> &Scene {
        self.inner.scene()
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if d.current_frame != self.frame {
            return Err("Lighting preview frame changed".into());
        }
        self.inner.commit(d)
    }
}
pub fn prepare(
    d: &Document,
    settings: Settings,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if !settings.valid() {
        return Err("Invalid lights / rendering settings".into());
    }
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if triangles > 262144
        || d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(triangles.saturating_mul(512))
            .saturating_add(1024 * 1024)
            > budget
    {
        return Err("Lighting scene / acceleration memory budget exceeded".into());
    }
    let mut s = d.snapshot();
    if let Some(r) = &settings.render
        && matches!(
            r.source,
            crate::scene_render::LightSource::Files
                | crate::scene_render::LightSource::Material
                | crate::scene_render::LightSource::Panorama
        )
    {
        s.scene.world3d.environment.as_mut().ok_or("Load/embed a local environment file or material panorama in scene-environment first")?.settings.illumination=true;
    }
    s.scene.world3d.lighting = Some(settings);
    let (c, mut l) = world3d::default_camera(d);
    if d.scene.camera.is_some() && d.scene.world3d.lens.is_none() {
        l.projection = world3d::Projection::Orthographic;
    }
    s.scene.camera = Some(c);
    s.scene.world3d.lens = Some(l);
    let frozen = crate::layer_merge::preview(&s, d.current_frame)?;
    crate::files::native_scene(&frozen).map_err(|e| e.to_string())?;
    crate::book_io::check_cancel(cancel)?;
    world3d::prepared(
        d,
        s,
        "Scene lighting / rendering and overlay-only vanishing guides".into(),
        vec![],
        vec![],
    )
    .map(|inner| Prepared {
        inner,
        frame: d.current_frame,
    })
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vanishing {
    Finite(Point),
    Parallel(Point),
}
pub fn vanishing(s: &Scene) -> Option<[Vanishing; 3]> {
    let c = s.camera.as_ref()?;
    let l = s.world3d.lens?;
    let inverse = Matrix::euler(l.angles).inverse();
    let f = (c.page_bounds.max[1] - c.page_bounds.min[1]) / 2. / (l.fov.to_radians() / 2.).tan();
    let center = l.screen_center(c);
    Some(std::array::from_fn(|axis| {
        let v = inverse.vector(Matrix::default().axis(axis));
        if l.projection == world3d::Projection::Orthographic || v[2].abs() < 1e-8 {
            let length = v[0].hypot(v[1]);
            Vanishing::Parallel(if length > 1e-12 {
                [v[0] / length, -v[1] / length]
            } else {
                [0.; 2]
            })
        } else {
            Vanishing::Finite([center[0] + v[0] * f / -v[2], center[1] - v[1] * f / -v[2]])
        }
    }))
}
fn clipped(a: Point, b: Point, r: crate::scene::Rect) -> Option<[Point; 2]> {
    let delta = std::array::from_fn::<_, 2, _>(|i| b[i] - a[i]);
    let (mut lo, mut hi) = (0f64, 1f64);
    for i in 0..2 {
        if delta[i].abs() < 1e-15 {
            if a[i] < r.min[i] || a[i] > r.max[i] {
                return None;
            }
        } else {
            let p = (r.min[i] - a[i]) / delta[i];
            let q = (r.max[i] - a[i]) / delta[i];
            lo = lo.max(p.min(q));
            hi = hi.min(p.max(q));
            if lo > hi {
                return None;
            }
        }
    }
    Some([
        std::array::from_fn(|i| a[i] + delta[i] * lo),
        std::array::from_fn(|i| a[i] + delta[i] * hi),
    ])
}

pub fn guide_lines(s: &Scene) -> Vec<([Point; 2], usize)> {
    if s.world3d.lighting.as_ref().is_none_or(|s| !s.guides) {
        return vec![];
    }
    let Some(v) = vanishing(s) else {
        return vec![];
    };
    let r = s.camera.as_ref().unwrap().page_bounds;
    let center = std::array::from_fn::<_, 2, _>(|i| (r.min[i] + r.max[i]) / 2.);
    let extent = (r.max[0] - r.min[0]).hypot(r.max[1] - r.min[1]) * 4.;
    let mut lines = vec![];
    for (axis, v) in v.into_iter().enumerate() {
        for anchor in [r.min, r.max, center] {
            let (a, b) = match v {
                Vanishing::Finite(p) => (anchor, p),
                Vanishing::Parallel(d) => {
                    if d == [0.; 2] {
                        continue;
                    }
                    (
                        [anchor[0] - d[0] * extent, anchor[1] - d[1] * extent],
                        [anchor[0] + d[0] * extent, anchor[1] + d[1] * extent],
                    )
                }
            };
            if let Some(line) = clipped(a, b, r) {
                lines.push((line, axis));
            }
        }
    }
    lines
}
