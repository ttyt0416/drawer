//! Snapshot illustration finishing: separate contour and stepped-tone raster layers.
use crate::{
    document::{Document, PlacedRaster, RasterLayer},
    layer_properties::Target,
    layer_search::Key,
    sizing::DocumentState,
    world_math::dot,
    world3d::V,
};
use std::sync::atomic::AtomicBool;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Image(u64),
    Scene,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub input: Input,
    pub lines: bool,
    pub tones: bool,
    pub luminance: bool,
    pub depth: bool,
    pub normals: bool,
    pub silhouette: bool,
    pub luma_threshold: f64,
    pub depth_threshold: f64,
    pub normal_angle: f64,
    pub radius: u8,
    pub bands: u8,
    pub strength: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            input: Input::Scene,
            lines: true,
            tones: true,
            luminance: true,
            depth: true,
            normals: true,
            silhouette: true,
            luma_threshold: 0.15,
            depth_threshold: 0.05,
            normal_angle: 35.,
            radius: 1,
            bands: 4,
            strength: 1.,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (self.lines || self.tones)
            && self.luma_threshold.is_finite()
            && (0.0001..=1.).contains(&self.luma_threshold)
            && self.depth_threshold.is_finite()
            && (0.0001..=1.).contains(&self.depth_threshold)
            && self.normal_angle.is_finite()
            && (1. ..=180.).contains(&self.normal_angle)
            && (1..=4).contains(&self.radius)
            && (2..=16).contains(&self.bands)
            && self.strength.is_finite()
            && (0. ..=1.).contains(&self.strength)
    }
}
#[derive(Clone, Copy)]
struct Geometry {
    depth: f64,
    normal: V,
}
struct Frame {
    rgba: Vec<[u8; 4]>,
    geometry: Vec<Option<Geometry>>,
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    frame: u32,
    pub added: Vec<u64>,
    pub warnings: Vec<String>,
    pub summary: String,
    pub thumbnail: Vec<u8>,
    pub edge_pixels: usize,
    pub visible_pixels: usize,
}
impl Prepared {
    pub fn matches(&self, d: &Document) -> bool {
        d.id == self.id && d.revision == self.revision && d.current_frame == self.frame
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("Line / tone preview became stale".into());
        }
        d.commit_state(self.state)
    }
    pub fn preview(&self, frame: u32) -> Result<Document, String> {
        crate::layer_merge::preview(&self.state, frame)
    }
}
fn frame(d: &Document, input: Input, cancel: &AtomicBool) -> Result<Frame, String> {
    let n = d.width as usize * d.height as usize;
    let mut f = Frame {
        rgba: Vec::with_capacity(n),
        geometry: if input == Input::Scene {
            Vec::with_capacity(n)
        } else {
            vec![]
        },
    };
    let surface_doc = if input == Input::Scene
        && d.scene
            .world3d
            .lighting
            .as_ref()
            .is_none_or(|s| s.style == crate::lighting::Style::Wire)
    {
        let mut s = d.snapshot();
        let mut settings = s.scene.world3d.lighting.clone().unwrap_or_default();
        settings.style = crate::lighting::Style::Unlit;
        s.scene.world3d.lighting = Some(settings);
        if s.scene.world3d.lens.is_none() {
            s.scene.world3d.lens = Some(crate::world3d::Lens {
                projection: crate::world3d::Projection::Orthographic,
                ..Default::default()
            });
        }
        {
            let mut frozen = crate::layer_merge::preview(&s, d.current_frame)?;
            frozen.keyframe_content = d.keyframe_content;
            Some(frozen)
        }
    } else {
        None
    };
    let capture = surface_doc.as_ref().unwrap_or(d);
    let renderer = crate::render::Renderer::new(capture);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let r = &renderer;
    let trace = (input == Input::Scene).then(|| crate::world_render::Trace::finish_view(capture));
    let mappings = (0..d.scene.worlds.len())
        .map(|i| {
            crate::camera2d::display_matrix(d, Key::World(d.scene.catalog.worlds.id(i)), false)
                .0
                .inverse()
                .then(d.scene.transform.inverse())
        })
        .collect::<Vec<_>>();
    let order = crate::render_order::normalized(&d.scene)
        .into_iter()
        .filter(|e| matches!(e.key, Key::World(_)))
        .collect::<Vec<_>>();
    for y in 0..d.height {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..d.width {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            match input {
                Input::Image(id) => f
                    .rgba
                    .push(r.source_pixel(Key::Layer(Target::Raster(id)), p)),
                Input::Scene => {
                    let mut color = crate::paper::Color::default();
                    let mut geometry: Option<Geometry> = None;
                    for e in &order {
                        let Key::World(id) = e.key else {
                            continue;
                        };
                        let Some(i) = d.scene.catalog.worlds.index(id, d.scene.worlds.len()) else {
                            continue;
                        };
                        let Some((depth, normal)) =
                            trace.as_ref().unwrap().detail(i, mappings[i].map(p))
                        else {
                            continue;
                        };
                        let pixel = r.source_pixel(Key::World(id), p);
                        if pixel[3] == 0 {
                            continue;
                        }
                        let mut source = crate::paper::Color::default();
                        source.add(pixel);
                        crate::layer_catalog::properties(d, Key::World(id))
                            .blend
                            .over(&mut color, source);
                        if geometry.is_none_or(|g| depth < g.depth) {
                            geometry = Some(Geometry { depth, normal });
                        }
                    }
                    f.rgba.push(color.rgba());
                    f.geometry.push(geometry);
                }
            }
        }
    }
    Ok(f)
}
fn linear(v: u8) -> f64 {
    let x = v as f64 / 255.;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn luma(c: [u8; 4]) -> f64 {
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}
fn gray(v: f64) -> u8 {
    let x = if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    };
    (x.clamp(0., 1.) * 255.).round() as u8
}
type Analysis = (Vec<[u8; 4]>, Vec<[u8; 4]>, usize, usize);
fn analyze(
    f: &Frame,
    w: usize,
    h: usize,
    s: Settings,
    cancel: &AtomicBool,
) -> Result<Analysis, String> {
    let mut lines = vec![[0; 4]; w * h];
    let mut tones = lines.clone();
    let mut edges = 0;
    let mut visible = 0;
    let cosine = s.normal_angle.to_radians().cos();
    for y in 0..h {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..w {
            let i = y * w + x;
            let c = f.rgba[i];
            if c[3] == 0 {
                continue;
            }
            visible += 1;
            let a = (c[3] as f64 * s.strength).round() as u8;
            if s.tones {
                let steps = (s.bands - 1) as f64;
                let value = gray((luma(c) * steps).round() / steps);
                tones[i] = [value, value, value, a];
            }
            if !s.lines {
                continue;
            }
            let mut edge = false;
            for distance in 1..=s.radius as isize {
                for (dx, dy) in [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    let nx = x as isize + dx * distance;
                    let ny = y as isize + dy * distance;
                    let other = if nx >= 0 && ny >= 0 && nx < w as isize && ny < h as isize {
                        Some(ny as usize * w + nx as usize)
                    } else {
                        None
                    };
                    let b = other.map_or([0; 4], |i| f.rgba[i]);
                    if s.silhouette && (b[3] == 0 || c[3].abs_diff(b[3]) > 16) {
                        edge = true;
                    }
                    if b[3] > 0 {
                        if s.luminance && (luma(c) - luma(b)).abs() >= s.luma_threshold {
                            edge = true;
                        }
                        if let Some(j) = other
                            && let (Some(a), Some(b)) = (
                                f.geometry.get(i).copied().flatten(),
                                f.geometry.get(j).copied().flatten(),
                            )
                        {
                            if s.depth
                                && (a.depth - b.depth).abs() / a.depth.min(b.depth).max(1e-6)
                                    >= s.depth_threshold
                            {
                                edge = true;
                            }
                            if s.normals && dot(a.normal, b.normal).clamp(-1., 1.) < cosine {
                                edge = true;
                            }
                        }
                    }
                }
            }
            if edge {
                lines[i] = [0, 0, 0, a];
                edges += 1;
            }
        }
    }
    Ok((lines, tones, edges, visible))
}
fn append(
    d: &Document,
    state: &mut DocumentState,
    pixels: Vec<[u8; 4]>,
    name: &str,
) -> Result<Option<u64>, String> {
    if pixels.iter().all(|p| p[3] == 0) {
        return Ok(None);
    }
    if state.layers.len() >= 256 {
        return Err("Output exceeds raster layer limit256".into());
    }
    state.scene.layer_ids.initialize(state.layers.len())?;
    state.scene.paper.roots = state.scene.paper.root_nodes(state.layers.len());
    let n = state.layers.len();
    state.scene.paper.roots.push(crate::paper::Node::Raster(n));
    let extent = PlacedRaster {
        width: d.width,
        height: d.height,
        offset: [0, 0],
    };
    let mut layer = RasterLayer {
        name: name.into(),
        pixels,
        precise: None,
        placement: Some(extent),
        locked: false,
    };
    if d.color != crate::color_storage::Config::legacy() {
        let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
        let samples = layer
            .pixels
            .iter()
            .map(|p| space.from_srgb(crate::color_precision::Sample::from8(*p)))
            .collect::<Result<Vec<_>, _>>()?;
        crate::color_storage::set_samples(&mut layer, &d.color, extent, samples)?;
    }
    state.layers.push(layer);
    state.scene.layer_ids.append(state.layers.len())?;
    let id = state.scene.layer_ids.id(n);
    state
        .scene
        .render_order
        .entries
        .push(crate::render_order::Entry {
            key: Key::Layer(Target::Raster(id)),
            group: crate::render_order::Group::World,
        });
    Ok(Some(id))
}
pub fn prepare(
    d: &Document,
    s: Settings,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if !s.valid() {
        return Err("Invalid contour / tone settings".into());
    }
    if let Input::Image(id) = s.input
        && crate::layers::index(d, id).is_none()
    {
        return Err("Select an existing raster image layer".into());
    }
    if s.input == Input::Scene
        && (d.scene.camera.is_none()
            || !d
                .scene
                .world3d
                .surfaces
                .values()
                .any(|s| !s.triangles.is_empty()))
    {
        return Err("The3D scene needs a camera and polygon faces".into());
    }
    let n = d.width as usize * d.height as usize;
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    let work = n.saturating_mul(if s.input == Input::Scene {
        d.scene.worlds.len().max(1)
    } else {
        1
    });
    let memory = n.saturating_mul(if s.input == Input::Scene { 128 } else { 80 });
    if d.memory_bytes().saturating_mul(3) > budget
        || s.strength > 0.
            && (n > 4 * 1024 * 1024
                || work > 32_000_000
                || d.memory_bytes()
                    .saturating_mul(3)
                    .saturating_add(memory)
                    .saturating_add(triangles * 1024)
                    > budget)
    {
        return Err("Line / tone frame, ray-work or memory budget exceeded".into());
    }
    let mut state = d.snapshot();
    let mut added = vec![];
    let mut warnings = vec![];
    let (mut edge_pixels, mut visible_pixels) = (0, 0);
    if s.strength > 0. {
        let source = frame(d, s.input, cancel)?;
        let (lines, tones, edges, visible) =
            analyze(&source, d.width as usize, d.height as usize, s, cancel)?;
        edge_pixels = edges;
        visible_pixels = visible;
        drop(source);
        state.scene.render_order.entries = crate::render_order::normalized(&state.scene);
        if s.tones
            && let Some(id) = append(d, &mut state, tones, "Extracted stepped tones")?
        {
            added.push(id);
        }
        if s.lines
            && let Some(id) = append(d, &mut state, lines, "Extracted contours")?
        {
            added.push(id);
        }
        if visible > 0 && edges * 4 > visible {
            warnings.push(format!("Dense contour result: {edges}/{visible} visible pixels are ink; raise thresholds or lower radius before adopting"));
        }
    }
    if added.is_empty() {
        state = d.snapshot();
    }
    crate::book_io::check_cancel(cancel)?;
    if !added.is_empty() {
        crate::layer_catalog::reconcile(d, &mut state)?;
        crate::layer_groups::finish(d, &state)?;
    }
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Derived layers exceed result / Undo memory budget".into());
    }
    let mut thumbnail = vec![];
    if !added.is_empty() {
        let mut frozen = crate::layer_merge::preview(&state, d.current_frame)?;
        frozen.keyframe_content = d.keyframe_content;
        let renderer = crate::render::Renderer::new(&frozen);
        if let Some(e) = renderer.error() {
            return Err(e.into());
        }
        for y in 0..128 {
            crate::book_io::check_cancel(cancel)?;
            for x in 0..128 {
                thumbnail.extend(renderer.pixel([
                    (x as f64 + 0.5) * d.width as f64 / 128.,
                    (y as f64 + 0.5) * d.height as f64 / 128.,
                ]));
            }
        }
    }
    Ok(Prepared {
        state,
        id: d.id,
        revision: d.revision,
        frame: d.current_frame,
        summary: format!(
            "{} new layers; {edge_pixels}/{visible_pixels} visible pixels outlined; source content retained",
            added.len()
        ),
        added,
        warnings,
        thumbnail,
        edge_pixels,
        visible_pixels,
    })
}
