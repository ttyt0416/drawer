//! Bounded BVH ray queries shared by page rendering, depth and hard shadows.
use crate::{
    document::Document,
    lighting::{Kind, Settings, Style},
    scene::Point,
    world_math::{add, cross, dot, mul, sub, unit},
    world3d::{Matrix, Projection, V},
};
#[derive(Clone, Copy)]
struct Bounds {
    lo: V,
    hi: V,
}
impl Bounds {
    fn of(p: [V; 3]) -> Self {
        Self {
            lo: std::array::from_fn(|i| p.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min)),
            hi: std::array::from_fn(|i| p.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max)),
        }
    }
    fn union(self, b: Self) -> Self {
        Self {
            lo: std::array::from_fn(|i| self.lo[i].min(b.lo[i])),
            hi: std::array::from_fn(|i| self.hi[i].max(b.hi[i])),
        }
    }
    fn hit(self, r: Ray, mut lo: f64, mut hi: f64) -> bool {
        for i in 0..3 {
            if r.d[i].abs() < 1e-15 {
                if r.o[i] < self.lo[i] - 1e-9 || r.o[i] > self.hi[i] + 1e-9 {
                    return false;
                }
            } else {
                let a = (self.lo[i] - r.o[i]) / r.d[i];
                let b = (self.hi[i] - r.o[i]) / r.d[i];
                lo = lo.max(a.min(b) - 1e-9);
                hi = hi.min(a.max(b) + 1e-9);
                if lo > hi {
                    return false;
                }
            }
        }
        true
    }
}
#[derive(Clone, Copy)]
struct Ray {
    o: V,
    d: V,
}
struct Triangle {
    p: [V; 3],
    n: Option<[V; 3]>,
    color: [f64; 4],
    object: usize,
    opaque: bool,
    bounds: Bounds,
}
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<[usize; 2]>,
}
pub(crate) struct Trace {
    triangles: Vec<Triangle>,
    object_bounds: Vec<Option<Bounds>>,
    rotation: Matrix,
    center: Point,
    camera_scale: f64,
    nodes: Vec<Node>,
    settings: Settings,
    camera: crate::scene::Camera,
    lens: crate::world3d::Lens,
}
#[derive(Clone, Copy)]
struct Hit {
    index: usize,
    t: f64,
    u: f64,
    v: f64,
}
fn intersect(t: &Triangle, r: Ray, lo: f64, hi: f64) -> Option<(f64, f64, f64)> {
    let e1 = sub(t.p[1], t.p[0]);
    let e2 = sub(t.p[2], t.p[0]);
    let h = cross(r.d, e2);
    let det = dot(e1, h);
    if det.abs() <= 1e-12 * (dot(e1, e1) * dot(e2, e2) * dot(r.d, r.d)).sqrt() {
        return None;
    }
    let s = sub(r.o, t.p[0]);
    let u = dot(s, h) / det;
    if !(-1e-9..=1. + 1e-9).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(r.d, q) / det;
    if v < -1e-9 || u + v > 1. + 1e-9 {
        return None;
    }
    let depth = dot(e2, q) / det;
    (depth >= lo && depth <= hi).then_some((depth, u, v))
}
impl Trace {
    pub(crate) fn new(d: &Document) -> Option<Self> {
        let settings = d.scene.world3d.lighting.as_ref()?;
        if settings.style == Style::Wire {
            return None;
        }
        Self::build_geometry(d, settings.clone(), false)
    }
    pub(crate) fn surfaces(d: &Document) -> Self {
        Self::build_geometry(d, Settings::default(), true).unwrap()
    }
    fn build_geometry(d: &Document, settings: Settings, physical: bool) -> Option<Self> {
        let (camera, lens) = crate::world3d::default_camera(d);
        let mut t = Self {
            object_bounds: vec![None; d.scene.worlds.len()],
            rotation: Matrix::euler(lens.angles),
            center: std::array::from_fn(|i| {
                (camera.page_bounds.min[i] + camera.page_bounds.max[i]) / 2.
            }),
            camera_scale: if lens.projection == Projection::Perspective {
                (camera.page_bounds.max[1] - camera.page_bounds.min[1])
                    / 2.
                    / (lens.fov.to_radians() / 2.).tan()
            } else {
                camera.pixels_per_unit
            },
            triangles: vec![],
            nodes: vec![],
            settings: settings.clone(),
            camera,
            lens,
        };
        for (object, m) in d.scene.worlds.iter().enumerate() {
            let id = d.scene.catalog.worlds.id(object);
            let key = crate::layer_search::Key::World(id);
            let props = crate::layer_catalog::properties(d, key);
            if props.hidden
                || (!physical && props.opacity == 0)
                || d.scene.file_objects.get(key).is_some()
            {
                continue;
            }
            let Some(s) = d.scene.world3d.surfaces.get(&id) else {
                continue;
            };
            let frame = crate::world3d::node(&d.scene, id).map_or(Matrix::default(), |n| n.frame);
            let inv = frame.inverse();
            for face in &s.triangles {
                let p = face.v.map(|i| m.vertices[i as usize]);
                let area = cross(sub(p[1], p[0]), sub(p[2], p[0]));
                if dot(area, area) < 1e-24 {
                    continue;
                }
                let color = s.materials[face.material as usize].rgba.map(|v| v as f64);
                if !physical && color[3] <= 0. {
                    continue;
                }
                let n = face.n.map(|indices| {
                    indices.map(|i| {
                        let n = s.normals[i as usize];
                        let mapped =
                            std::array::from_fn(|i| (0..3).map(|j| inv.0[j][i] * n[j]).sum());
                        unit(mapped)
                    })
                });
                let bounds = Bounds::of(p);
                t.object_bounds[object] =
                    Some(t.object_bounds[object].map_or(bounds, |b| b.union(bounds)));
                t.triangles.push(Triangle {
                    p,
                    n,
                    color,
                    object,
                    opaque: physical || color[3] >= 1. && props.opacity == 100,
                    bounds: Bounds::of(p),
                });
            }
        }
        if !t.triangles.is_empty() {
            t.build(0, t.triangles.len());
        }
        Some(t)
    }
    pub(crate) fn cast(
        &self,
        origin: V,
        direction: V,
        maximum: f64,
        objects: &std::collections::BTreeSet<usize>,
    ) -> Option<V> {
        let ray = Ray {
            o: origin,
            d: direction,
        };
        self.nearest(ray, 0., maximum, |t| objects.contains(&t.object))
            .map(|h| add(origin, mul(direction, h.t)))
    }
    fn build(&mut self, start: usize, end: usize) -> usize {
        let bounds = self.triangles[start..end]
            .iter()
            .map(|t| t.bounds)
            .reduce(Bounds::union)
            .unwrap();
        let id = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let axis = (0..3)
                .max_by(|a, b| {
                    (bounds.hi[*a] - bounds.lo[*a]).total_cmp(&(bounds.hi[*b] - bounds.lo[*b]))
                })
                .unwrap();
            self.triangles[start..end].sort_unstable_by(|a, b| {
                (a.bounds.lo[axis] + a.bounds.hi[axis])
                    .total_cmp(&(b.bounds.lo[axis] + b.bounds.hi[axis]))
            });
            let mid = (start + end) / 2;
            let a = self.build(start, mid);
            let b = self.build(mid, end);
            self.nodes[id].children = Some([a, b]);
        }
        id
    }
    fn nearest(
        &self,
        ray: Ray,
        lo: f64,
        hi: f64,
        accept: impl Fn(&Triangle) -> bool,
    ) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best = None;
        let mut limit = hi;
        let mut stack = [0usize; 64];
        let mut count = 1;
        while count > 0 {
            count -= 1;
            let node = &self.nodes[stack[count]];
            if !node.bounds.hit(ray, lo, limit) {
                continue;
            }
            if let Some(children) = node.children {
                for child in children {
                    stack[count] = child;
                    count += 1;
                }
            } else {
                for i in node.start..node.end {
                    let t = &self.triangles[i];
                    if accept(t)
                        && let Some((depth, u, v)) = intersect(t, ray, lo, limit)
                    {
                        limit = depth;
                        best = Some(Hit {
                            index: i,
                            t: depth,
                            u,
                            v,
                        });
                    }
                }
            }
        }
        best
    }
    fn ray(&self, p: Point) -> Ray {
        let x = (p[0] - self.center[0]) / self.camera_scale;
        let y = -(p[1] - self.center[1]) / self.camera_scale;
        match self.lens.projection {
            Projection::Perspective => Ray {
                o: self.camera.position,
                d: self.rotation.vector([x, y, -1.]),
            },
            Projection::Orthographic => Ray {
                o: add(self.camera.position, self.rotation.vector([x, y, 0.])),
                d: self.rotation.vector([0., 0., -1.]),
            },
        }
    }
    pub(crate) fn pixel(&self, object: usize, p: Point) -> [u8; 4] {
        let ray = self.ray(p);
        if self
            .object_bounds
            .get(object)
            .and_then(|b| *b)
            .is_none_or(|b| !b.hit(ray, self.lens.near, self.lens.far))
        {
            return [0; 4];
        }
        let Some(h) = self.nearest(ray, self.lens.near, self.lens.far, |t| t.object == object)
        else {
            return [0; 4];
        };
        if self
            .nearest(ray, self.lens.near, h.t - 1e-7, |t| {
                t.object != object && t.opaque
            })
            .is_some()
        {
            return [0; 4];
        }
        self.shade(ray, h)
    }
    fn shade(&self, r: Ray, h: Hit) -> [u8; 4] {
        let t = &self.triangles[h.index];
        let p = add(r.o, mul(r.d, h.t));
        let mut n = unit(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0])));
        if matches!(self.settings.style, Style::Smooth | Style::Toon)
            && let Some(normals) = t.n
        {
            let normal = add(
                add(mul(normals[0], 1. - h.u - h.v), mul(normals[1], h.u)),
                mul(normals[2], h.v),
            );
            if dot(normal, normal) > 1e-20 {
                n = unit(normal);
            }
        }
        if dot(n, r.d) > 0. {
            n = mul(n, -1.);
        }
        let mut light = [self.settings.ambient; 3];
        if self.settings.style == Style::Unlit {
            light = [1.; 3];
        } else {
            for lamp in &self.settings.lights {
                if lamp.intensity == 0. {
                    continue;
                }
                let (dir, maximum, attenuation) = match lamp.kind {
                    Kind::Directional { direction } => (mul(unit(direction), -1.), 1e12, 1.),
                    Kind::Point { position } | Kind::Spot { position, .. } => {
                        let delta = sub(position, p);
                        let distance = dot(delta, delta).sqrt();
                        if distance < 1e-9 {
                            continue;
                        }
                        let mut attenuation = 1. / (1. + distance * distance);
                        if let Kind::Spot {
                            direction, angle, ..
                        } = lamp.kind
                        {
                            let cosine = dot(mul(unit(delta), -1.), unit(direction));
                            let cutoff = angle.to_radians().cos();
                            attenuation *= ((cosine - cutoff) / (1. - cutoff)).clamp(0., 1.);
                        }
                        (mul(delta, 1. / distance), distance, attenuation)
                    }
                };
                let mut cosine = dot(n, dir).max(0.);
                if cosine == 0. || attenuation == 0. {
                    continue;
                }
                if self.settings.shadows && lamp.shadow {
                    let epsilon = 1e-5 + p.into_iter().map(f64::abs).fold(0., f64::max) * 1e-12;
                    let shadow = Ray {
                        o: add(p, mul(n, epsilon)),
                        d: dir,
                    };
                    if self
                        .nearest(shadow, epsilon, (maximum - epsilon).max(epsilon), |t| {
                            t.opaque
                        })
                        .is_some()
                    {
                        continue;
                    }
                }
                if self.settings.style == Style::Toon {
                    let steps = (self.settings.bands - 1) as f64;
                    cosine = (cosine * steps).round() / steps;
                }
                for (i, v) in light.iter_mut().enumerate() {
                    *v += cosine * attenuation * lamp.intensity * lamp.color[i];
                }
            }
        }
        let linear = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let encode = |v: f64| {
            let v = v.max(0.);
            let s = if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            };
            (s.clamp(0., 1.) * 255.).round() as u8
        };
        [
            encode(linear(t.color[0]) * light[0]),
            encode(linear(t.color[1]) * light[1]),
            encode(linear(t.color[2]) * light[2]),
            (t.color[3] * 255.).round() as u8,
        ]
    }
    #[cfg(test)]
    pub(crate) fn compare_queries(&self) -> bool {
        for y in 0..20 {
            for x in 0..20 {
                let ray = self.ray([x as f64 * 3.2 + 0.1, y as f64 * 3.2 + 0.2]);
                let fast = self.nearest(ray, self.lens.near, self.lens.far, |_| true);
                let slow = self
                    .triangles
                    .iter()
                    .filter_map(|t| intersect(t, ray, self.lens.near, self.lens.far))
                    .map(|h| h.0)
                    .min_by(f64::total_cmp);
                if fast
                    .map(|h| h.t)
                    .zip(slow)
                    .is_some_and(|(a, b)| (a - b).abs() > 1e-7)
                    || fast.is_some() != slow.is_some()
                {
                    return false;
                }
            }
        }
        true
    }
}
