use super::*;
use crate::{layer_search::Key, scene::PageObject};
impl Renderer<'_> {
    pub(crate) fn vector_run(&self, keys: &[Key], point: Point) -> Result<Sample, String> {
        let original = self.document.scene.transform.inverse().map(point);
        let Some(Key::Layer(Target::Vector(base))) = keys.first() else {
            return Err("Missing vector run".into());
        };
        let mut group = self.vector(*base, point, original)?;
        for key in &keys[1..] {
            if let Key::Layer(Target::Vector(top)) = key {
                group = self.plan().space.atop(
                    group,
                    self.vector(*top, point, original)?,
                    crate::layer_blend::mode(self.document, Target::Vector(*top)),
                    1.,
                )?;
            }
        }
        Ok(group)
    }
    pub(crate) fn source_pixel(&self, key: Key, point: Point) -> Result<Sample, String> {
        let d = self.document;
        if point.iter().any(|v| !v.is_finite()) {
            return Err("Nonfinite source sample point".into());
        }
        let original = d.scene.transform.inverse().map(point);
        if matches!(
            key,
            Key::Gradient(_) | Key::Object(_) | Key::World(_) | Key::Layer(Target::Vector(_))
        ) && (d.scene.clipping.solo.is_some()
            || d.scene
                .clips
                .iter()
                .any(|clip| !crate::render::inside(clip, original)))
        {
            return Ok(Sample::default());
        }
        match crate::layer_catalog::target(d, key) {
            Some(target @ (Target::Raster(_) | Target::Folder(_))) => {
                let node = crate::clipping::node(d, target).ok_or("Missing raster source")?;
                if let Target::Raster(id) = target
                    && self.plan().surfaces.contains_key(&id)
                {
                    return self.surface_contribution(
                        crate::layers::index(d, id).ok_or("Missing surface source")?,
                        point.map(|v| v.floor() as i64),
                    );
                }
                if let Some(base) = d.scene.clipping.binding(target) {
                    if !crate::clipping::active(d, target) {
                        return Ok(Sample::default());
                    }
                    let Some(base) = crate::clipping::node(d, base) else {
                        return Ok(Sample::default());
                    };
                    let pixel = point.map(|v| v.floor() as i64);
                    let original = d
                        .scene
                        .transform
                        .inverse()
                        .map(pixel.map(|v| v as f64 + 0.5));
                    return self
                        .source(node, pixel, original, 0)?
                        .coverage(self.source(base, pixel, original, 0)?.alpha());
                }
                if matches!(target, Target::Raster(_)) {
                    self.page_source(key, point, original)
                } else {
                    self.nodes(
                        &[node],
                        Sample::default(),
                        point.map(|v| v.floor() as i64),
                        original,
                        0,
                    )
                }
            }
            Some(Target::Vector(id)) => {
                let target = Target::Vector(id);
                let raw = self.vector(id, point, original)?;
                if let Some(Target::Vector(base)) = d.scene.clipping.binding(target) {
                    if !crate::clipping::active(d, target) {
                        return Ok(Sample::default());
                    }
                    return raw.coverage(self.vector(base, point, original)?.alpha());
                }
                Ok(raw)
            }
            Some(Target::Track(index)) => {
                let Some((_, cell, _)) = d.animation.as_ref().and_then(|a| {
                    a.indexed_visible_cells(d.current_frame)
                        .find(|(t, _, _)| *t == index as usize)
                }) else {
                    return Ok(Sample::default());
                };
                let track = d.scene.catalog.tracks.id(index as usize);
                self.source_pixel(
                    Key::Cell {
                        track,
                        cell: d.scene.catalog.cell_id(track, cell),
                    },
                    point,
                )
            }
            Some(Target::Cell { track, cell }) => {
                let Some(c) = d
                    .animation
                    .as_ref()
                    .and_then(|a| a.tracks.get(track as usize))
                    .and_then(|t| t.cells.get(cell as usize))
                else {
                    return Ok(Sample::default());
                };
                let t = Target::Track(track);
                let id = d.scene.catalog.tracks.id(track as usize);
                let track_key = Key::Track(id);
                let raw = if let Some(cache) = d
                    .scene
                    .file_objects
                    .pixel(track_key, original)
                    .or_else(|| d.scene.file_objects.pixel(key, original))
                {
                    self.plan().space.from_srgb(Sample::from8(cache))?
                } else {
                    Sample::from8(c.pixel(point[0].floor() as i64, point[1].floor() as i64))
                };
                let weight = p::factor(d, t)
                    * if d.scene.file_objects.get(track_key).is_some() {
                        1.
                    } else {
                        p::factor(d, Target::Cell { track, cell })
                    };
                self.plan().space.compose(
                    Sample::default(),
                    raw,
                    crate::layer_blend::mode(d, t),
                    weight,
                )
            }
            Some(target @ (Target::Object(_) | Target::Gradient(_) | Target::World(_))) => {
                self.plan().space.compose(
                    Sample::default(),
                    self.page_source(key, point, original)?,
                    crate::layer_blend::mode(d, target),
                    1.,
                )
            }
            _ => Ok(Sample::default()),
        }
    }
    fn gradient_mix(
        &self,
        a: [u8; 4],
        b: [u8; 4],
        w: f64,
        space: crate::gradient_ramp::Space,
        brightness: crate::gradient_ramp::Brightness,
    ) -> Result<Sample, String> {
        use crate::{
            color_mix::{self as c, Space as C},
            gradient_ramp::Space as G,
        };
        let a = self.plan().space.to_srgb(Sample::from8(a))?;
        let b = self.plan().space.to_srgb(Sample::from8(b))?;
        let aa = a.alpha() * (1. - w);
        let ba = b.alpha() * w;
        let alpha = aa + ba;
        if alpha == 0. {
            return Ok(Sample::default());
        }
        let cs = match space {
            G::Standard | G::LinearRgb => C::Standard,
            G::PerceptualA => C::Perceptual,
            G::PerceptualB => C::LabD65,
        };
        let mut ac = c::coordinates_float(cs, a.rgb());
        let mut bc = c::coordinates_float(cs, b.rgb());
        if space == G::LinearRgb {
            ac = ac.map(c::linear);
            bc = bc.map(c::linear);
        }
        let mixed = [0, 1, 2].map(|i| (ac[i] * aa + bc[i] * ba) / alpha);
        let mut rgb = if space == G::LinearRgb {
            mixed.map(c::encoded)
        } else {
            c::rgb_float(cs, mixed)
        };
        if brightness != crate::gradient_ramp::Brightness::Off {
            let luminance = |v: [f64; 3]| v[0] * 0.2126 + v[1] * 0.7152 + v[2] * 0.0722;
            let linear = rgb.map(c::linear);
            let current = luminance(linear);
            let desired = (luminance(a.rgb().map(c::linear)) * aa
                + luminance(b.rgb().map(c::linear)) * ba)
                / alpha;
            let target = current + (desired - current) * brightness.amount();
            rgb = linear
                .map(|v| {
                    if target >= current {
                        v + (1. - v) * (target - current) / (1. - current).max(1e-12)
                    } else {
                        v * target / current.max(1e-12)
                    }
                })
                .map(c::encoded);
        }
        self.plan()
            .space
            .from_srgb(Sample::new([rgb[0], rgb[1], rgb[2], alpha])?)
    }
    fn gradient_color(
        &self,
        s: &crate::gradient::Settings,
        p: Point,
        anchors: Option<[Point; 2]>,
    ) -> Result<Sample, String> {
        use crate::gradient_ramp::{Brightness, Curve, Space};
        let Some(t) = s.parameter(p, anchors) else {
            return Ok(Sample::default());
        };
        let mut sample = if let Some(ramp) = &s.ramp {
            let i = ramp.stops.partition_point(|s| s.position <= t);
            if i == 0 {
                Sample::from8(ramp.stops[0].color)
            } else {
                let a = &ramp.stops[i - 1];
                if i == ramp.stops.len() || t == a.position {
                    Sample::from8(a.color)
                } else {
                    let b = &ramp.stops[i];
                    let w = ((t - a.position) / (b.position - a.position))
                        .clamp(0., 1.)
                        .powf(0.5f64.ln() / a.midpoint.ln());
                    let w = match a.curve {
                        Curve::Linear => w,
                        Curve::Smooth => w * w * (3. - 2. * w),
                        Curve::EaseIn => w * w,
                        Curve::EaseOut => 1. - (1. - w) * (1. - w),
                    };
                    self.gradient_mix(a.color, b.color, w, ramp.space, ramp.brightness)?
                }
            }
        } else {
            self.gradient_mix(s.first, s.last, t, Space::Standard, Brightness::Off)?
        };
        if s.dithering && sample.alpha() > 0. {
            let bayer = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
            let seed = self.document.color.dither.seed;
            let x = (p[0].floor() as i64 + (seed & 3) as i64).rem_euclid(4) as usize;
            let y = (p[1].floor() as i64 + ((seed >> 2) & 3) as i64).rem_euclid(4) as usize;
            let noise = ((bayer[y][x] as f64 + 0.5) / 16. - 0.5)
                / self.document.color.depth.maximum() as f64;
            sample = sample.with_rgb(sample.rgb().map(|v| v + noise))?;
        }
        Ok(sample)
    }
    pub(crate) fn gradient(
        &self,
        g: &crate::gradient::Layer,
        original: Point,
    ) -> Result<Sample, String> {
        use crate::gradient::Cover;
        if g.hidden {
            return Ok(Sample::default());
        }
        let p = g.creation.map(original);
        for area in &g.areas {
            let mut coverage = area.paint.coverage(p);
            let mut color = self.gradient_color(&g.settings, p, area.anchors)?;
            for (mask, policy) in [
                (&area.black, g.settings.black),
                (&area.transparent, g.settings.transparency),
            ] {
                let strength = mask.coverage(p);
                if strength > coverage && policy != Cover::Keep {
                    coverage = strength;
                    if policy == Cover::DrawingColor {
                        color = Sample::from8(g.settings.drawing);
                    }
                }
            }
            if coverage > 0 {
                return color.coverage(coverage as f64 / 255.);
            }
        }
        Ok(Sample::default())
    }
    fn material(
        &self,
        m: &crate::material_layer::Layer,
        original: Point,
    ) -> Result<Sample, String> {
        let Some(p) = m.local(original) else {
            return Ok(Sample::default());
        };
        if let crate::material_layer::Source::Gradient(s) = &m.source {
            self.gradient_color(s, p, None)
        } else {
            if matches!(
                m.source,
                crate::material_layer::Source::Image(_) | crate::material_layer::Source::Metal(_)
            ) {
                self.plan()
                    .space
                    .from_srgb(Sample::from8(m.pixel(original)))
            } else {
                Ok(Sample::from8(m.pixel(original)))
            }
        }
    }
    fn object(&self, n: usize, original: Point) -> Result<Sample, String> {
        match &self.document.scene.objects[n] {
            PageObject::Material(m) => self.material(m, original),
            PageObject::Shadow {
                bounds,
                offset,
                radius_px,
                color,
            } => {
                let p = [original[0] - offset[0], original[1] - offset[1]];
                let delta =
                    [0, 1].map(|i| (bounds.min[i] - p[i]).max(0.).max(p[i] - bounds.max[i]));
                Sample::from8(*color)
                    .coverage((1. - delta[0].hypot(delta[1]) / radius_px).clamp(0., 1.))
            }
            object => Ok(Sample::from8(
                self.plan().geometry.object_pixel(n, object, original),
            )),
        }
    }
    pub(super) fn vector_raw(&self, id: u64, original: Point) -> Result<Sample, String> {
        let d = self.document;
        let t = Target::Vector(id);
        if let Some(cache) = d.scene.file_objects.pixel(Key::Layer(t), original) {
            self.plan().space.from_srgb(Sample::from8(cache))
        } else {
            let Some(layer) = d.scene.lines.layers.iter().find(|l| l.id == id) else {
                return Ok(Sample::default());
            };
            let mut raw = Sample::default();
            for path in &layer.paths {
                if self
                    .sampling
                    .fill
                    .is_some_and(|(_, s)| s.advanced.vector_centerline)
                {
                    let point = d.scene.transform.map(original);
                    let value = if path.segments().any(|(a, b)| {
                        crate::line_tools::distance(
                            point,
                            d.scene.transform.map(a),
                            d.scene.transform.map(b),
                        )
                        .0 <= 0.5
                    }) {
                        if path.stroke {
                            path.color
                        } else {
                            path.fill.unwrap_or(path.color)
                        }
                    } else {
                        [0; 4]
                    };
                    raw = self.plan().space.compose(
                        raw,
                        Sample::from8(value),
                        crate::layer_blend::Mode::Normal,
                        1.,
                    )?;
                    continue;
                }
                let fill = path.fill_pixel(original);
                let mut sample = Sample::from8(fill);
                if path.stroke && path.stroke_hit(original) {
                    sample = self.plan().space.compose(
                        sample,
                        Sample::from8(path.color),
                        crate::layer_blend::Mode::Normal,
                        1.,
                    )?;
                }
                raw =
                    self.plan()
                        .space
                        .compose(raw, sample, crate::layer_blend::Mode::Normal, 1.)?;
            }
            Ok(raw)
        }
    }
    fn vector(&self, id: u64, point: Point, original: Point) -> Result<Sample, String> {
        let d = self.document;
        let t = Target::Vector(id);
        let Some(index) = d.scene.lines.layers.iter().position(|l| l.id == id) else {
            return Ok(Sample::default());
        };
        if self.sampling.references.is_some_and(|r| !r.vectors)
            || self.sampling.fill.is_some_and(|(_, s)| {
                !crate::fill_reference::vector(&d.scene.lines.layers[index], index, s)
            })
        {
            return Ok(Sample::default());
        }
        if self
            .sampling
            .fill
            .is_some_and(|(_, s)| s.advanced.vector_centerline)
            && d.scene.file_objects.get(Key::Layer(t)).is_none()
        {
            return self
                .vector_raw(id, original)?
                .coverage(p::factor(d, t) * d.scene.layer_effects.mask(t, original) as f64);
        }
        let raw = if let Some(sample) = self.cached_display(t, point) {
            sample
        } else {
            let raw = self.vector_raw(id, original)?;
            if d.scene.file_objects.get(Key::Layer(t)).is_some() {
                raw
            } else {
                self.display(
                    raw,
                    t,
                    point,
                    d.scene
                        .transform
                        .inverse()
                        .map(point.map(|v| v.floor() + 0.5)),
                )?
            }
        };
        raw.coverage(p::factor(d, t) * d.scene.layer_effects.mask(t, original) as f64)
    }
    fn page_source(&self, key: Key, point: Point, original: Point) -> Result<Sample, String> {
        let d = self.document;
        let Some(target) = crate::layer_catalog::target(d, key) else {
            return Ok(Sample::default());
        };
        if let Target::Raster(id) = target {
            let n = crate::layers::index(d, id).ok_or("Missing ordered raster")?;
            let node = Node::Raster(n);
            if self.plan().surfaces.contains_key(&id) {
                return self.put(
                    Sample::default(),
                    self.surface_contribution(n, point.map(|v| v.floor() as i64))?,
                    node,
                    original,
                );
            }
            let source = self
                .shade(
                    self.paper(node),
                    self.raster(n, point.map(|v| v.floor() as i64), original)?,
                    original,
                )?
                .coverage(self.weight(node, original))?;
            return self.put(Sample::default(), source, node, original);
        }
        let raw = if let Some(cache) = d.scene.file_objects.pixel(key, original) {
            self.plan().space.from_srgb(Sample::from8(cache))?
        } else {
            match key {
                Key::Gradient(id) => d
                    .scene
                    .gradients
                    .layers
                    .iter()
                    .find(|g| g.id == id)
                    .map_or_else(|| Ok(Sample::default()), |g| self.gradient(g, original))?,
                Key::Object(_) => {
                    let n = crate::layer_catalog::source_index(d, key)
                        .ok_or("Missing float page object")?;
                    self.object(n, original)?
                }
                Key::World(_) => {
                    let n =
                        crate::layer_catalog::source_index(d, key).ok_or("Missing float world")?;
                    Sample::from8(self.plan().geometry.world_pixel(n, original))
                }
                _ => return Ok(Sample::default()),
            }
        };
        raw.coverage(p::factor(d, target))
    }
    pub(super) fn timeline(
        &self,
        mut back: Sample,
        pixel: [i64; 2],
        original: Point,
    ) -> Result<Sample, String> {
        let d = self.document;
        if self.sampling.references.is_some_and(|r| !r.animation)
            || self.sampling.fill.is_some_and(|(_, s)| {
                !matches!(
                    s.reference,
                    crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                )
            })
        {
            return Ok(back);
        }
        for (track, cell_index, cell) in d
            .animation
            .iter()
            .flat_map(|a| a.indexed_visible_cells(d.current_frame))
        {
            let target = Target::Track(track as u16);
            let id = d.scene.catalog.tracks.id(track);
            let track_key = Key::Track(id);
            let cell_key = Key::Cell {
                track: id,
                cell: d.scene.catalog.cell_id(id, cell_index),
            };
            let raw = if let Some(cache) = d
                .scene
                .file_objects
                .pixel(track_key, original)
                .or_else(|| d.scene.file_objects.pixel(cell_key, original))
            {
                self.plan().space.from_srgb(Sample::from8(cache))?
            } else {
                Sample::from8(cell.pixel(pixel[0], pixel[1]))
            };
            let weight = p::factor(d, target)
                * if d.scene.file_objects.get(track_key).is_some() {
                    1.
                } else {
                    p::factor(
                        d,
                        Target::Cell {
                            track: track as u16,
                            cell: cell_index as u16,
                        },
                    )
                };
            back = self.plan().space.compose(
                back,
                raw,
                crate::layer_blend::mode(d, target),
                weight,
            )?;
        }
        Ok(back)
    }
    pub(super) fn ordered(
        &self,
        mut back: Sample,
        point: Point,
        original: Point,
    ) -> Result<Sample, String> {
        let d = self.document;
        if d.scene.clipping.solo.is_some()
            || d.scene
                .clips
                .iter()
                .any(|clip| !crate::render::inside(clip, original))
        {
            return Ok(back);
        }
        let mut i = 0;
        while i < self.plan().order.len() {
            let key = self.plan().order[i].key;
            if let Key::Layer(Target::Vector(id)) = key {
                let target = Target::Vector(id);
                if d.scene.clipping.binding(target).is_some() {
                    i += 1;
                    continue;
                }
                let mut group = self.vector(id, point, original)?;
                let mut next = i + 1;
                while next < self.plan().order.len() {
                    let Key::Layer(Target::Vector(top)) = self.plan().order[next].key else {
                        break;
                    };
                    let t = Target::Vector(top);
                    let Some(base) = d.scene.clipping.binding(t) else {
                        break;
                    };
                    if base == target {
                        group = self.plan().space.atop(
                            group,
                            self.vector(top, point, original)?,
                            crate::layer_blend::mode(d, t),
                            1.,
                        )?;
                    }
                    next += 1;
                }
                back = self.plan().space.compose(
                    back,
                    group,
                    crate::layer_blend::mode(d, target),
                    1.,
                )?;
                i = next;
            } else {
                let raster = matches!(key, Key::Layer(Target::Raster(_)));
                let objects = self.sampling.references.is_none_or(|r| r.objects)
                    && self.sampling.fill.is_none_or(|(_, s)| {
                        matches!(
                            s.reference,
                            crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                        )
                    });
                if !raster && !objects {
                    i += 1;
                    continue;
                }
                if let Some(target) = crate::layer_catalog::target(d, key) {
                    back = self.plan().space.compose(
                        back,
                        self.page_source(key, point, original)?,
                        crate::layer_blend::mode(d, target),
                        1.,
                    )?;
                }
                i += 1;
            }
        }
        Ok(back)
    }
}
