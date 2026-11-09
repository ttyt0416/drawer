//! Bounded BVH ray queries shared by page rendering, depth and hard shadows.
use crate::{
    document::Document,
    lighting::{Kind, Settings, Style},
    scene::Point,
    world_math::{add, cross, dot, mul, sub, unit},
    world3d::{Matrix, V},
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
    uv: Option<[V; 3]>,
    face: usize,
    opaque: bool,
    material: u16,
    bounds: Bounds,
    visible: bool,
}
struct ImportedMaterial {
    pixels: Option<(usize, usize, Vec<u8>)>,
    wrap: [crate::model_import::Wrap; 2],
    flip_v: bool,
    alpha: crate::model_import::Alpha,
    cutoff: f64,
    uv: crate::surface_edit::Uv,
    normal: Option<(usize, usize, Vec<u8>)>,
    strength: f64,
    flip_green: bool,
}
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<[usize; 2]>,
}
struct FragmentCache {
    point: Point,
    objects: Vec<usize>,
    pixels: Vec<(usize, [u8; 4])>,
}
pub(crate) struct Trace {
    cache: std::sync::Mutex<Option<FragmentCache>>,
    surface_tools: bool,
    wires: Vec<Option<crate::render::Wires>>,
    wire_colors: Vec<[u8; 4]>,
    triangles: Vec<Triangle>,
    textures: Vec<Option<(usize, Vec<u8>)>>,
    imported: Vec<std::collections::BTreeMap<u16, ImportedMaterial>>,
    physical: bool,
    environment: Option<crate::environment::Light>,
    object_bounds: Vec<Option<Bounds>>,
    rays: crate::world3d::Rays,
    nodes: Vec<Node>,
    settings: Settings,
    lens: crate::world3d::Lens,
    camera: crate::scene::Camera,
    policies: Vec<crate::scene_render::Object>,
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
        if settings.style == Style::Wire
            && !d.scene.world3d.surfaces.values().any(|s| s.tools.active())
            && !settings.render.as_ref().is_some_and(|r| {
                r.objects
                    .values()
                    .any(|o| o.billboard != crate::scene_render::Billboard::Off)
            })
        {
            return None;
        }
        let mut trace = Self::build_geometry(d, settings.clone(), false)?;
        trace.environment = if settings
            .render
            .as_ref()
            .is_some_and(|r| r.source == crate::scene_render::LightSource::SingleColor)
        {
            None
        } else {
            d.scene
                .world3d
                .environment
                .as_ref()
                .filter(|e| e.settings.illumination && e.settings.strength > 0.)
                .and_then(|e| crate::environment::Sampler::new(e).ok())
                .map(|s| s.light())
        };
        Some(trace)
    }
    pub(crate) fn surfaces(d: &Document) -> Self {
        Self::build_geometry(d, Settings::default(), true).unwrap()
    }
    fn build_geometry(d: &Document, mut settings: Settings, physical: bool) -> Option<Self> {
        let (camera, lens) = crate::world3d::default_camera(d);
        if !physical {
            for lamp in &mut settings.lights {
                if lamp.follow_camera {
                    let rotation = Matrix::euler(lens.angles);
                    let pose = rotation.then(Matrix::translation(camera.position));
                    lamp.kind = match lamp.kind {
                        Kind::Directional { direction } => Kind::Directional {
                            direction: rotation.vector(direction),
                        },
                        Kind::Point { position } => Kind::Point {
                            position: pose.map(position),
                        },
                        Kind::Spot {
                            position,
                            direction,
                            angle,
                        } => Kind::Spot {
                            position: pose.map(position),
                            direction: rotation.vector(direction),
                            angle,
                        },
                    };
                }
            }
        }
        let mut t = Self {
            cache: std::sync::Mutex::new(None),
            surface_tools: d.scene.world3d.surfaces.values().any(|s| s.tools.active())
                || settings.style == Style::Wire,
            wires: (0..d.scene.worlds.len()).map(|_| None).collect(),
            wire_colors: vec![[24, 24, 24, 255]; d.scene.worlds.len()],
            camera: camera.clone(),
            policies: (0..d.scene.worlds.len())
                .map(|i| {
                    settings
                        .render
                        .as_ref()
                        .map_or_else(Default::default, |r| r.object(d.scene.catalog.worlds.id(i)))
                })
                .collect(),
            object_bounds: vec![None; d.scene.worlds.len()],
            rays: lens.rays(&camera),
            textures: vec![None; d.scene.worlds.len()],
            imported: (0..d.scene.worlds.len())
                .map(|_| Default::default())
                .collect(),
            physical,
            environment: None,
            triangles: vec![],
            nodes: vec![],
            settings: settings.clone(),
            lens,
        };
        let poses = if physical {
            Default::default()
        } else {
            crate::pose_sequence::meshes(d).unwrap_or_default()
        };
        let frames = if physical {
            Default::default()
        } else {
            crate::model_edit::sampled_frames(d).unwrap_or_default()
        };
        for (object, m) in d.scene.worlds.iter().enumerate() {
            let m = poses.get(&d.scene.catalog.worlds.id(object)).unwrap_or(m);
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
            let frame = frames.get(&id).copied().unwrap_or_else(|| {
                crate::world3d::node(&d.scene, id).map_or(Matrix::default(), |n| n.frame)
            });
            let original_frame = frame;
            let frame = if physical {
                frame
            } else {
                crate::scene_render::billboard(frame, t.policies[object].billboard, &camera, lens)
            };
            let delta = original_frame.inverse().then(frame);
            let inv = frame.inverse();
            if !physical && let Some(attributes) = &s.imported {
                for (index, m) in &attributes.materials {
                    let pixels = m.image.as_ref().and_then(|i| {
                        i.decode()
                            .ok()
                            .map(|p| (i.width as usize, i.height as usize, p))
                    });
                    t.imported[object].insert(
                        *index,
                        ImportedMaterial {
                            pixels,
                            wrap: m.wrap,
                            flip_v: m.flip_v,
                            alpha: m.alpha,
                            cutoff: m.cutoff,
                            uv: Default::default(),
                            normal: None,
                            strength: 1.,
                            flip_green: false,
                        },
                    );
                }
            }
            let visible = physical
                || match s.tools.display {
                    crate::surface_edit::Display::Wire => false,
                    crate::surface_edit::Display::Source => settings.style != Style::Wire,
                    _ => true,
                };
            if !physical {
                if matches!(
                    s.tools.display,
                    crate::surface_edit::Display::Wire | crate::surface_edit::Display::Both
                ) || s.tools.display == crate::surface_edit::Display::Source
                    && settings.style == Style::Wire
                {
                    let mut mesh = m.clone();
                    if t.policies[object].billboard != crate::scene_render::Billboard::Off {
                        for p in &mut mesh.vertices {
                            *p = delta.map(*p);
                        }
                    }
                    t.wires[object] = Some(crate::render::Wires::new(&d.scene, &mesh));
                    if s.tools.display == crate::surface_edit::Display::Source {
                        t.wire_colors[object] = m.color;
                    }
                }
                for (index, custom) in &s.tools.materials {
                    let slot =
                        t.imported[object]
                            .entry(*index)
                            .or_insert_with(|| ImportedMaterial {
                                pixels: None,
                                wrap: [crate::model_import::Wrap::Repeat; 2],
                                flip_v: true,
                                alpha: if s.materials[*index as usize].rgba[3] >= 1. {
                                    crate::model_import::Alpha::Opaque
                                } else {
                                    crate::model_import::Alpha::Blend
                                },
                                cutoff: 0.5,
                                uv: Default::default(),
                                normal: None,
                                strength: 1.,
                                flip_green: false,
                            });
                    if let Some(alpha) = custom.alpha {
                        slot.alpha = alpha;
                        slot.cutoff = custom.cutoff;
                    }
                    slot.uv = custom.uv.clone();
                    for axis in 0..2 {
                        if let Some(w) = custom.uv.wrap[axis] {
                            slot.wrap[axis] = w;
                        }
                    }
                    if let Some(img) = &custom.diffuse {
                        slot.pixels = Some((
                            img.image.width as usize,
                            img.image.height as usize,
                            img.image.decode().ok()?,
                        ));
                        slot.flip_v = true;
                    }
                    if let Some(img) = &custom.normal {
                        slot.normal = Some((
                            img.image.width as usize,
                            img.image.height as usize,
                            img.image.decode().ok()?,
                        ));
                    }
                    slot.strength = custom.normal_strength;
                    slot.flip_green = custom.flip_green;
                }
            }
            if !physical
                && let Some(paint) = &s.paint
                && let Ok(pixels) = paint.decode()
            {
                t.textures[object] = Some((paint.size as usize, pixels));
            }
            let flip = poses.contains_key(&id)
                && crate::pose_sequence::sampled_rig(d, id).is_ok_and(|r| {
                    d.scene
                        .world3d
                        .rigs
                        .get(&id)
                        .is_some_and(|base| r.mirrored != base.mirrored)
                });
            for (face_index, face) in s.triangles.iter().enumerate() {
                let mut reflected;
                let face = if flip {
                    reflected = face.clone();
                    reflected.v.swap(1, 2);
                    if let Some(uv) = &mut reflected.t {
                        uv.swap(1, 2);
                    }
                    &reflected
                } else {
                    face
                };
                let p = face.v.map(|i| {
                    if physical
                        || t.policies[object].billboard == crate::scene_render::Billboard::Off
                    {
                        m.vertices[i as usize]
                    } else {
                        delta.map(m.vertices[i as usize])
                    }
                });
                let area = cross(sub(p[1], p[0]), sub(p[2], p[0]));
                if dot(area, area) < 1e-24 {
                    continue;
                }
                let color = s.materials[face.material as usize].rgba.map(|v| v as f64);
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
                    visible,
                    p,
                    n,
                    color,
                    object,
                    uv: face
                        .t
                        .map(|indices| indices.map(|i| s.texcoords[i as usize])),
                    face: face_index,
                    opaque: physical
                        || (match t.policies[object].alpha {
                            crate::scene_render::Alpha::Opaque
                            | crate::scene_render::Alpha::Threshold => props.opacity == 100,
                            crate::scene_render::Alpha::Blend => false,
                            crate::scene_render::Alpha::Source => {
                                props.opacity == 100
                                    && t.imported[object]
                                        .get(&face.material)
                                        .map_or(color[3] >= 1., |m| {
                                            m.alpha != crate::model_import::Alpha::Blend
                                        })
                            }
                        }),
                    material: face.material,
                    bounds: Bounds::of(p),
                });
            }
        }
        if !t.triangles.is_empty() {
            t.build(0, t.triangles.len());
        }
        Some(t)
    }
    pub(crate) fn paint_view(d: &Document) -> Self {
        let mut t = Self::build_geometry(d, Settings::default(), false).unwrap();
        for triangle in &mut t.triangles {
            triangle.visible = true;
        }
        t
    }
    pub(crate) fn uv(&self, object: usize, point: Point) -> Option<(Point, usize)> {
        let h = self.nearest(self.ray(point), self.lens.near, self.lens.far, |t| {
            t.visible
        })?;
        let t = &self.triangles[h.index];
        if t.object != object {
            return None;
        }
        let uv = self.mapped_uv(t, h.u, h.v)?;
        let uv = if let Some(m) = self.imported[t.object].get(&t.material) {
            std::array::from_fn(|i| crate::surface_edit::wrap(uv[i], m.wrap[i]))
        } else {
            uv
        };
        Some((uv, t.face))
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
                        && (self.physical || self.imported_sample(t, u, v)[3] > 0.)
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
        let (o, d) = self.rays.at(p);
        Ray { o, d }
    }
    pub(crate) fn modern(&self) -> bool {
        self.settings.render.is_some() || self.surface_tools
    }
    pub(crate) fn fragments_cached(&self, objects: &[usize], p: Point) -> Vec<(usize, [u8; 4])> {
        if let Ok(cache) = self.cache.lock()
            && let Some(c) = &*cache
            && c.point == p
            && c.objects == objects
        {
            return c.pixels.clone();
        }
        let pixels = self.fragments(objects, p);
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(FragmentCache {
                point: p,
                objects: objects.to_vec(),
                pixels: pixels.clone(),
            });
        }
        pixels
    }
    pub(crate) fn fragments(&self, objects: &[usize], p: Point) -> Vec<(usize, [u8; 4])> {
        let ray = self.ray(p);
        let mut hits = vec![];
        let mut stack = vec![];
        if !self.nodes.is_empty() {
            stack.push(0);
        }
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if !node.bounds.hit(ray, self.lens.near, self.lens.far) {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
            } else {
                for i in node.start..node.end {
                    let t = &self.triangles[i];
                    if t.visible
                        && objects.contains(&t.object)
                        && let Some((depth, u, v)) =
                            intersect(t, ray, self.lens.near, self.lens.far)
                        && self.imported_sample(t, u, v)[3] > 0.
                    {
                        hits.push(Hit {
                            index: i,
                            t: depth,
                            u,
                            v,
                        });
                    }
                }
            }
        }
        hits.sort_by(|a, b| {
            b.t.total_cmp(&a.t).then_with(|| {
                self.triangles[a.index]
                    .object
                    .cmp(&self.triangles[b.index].object)
            })
        });
        let mut last = vec![None::<f64>; self.object_bounds.len()];
        hits.retain(|h| {
            let object = self.triangles[h.index].object;
            let duplicate =
                last[object].is_some_and(|depth| (depth - h.t).abs() < 2e-9 + 1e-12 * h.t.abs());
            if !duplicate {
                last[object] = Some(h.t);
            }
            !duplicate
        });
        let mut output = hits
            .iter()
            .map(|h| (self.triangles[h.index].object, self.shade(ray, *h)))
            .collect::<Vec<_>>();
        if let Some(options) = &self.settings.render
            && options.outline > 0
        {
            let near = hits.last().copied();
            let radius = options.outline as f64;
            let mut edge = None;
            for delta in [[-radius, 0.], [radius, 0.], [0., -radius], [0., radius]] {
                let other_ray = self.ray(add2(p, delta));
                let other = self.nearest(other_ray, self.lens.near, self.lens.far, |t| {
                    objects.contains(&t.object)
                });
                let changed = match (near, other) {
                    (Some(a), Some(b)) => {
                        self.triangles[a.index].object != self.triangles[b.index].object
                            || (a.t - b.t).abs() > 0.02 * (1. + a.t.abs())
                            || dot(self.normal(ray, a, true), self.normal(other_ray, b, true)) < 0.7
                    }
                    (None, None) => false,
                    _ => true,
                };
                if changed {
                    edge = near.or(other);
                    break;
                }
            }
            if let Some(hit) = edge {
                output.push((self.triangles[hit.index].object, options.line));
            }
        }
        for object in objects {
            if let Some(w) = &self.wires[*object]
                && w.near(p)
            {
                output.push((*object, self.wire_colors[*object]));
            }
        }
        output
    }
    pub(crate) fn pixel(&self, object: usize, p: Point) -> [u8; 4] {
        if self.modern() {
            return self
                .fragments(&[object], p)
                .into_iter()
                .fold([0; 4], |back, (_, top)| crate::render::over(back, top));
        }

        let ray = self.ray(p);
        if self
            .object_bounds
            .get(object)
            .and_then(|b| *b)
            .is_none_or(|b| !b.hit(ray, self.lens.near, self.lens.far))
        {
            return [0; 4];
        }
        let Some(h) = self.nearest(ray, self.lens.near, self.lens.far, |t| {
            t.visible && t.object == object
        }) else {
            return [0; 4];
        };
        if self
            .nearest(ray, self.lens.near, h.t - 1e-7, |t| {
                t.visible && t.object != object && t.opaque
            })
            .is_some()
        {
            return [0; 4];
        }
        self.shade(ray, h)
    }
    pub(crate) fn finish_view(d: &Document) -> Self {
        let mut settings = d.scene.world3d.lighting.clone().unwrap_or_default();
        if d.scene.world3d.lighting.is_none() || settings.style == Style::Wire {
            settings.style = Style::Unlit;
        }
        Self::build_geometry(d, settings, false).unwrap()
    }
    pub(crate) fn detail(&self, object: usize, point: Point) -> Option<(f64, V)> {
        let r = self.ray(point);
        let b = self.object_bounds.get(object).and_then(|v| *v)?;
        if !b.hit(r, self.lens.near, self.lens.far) {
            return None;
        }
        let h = self.nearest(r, self.lens.near, self.lens.far, |t| {
            t.visible && t.object == object
        })?;
        if self
            .nearest(r, self.lens.near, h.t - 1e-7, |t| {
                t.visible && t.object != object && t.opaque
            })
            .is_some()
        {
            return None;
        }
        Some((h.t, self.normal(r, h, true)))
    }
    fn mapped_uv(&self, t: &Triangle, u: f64, v: f64) -> Option<Point> {
        let uv = t.uv?;
        let p = std::array::from_fn(|i| uv[0][i] * (1. - u - v) + uv[1][i] * u + uv[2][i] * v);
        Some(
            self.imported[t.object]
                .get(&t.material)
                .map_or(p, |m| m.uv.map(p)),
        )
    }
    fn texture_pixel(
        &self,
        p: Point,
        m: &ImportedMaterial,
        flip_v: bool,
        w: usize,
        height: usize,
    ) -> usize {
        let x = crate::surface_edit::wrap(p[0], m.wrap[0]);
        let y = crate::surface_edit::wrap(if flip_v { 1. - p[1] } else { p[1] }, m.wrap[1]);
        (((y * height as f64).floor() as usize).min(height - 1) * w
            + ((x * w as f64).floor() as usize).min(w - 1))
            * 4
    }
    fn normal(&self, r: Ray, h: Hit, smooth: bool) -> V {
        let t = &self.triangles[h.index];
        let mut n = unit(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0])));
        if smooth && let Some(normals) = t.n {
            let v = add(
                add(mul(normals[0], 1. - h.u - h.v), mul(normals[1], h.u)),
                mul(normals[2], h.v),
            );
            if dot(v, v) > 1e-20 {
                n = unit(v);
            }
        }
        if let Some(m) = self.imported[t.object].get(&t.material)
            && let Some((w, height, pixels)) = &m.normal
            && let Some(p) = self.mapped_uv(t, h.u, h.v)
            && let Some(uv) = t.uv
        {
            let coordinates = uv.map(|uv| m.uv.map([uv[0], uv[1]]));
            let a = sub(t.p[1], t.p[0]);
            let b = sub(t.p[2], t.p[0]);
            let du1 = coordinates[1][0] - coordinates[0][0];
            let dv1 = coordinates[1][1] - coordinates[0][1];
            let du2 = coordinates[2][0] - coordinates[0][0];
            let dv2 = coordinates[2][1] - coordinates[0][1];
            let determinant = du1 * dv2 - du2 * dv1;
            if determinant.abs() > 1e-12 && m.strength > 0. {
                let tangent = mul(sub(mul(a, dv2), mul(b, dv1)), 1. / determinant);
                let bitangent = mul(sub(mul(b, du1), mul(a, du2)), 1. / determinant);
                let tangent = sub(tangent, mul(n, dot(tangent, n)));
                if dot(tangent, tangent) > 1e-20 {
                    let tangent = unit(tangent);
                    let handed = if dot(cross(n, tangent), bitangent) < 0. {
                        -1.
                    } else {
                        1.
                    };
                    let binormal = mul(cross(n, tangent), handed);
                    let index = self.texture_pixel(p, m, true, *w, *height);
                    let x = (pixels[index] as f64 / 255. * 2. - 1.) * m.strength;
                    let y = (pixels[index + 1] as f64 / 255. * 2. - 1.)
                        * m.strength
                        * if m.flip_green { -1. } else { 1. };
                    let z = pixels[index + 2] as f64 / 255. * 2. - 1.;
                    let mapped = add(add(mul(tangent, x), mul(binormal, y)), mul(n, z));
                    if dot(mapped, mapped) > 1e-20 {
                        n = unit(mapped);
                    }
                }
            }
        }
        if dot(n, r.d) > 0. {
            n = mul(n, -1.);
        }
        n
    }
    fn shade(&self, r: Ray, h: Hit) -> [u8; 4] {
        let t = &self.triangles[h.index];
        let p = add(r.o, mul(r.d, h.t));
        let n = self.normal(
            r,
            h,
            matches!(self.settings.style, Style::Smooth | Style::Toon),
        );
        let mut light = self
            .settings
            .render
            .as_ref()
            .map_or([self.settings.ambient; 3], |r| {
                r.tint.map(|v| v * self.settings.ambient)
            });
        if matches!(self.settings.style, Style::Unlit | Style::Wire) {
            light = [1.; 3];
        } else {
            if let Some(environment) = &self.environment {
                let color = environment.diffuse(n);
                for i in 0..3 {
                    light[i] += color[i];
                }
            }
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
                if self.settings.shadows
                    && lamp.shadow
                    && self
                        .settings
                        .render
                        .as_ref()
                        .is_none_or(|r| r.receives(&self.policies[t.object]))
                {
                    let epsilon = 1e-5 + p.into_iter().map(f64::abs).fold(0., f64::max) * 1e-12;
                    let sample = if let Some(r) = &self.settings.render {
                        let span = self.nodes.first().map_or(1., |n| {
                            (0..3)
                                .map(|i| n.bounds.hi[i] - n.bounds.lo[i])
                                .fold(0., f64::max)
                        });
                        crate::scene_render::shadow_sample(p, dir, span, r.shadow.resolution())
                    } else {
                        p
                    };
                    let shadow = Ray {
                        o: add(sample, mul(n, epsilon)),
                        d: dir,
                    };
                    if self
                        .nearest(shadow, epsilon, (maximum - epsilon).max(epsilon), |t| {
                            t.opaque
                                && (self.settings.render.is_none()
                                    || (self.policies[t.object].cast
                                        && (t.face != self.triangles[h.index].face
                                            || t.object != self.triangles[h.index].object)))
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
        let imported = self.imported_sample(t, h.u, h.v);
        let mut texture = [imported[0], imported[1], imported[2]];
        if let Some((size, pixels)) = &self.textures[t.object]
            && let Some(uv) = t.uv
        {
            let _ = uv;
            let p = self.mapped_uv(t, h.u, h.v).unwrap();
            let p = if let Some(m) = self.imported[t.object].get(&t.material) {
                std::array::from_fn(|i| crate::surface_edit::wrap(p[i], m.wrap[i]))
            } else {
                p
            };
            let [u, v] = p;
            let [u, v] = if self.imported[t.object].contains_key(&t.material) {
                [u, v]
            } else {
                [u.rem_euclid(1.), v.rem_euclid(1.)]
            };
            let x = (u * *size as f64).floor() as usize;
            let y = ((1. - v) * *size as f64).floor() as usize;
            let i = (y.min(*size - 1) * *size + x.min(*size - 1)) * 4;
            texture = std::array::from_fn(|c| pixels[i + c] as f64 / 255.);
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
        let fog = self.settings.render.as_ref().and_then(|r| r.fog.as_ref());
        let coverage = fog.map_or(0., |f| f.coverage(p, &self.camera, self.lens));
        let color = std::array::from_fn::<_, 3, _>(|i| {
            encode(
                linear(t.color[i]) * linear(texture[i]) * light[i] * (1. - coverage)
                    + fog.map_or(0., |f| linear(f.color[i])) * coverage,
            )
        });
        [
            color[0],
            color[1],
            color[2],
            (imported[3] * 255.).round() as u8,
        ]
    }
    fn policy_alpha(&self, t: &Triangle, alpha: f64) -> f64 {
        use crate::scene_render::Alpha;
        let p = &self.policies[t.object];
        match p.alpha {
            Alpha::Source => alpha,
            Alpha::Blend => alpha * p.alpha_scale,
            Alpha::Opaque => 1.,
            Alpha::Threshold => {
                if alpha * p.alpha_scale >= p.threshold {
                    1.
                } else {
                    0.
                }
            }
        }
    }
    fn imported_sample(&self, t: &Triangle, u: f64, v: f64) -> [f64; 4] {
        use crate::model_import::Alpha;
        let Some(m) = self.imported[t.object].get(&t.material) else {
            let alpha = self.policy_alpha(t, t.color[3]);
            return [1., 1., 1., alpha];
        };
        let mut color = [1.; 4];
        if let Some((w, h, pixels)) = &m.pixels
            && let Some(p) = self.mapped_uv(t, u, v)
        {
            let index = self.texture_pixel(p, m, m.flip_v, *w, *h);
            color = std::array::from_fn(|i| pixels[index + i] as f64 / 255.);
        }
        let alpha = color[3] * t.color[3];
        color[3] = if self.policies[t.object].alpha != crate::scene_render::Alpha::Source {
            self.policy_alpha(t, alpha)
        } else {
            match m.alpha {
                Alpha::Opaque => 1.,
                Alpha::Mask => {
                    if alpha >= m.cutoff {
                        1.
                    } else {
                        0.
                    }
                }
                Alpha::Blend => alpha,
            }
        };
        color
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

fn add2(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
