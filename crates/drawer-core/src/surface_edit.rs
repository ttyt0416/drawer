//! Original local primitive / surface editing and frozen texture delivery.
use crate::{
    document::Document,
    model_import::{Image, Wrap},
    model_library::{self, Shape},
    model_mesh::Surface,
    world3d,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    #[default]
    Source,
    Wire,
    Face,
    Both,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Primitive {
    pub shape: Shape,
    pub detail: u16,
}
impl Primitive {
    pub fn valid(&self) -> bool {
        if matches!(self.shape, Shape::Sphere | Shape::Cylinder | Shape::Cone) {
            (3..=64).contains(&self.detail)
        } else {
            (1..=32).contains(&self.detail)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uv {
    pub offset: [f64; 2],
    pub scale: [f64; 2],
    pub angle: f64,
    pub wrap: [Option<Wrap>; 2],
}
impl Default for Uv {
    fn default() -> Self {
        Self {
            offset: [0.; 2],
            scale: [1.; 2],
            angle: 0.,
            wrap: [None; 2],
        }
    }
}
impl Uv {
    pub fn valid(&self) -> bool {
        self.offset
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 10000.)
            && self
                .scale
                .iter()
                .all(|v| v.is_finite() && (0.001..=128.).contains(&v.abs()))
            && self.angle.is_finite()
            && self.angle.abs() <= 36000.
    }
    pub fn map(&self, p: [f64; 2]) -> [f64; 2] {
        if self.offset == [0.; 2] && self.scale == [1.; 2] && self.angle == 0. {
            return p;
        }
        let (s, c) = self.angle.to_radians().sin_cos();
        let x = (p[0] - 0.5) * self.scale[0];
        let y = (p[1] - 0.5) * self.scale[1];
        [
            c * x - s * y + 0.5 + self.offset[0],
            s * x + c * y + 0.5 + self.offset[1],
        ]
    }
}
pub fn wrap(value: f64, mode: Wrap) -> f64 {
    match mode {
        Wrap::Repeat => value.rem_euclid(1.),
        Wrap::Clamp => value.clamp(0., 1.),
        Wrap::Mirror => {
            let p = value.rem_euclid(2.);
            if p > 1. { 2. - p } else { p }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedImage {
    pub image: Image,
    pub source: String,
    pub name: String,
}
impl LinkedImage {
    fn load(path: &Path) -> Result<Self, String> {
        let path = crate::model_import::local_path(path)?;
        let data = crate::book_io::read_local(&path, 8 * 1024 * 1024)?;
        let image = Image::from_bytes(&data)?;
        let source = path
            .to_str()
            .ok_or("Choose a UTF-8 local texture path")?
            .to_owned();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let out = Self {
            image,
            source,
            name,
        };
        if !out.valid() {
            return Err("Invalid local texture reference".into());
        }
        Ok(out)
    }
    fn valid(&self) -> bool {
        self.source.len() <= 4096
            && !self.source.chars().any(char::is_control)
            && Path::new(&self.source).is_absolute()
            && crate::font_library::local(Path::new(&self.source))
            && !self.name.is_empty()
            && self.name.len() <= 256
            && !self
                .name
                .chars()
                .any(|c| c.is_control() || "/\\:".contains(c))
            && self.image.decode().is_ok()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub uv: Uv,
    pub diffuse: Option<LinkedImage>,
    pub normal: Option<LinkedImage>,
    pub alpha: Option<crate::model_import::Alpha>,
    pub cutoff: f64,
    pub normal_strength: f64,
    pub flip_green: bool,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            uv: Default::default(),
            diffuse: None,
            normal: None,
            alpha: None,
            cutoff: 0.5,
            normal_strength: 1.,
            flip_green: false,
        }
    }
}
impl Material {
    pub fn valid(&self) -> bool {
        self.cutoff.is_finite()
            && (0. ..=1.).contains(&self.cutoff)
            && self.uv.valid()
            && self.diffuse.as_ref().is_none_or(LinkedImage::valid)
            && self.normal.as_ref().is_none_or(LinkedImage::valid)
            && self.normal_strength.is_finite()
            && (0. ..=4.).contains(&self.normal_strength)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub primitive: Option<Primitive>,
    pub display: Display,
    pub materials: BTreeMap<u16, Material>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn active(&self) -> bool {
        self.display != Display::Source || !self.materials.is_empty()
    }
    pub fn valid(&self, s: &Surface) -> bool {
        self.primitive.as_ref().is_none_or(Primitive::valid)
            && self.materials.len() <= 256
            && self
                .materials
                .iter()
                .all(|(i, m)| (*i as usize) < s.materials.len() && m.valid())
    }
    pub fn bytes(&self) -> usize {
        self.materials
            .values()
            .map(|m| {
                256 + m
                    .diffuse
                    .as_ref()
                    .map_or(0, |i| i.source.len() + i.name.len() + i.image.bytes())
                    + m.normal
                        .as_ref()
                        .map_or(0, |i| i.source.len() + i.name.len() + i.image.bytes())
            })
            .sum()
    }
}
#[derive(Clone, Debug)]
pub enum Change {
    Material {
        material: u16,
        alpha: Option<crate::model_import::Alpha>,
        cutoff: f64,
    },
    GenerateUv,
    ClearTextures,
    Subdivide(u16),
    Display(Display),
    Uv {
        material: u16,
        value: Uv,
    },
    Load {
        material: u16,
        normal: bool,
        path: PathBuf,
    },
    Reload {
        material: u16,
        normal: bool,
    },
    Remove {
        material: u16,
        normal: bool,
    },
    Normal {
        material: u16,
        strength: f64,
        flip_green: bool,
    },
}
fn grid(
    mut model: crate::model_mesh::Model,
    n: u16,
    cancel: &AtomicBool,
) -> Result<crate::model_mesh::Model, String> {
    let n = n as usize;
    let faces = model.surface.triangles.len();
    let vertices = faces.saturating_mul((n + 1) * (n + 2) / 2);
    let triangles = faces.saturating_mul(n * n);
    if vertices > 65536 || triangles > 131072 || faces.saturating_mul(3 * n * (n + 1) / 2) > 65536 {
        return Err("Primitive subdivision exceeds65536 vertices/edges or131072 faces".into());
    }
    let old = model.mesh.vertices.clone();
    let old_faces = model.surface.triangles.clone();
    model.mesh.vertices.clear();
    model.mesh.edges.clear();
    model.surface.triangles.clear();
    let mut edges = BTreeSet::new();
    for t in old_faces {
        crate::book_io::check_cancel(cancel)?;
        let a = old[t.v[0] as usize];
        let b = old[t.v[1] as usize];
        let c = old[t.v[2] as usize];
        let start = model.mesh.vertices.len();
        let mut rows = vec![];
        for i in 0..=n {
            rows.push(model.mesh.vertices.len() - start);
            for j in 0..=n - i {
                model.mesh.vertices.push(std::array::from_fn(|axis| {
                    (a[axis] * (n - i - j) as f64 + b[axis] * i as f64 + c[axis] * j as f64)
                        / n as f64
                }));
            }
        }
        for i in 0..n {
            for j in 0..n - i {
                let a = start + rows[i] + j;
                let b = start + rows[i + 1] + j;
                let c = a + 1;
                let mut emit = |v: [usize; 3]| {
                    for k in 0..3 {
                        let x = v[k];
                        let y = v[(k + 1) % 3];
                        edges.insert([x.min(y), x.max(y)]);
                    }
                    let mut face = t.clone();
                    face.v = v.map(|x| x as u32);
                    face.n = None;
                    face.t = None;
                    model.surface.triangles.push(face);
                };
                emit([a, b, c]);
                if j + 1 < n - i {
                    emit([c, b, b + 1]);
                }
            }
        }
    }
    model.mesh.edges = edges.into_iter().collect();
    Ok(model)
}
pub fn prepare(
    d: &Document,
    id: u64,
    change: Change,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(20 * 1024 * 1024)
        > budget
    {
        return Err("Surface snapshot / image budget exceeded".into());
    }
    model_library::editable(d, &BTreeSet::from([id]))?;
    let mut state = d.snapshot();
    let index = state
        .scene
        .catalog
        .worlds
        .index(id, state.scene.worlds.len())
        .ok_or("Choose a model")?;
    let node = world3d::node(&state.scene, id).ok_or("Missing model axes")?;
    let frame = node.frame;
    let surface = state
        .scene
        .world3d
        .surfaces
        .get_mut(&id)
        .ok_or("Choose a surface model")?;
    match change {
        Change::Subdivide(detail) => {
            if d.scene.world3d.rigs.contains_key(&id)
                || d.scene.world3d.editing.morphs.contains_key(&id)
            {
                return Err("Detach rig / shape keys before rebuilding primitive topology".into());
            }
            let mut spec = surface.tools.primitive.clone().ok_or(
                "Only original primitives have editable subdivision; imported topology is retained",
            )?;
            spec.detail = detail;
            if !spec.valid() {
                return Err(
                    "Round subdivisions3..64; planar/box/chair/room1..32; zero is invalid".into(),
                );
            }
            if surface.paint.is_some()
                || !surface.tools.materials.is_empty()
                || surface.imported.is_some()
            {
                return Err(
                    "Remove painted/linked materials before rebuilding primitive UV topology"
                        .into(),
                );
            }
            let rounded = matches!(spec.shape, Shape::Sphere | Shape::Cylinder | Shape::Cone);
            let mut m =
                model_library::primitive(spec.shape, if rounded { detail } else { 3 }, cancel)?
                    .models
                    .remove(0);
            if !rounded && detail > 1 {
                m = grid(m, detail, cancel)?;
            }
            let center = world3d::center(&m.mesh);
            for p in &mut m.mesh.vertices {
                *p = frame.map(std::array::from_fn(|i| p[i] - center[i]));
            }
            m.mesh.color = state.scene.worlds[index].color;
            m.surface.materials = surface.materials.clone();
            m.surface.tools = surface.tools.clone();
            m.surface.tools.primitive = Some(spec);
            state.scene.worlds[index] = m.mesh;
            state.scene.world3d.surfaces.insert(id, m.surface);
        }
        Change::Display(mode) => surface.tools.display = mode,
        Change::ClearTextures => {
            surface.paint = None;
            surface.tools.materials.clear();
        }
        Change::GenerateUv => {
            if surface.triangles.len().saturating_mul(3) > 65536 {
                return Err("Generated UV chart exceeds65536 coordinates".into());
            }
            let positions = state.scene.worlds[index]
                .vertices
                .iter()
                .map(|p| frame.inverse().map(*p))
                .collect::<Vec<_>>();
            let lo = std::array::from_fn::<_, 3, _>(|a| {
                positions.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min)
            });
            let hi = std::array::from_fn::<_, 3, _>(|a| {
                positions
                    .iter()
                    .map(|p| p[a])
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            surface.texcoords.clear();
            for t in &mut surface.triangles {
                crate::book_io::check_cancel(cancel)?;
                let p = t.v.map(|i| positions[i as usize]);
                let n = crate::world_math::cross(
                    crate::world_math::sub(p[1], p[0]),
                    crate::world_math::sub(p[2], p[0]),
                );
                let skip = (0..3)
                    .max_by(|a, b| n[*a].abs().total_cmp(&n[*b].abs()))
                    .unwrap();
                let axes = (0..3).filter(|a| *a != skip).collect::<Vec<_>>();
                let start = surface.texcoords.len() as u32;
                surface.texcoords.extend(p.map(|p| {
                    [
                        if hi[axes[0]] > lo[axes[0]] {
                            (p[axes[0]] - lo[axes[0]]) / (hi[axes[0]] - lo[axes[0]])
                        } else {
                            0.5
                        },
                        if hi[axes[1]] > lo[axes[1]] {
                            (p[axes[1]] - lo[axes[1]]) / (hi[axes[1]] - lo[axes[1]])
                        } else {
                            0.5
                        },
                        0.,
                    ]
                }));
                t.t = Some([start, start + 1, start + 2]);
            }
        }
        other => {
            let material = match &other {
                Change::Uv { material, .. }
                | Change::Load { material, .. }
                | Change::Reload { material, .. }
                | Change::Remove { material, .. }
                | Change::Normal { material, .. }
                | Change::Material { material, .. } => *material,
                _ => unreachable!(),
            };
            if material as usize >= surface.materials.len() {
                return Err("Missing material slot".into());
            }
            if surface
                .triangles
                .iter()
                .filter(|t| t.material == material)
                .any(|t| t.t.is_none())
                && !matches!(other, Change::Remove { .. })
            {
                return Err(
                    "This material needs UVs; prepare UV coordinates in uv-paint before mapping"
                        .into(),
                );
            }
            let m = surface.tools.materials.entry(material).or_default();
            match other {
                Change::Uv { value, .. } => m.uv = value,
                Change::Material { alpha, cutoff, .. } => {
                    m.alpha = alpha;
                    m.cutoff = cutoff;
                }
                Change::Load { normal, path, .. } => {
                    let img = LinkedImage::load(&path)?;
                    if normal {
                        m.normal = Some(img);
                    } else {
                        m.diffuse = Some(img);
                        surface.paint = None;
                    }
                }
                Change::Reload { normal, .. } => {
                    let old = if normal { &m.normal } else { &m.diffuse };
                    let path=old.as_ref().ok_or("No local link: assign a local image first; imported snapshots remain embedded")?.source.clone();
                    let img = LinkedImage::load(Path::new(&path))?;
                    if normal {
                        m.normal = Some(img);
                    } else {
                        m.diffuse = Some(img);
                    }
                }
                Change::Remove { normal, .. } => {
                    if normal {
                        m.normal = None;
                    } else {
                        m.diffuse = None;
                    }
                }
                Change::Normal {
                    strength,
                    flip_green,
                    ..
                } => {
                    m.normal_strength = strength;
                    m.flip_green = flip_green;
                }
                _ => unreachable!(),
            }
        }
    }
    let tools = &state.scene.world3d.surfaces[&id].tools;
    if state.scene.world3d.lighting.is_none()
        && (tools.display != Display::Source
            || tools
                .materials
                .values()
                .any(|m| m.diffuse.is_some() || m.normal.is_some()))
    {
        let style = if tools.display == Display::Wire {
            crate::lighting::Style::Wire
        } else {
            crate::lighting::Style::Unlit
        };
        state.scene.world3d.lighting = Some(crate::lighting::Settings {
            style,
            ..Default::default()
        });
        let (c, mut lens) = world3d::default_camera(d);
        if d.scene.camera.is_some() && d.scene.world3d.lens.is_none() {
            lens.projection = world3d::Projection::Orthographic;
        }
        state.scene.camera = Some(c);
        state.scene.world3d.lens = Some(lens);
    }
    crate::model_edit::sync(d, &mut state.scene)?;
    if state
        .memory_bytes()
        .saturating_mul(3)
        .saturating_add(8 * 1024 * 1024)
        > budget
    {
        return Err("Surface / Undo / decoded map exceeds budget".into());
    }
    let frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&frozen).map_err(|e| e.to_string())?;
    crate::book_io::check_cancel(cancel)?;
    world3d::prepared(
        d,
        state,
        "Primitive divisions / surface maps and display".into(),
        vec![id],
        vec![],
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Fail,
    Skip,
    Rename,
}
pub struct Export {
    files: Vec<(String, Vec<u8>)>,
    sources: BTreeSet<PathBuf>,
}
pub struct Report {
    pub folder: PathBuf,
    pub files: usize,
    pub skipped: usize,
}
fn png(image: &Image) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let pixels = image.decode()?;
    let mut output = vec![];
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            &pixels,
            image.width as u32,
            image.height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(output)
}
fn name(raw: &str) -> String {
    let stem = Path::new(raw)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let clean = stem
        .chars()
        .take(100)
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!(
        "texture-{}.png",
        if clean.is_empty() { "image" } else { &clean }
    )
}
impl Export {
    pub fn capture(d: &Document, budget: usize, cancel: &AtomicBool) -> Result<Self, String> {
        crate::book_io::check_cancel(cancel)?;
        if d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(8 * 1024 * 1024)
            > budget
        {
            return Err("Texture export snapshot exceeds memory budget".into());
        }
        let mut files = vec![];
        let mut sources = BTreeSet::new();
        let mut bytes = 0usize;
        for (id, s) in &d.scene.world3d.surfaces {
            let used = s
                .triangles
                .iter()
                .map(|t| t.material)
                .collect::<BTreeSet<_>>();
            if used.is_empty() {
                continue;
            }
            if let Some(p) = &s.paint {
                let data = p.png_bytes()?;
                bytes = bytes.saturating_add(data.len());
                files.push((format!("model-{id}-paint.png"), data));
            }
            for material in used {
                crate::book_io::check_cancel(cancel)?;
                let custom = s.tools.materials.get(&material);
                if let Some(image) = custom.and_then(|m| m.diffuse.as_ref()) {
                    let data = png(&image.image)?;
                    bytes = bytes.saturating_add(data.len());
                    sources.insert(PathBuf::from(&image.source));
                    files.push((name(&image.name), data));
                } else if let Some(image) = s
                    .imported
                    .as_ref()
                    .and_then(|a| a.materials.get(&material))
                    .and_then(|m| m.image.as_ref())
                {
                    let data = png(image)?;
                    bytes = bytes.saturating_add(data.len());
                    files.push((name(&s.materials[material as usize].name), data));
                }
                if let Some(image) = custom.and_then(|m| m.normal.as_ref()) {
                    let data = png(&image.image)?;
                    bytes = bytes.saturating_add(data.len());
                    sources.insert(PathBuf::from(&image.source));
                    files.push((name(&image.name), data));
                }
                if files.len() > 1024
                    || bytes > 128 * 1024 * 1024
                    || bytes.saturating_mul(3).saturating_add(d.memory_bytes()) > budget
                {
                    return Err("Texture export exceeds1024 files /128MiB /memory budget".into());
                }
            }
        }
        if files.is_empty() {
            return Err("No assigned face texture / normal map / painted image to export".into());
        }
        Ok(Self { files, sources })
    }
    pub fn write(
        &self,
        destination: &Path,
        policy: Conflict,
        cancel: &AtomicBool,
    ) -> Result<Report, String> {
        self.write_with(destination, policy, cancel, || {})
    }
    pub(crate) fn write_with(
        &self,
        destination: &Path,
        policy: Conflict,
        cancel: &AtomicBool,
        mut before_publish: impl FnMut(),
    ) -> Result<Report, String> {
        use std::{fs, io::Write};
        crate::book_io::check_cancel(cancel)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !parent.is_dir() || !crate::font_library::local(&parent) {
            return Err("Choose a local texture export parent".into());
        }
        let leaf = destination
            .file_name()
            .filter(|n| !n.is_empty())
            .ok_or("Choose a new output folder name")?;
        let mut output = parent.join(leaf);
        if self.sources.contains(&output) {
            return Err("Preserve the original image file".into());
        }
        if fs::symlink_metadata(&output).is_ok() {
            match policy {
                Conflict::Fail => {
                    return Err("Output exists; no folder merging or replacement".into());
                }
                Conflict::Skip => {
                    return Ok(Report {
                        folder: output,
                        files: 0,
                        skipped: self.files.len(),
                    });
                }
                Conflict::Rename => {
                    let stem = leaf.to_string_lossy();
                    let mut found = None;
                    for i in 2..=10000 {
                        let p = parent.join(format!("{stem}-{i}"));
                        if fs::symlink_metadata(&p).is_err() {
                            found = Some(p);
                            break;
                        }
                    }
                    output = found.ok_or("No free export folder name")?;
                }
            }
        }
        let mut names = BTreeSet::new();
        let mut planned = vec![];
        let mut skipped = 0;
        for (file, data) in &self.files {
            let mut file = file.clone();
            if !names.insert(file.to_lowercase()) {
                match policy {
                    Conflict::Fail => {
                        return Err("Duplicate texture names: choose Skip or Rename".into());
                    }
                    Conflict::Skip => {
                        skipped += 1;
                        continue;
                    }
                    Conflict::Rename => {
                        let stem = Path::new(&file).file_stem().unwrap().to_string_lossy();
                        let mut found = None;
                        for i in 2..=10000 {
                            let candidate = format!("{stem}-{i}.png");
                            if names.insert(candidate.to_lowercase()) {
                                found = Some(candidate);
                                break;
                            }
                        }
                        file = found.ok_or("No free texture name")?;
                    }
                }
            }
            planned.push((file, data));
        }
        let mut stage = Stage::new(&parent)?;
        for (file, data) in &planned {
            crate::book_io::check_cancel(cancel)?;
            let p = stage.root.join(file);
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .map_err(|e| e.to_string())?;
            stage.files.push(p);
            f.write_all(data)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        before_publish();
        crate::book_io::check_cancel(cancel)?;
        fs::create_dir(&output)
            .map_err(|e| format!("Output appeared during preparation; originals preserved: {e}"))?;
        let mut published = vec![];
        for (file, data) in &planned {
            let target = output.join(file);
            if let Err(e) = fs::hard_link(stage.root.join(file), &target) {
                for (p, bytes) in &published {
                    if fs::read(p).ok().as_deref() == Some(*bytes) {
                        let _ = fs::remove_file(p);
                    }
                }
                let _ = fs::remove_dir(&output);
                return Err(format!(
                    "Texture publication failed; changed/foreign files retained at{}: {e}",
                    output.display()
                ));
            }
            published.push((target, data.as_slice()));
        }
        Ok(Report {
            folder: output,
            files: planned.len(),
            skipped,
        })
    }
}
struct Stage {
    root: PathBuf,
    files: Vec<PathBuf>,
}
impl Stage {
    fn new(parent: &Path) -> Result<Self, String> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        for _ in 0..100 {
            let root = parent.join(format!(
                ".drawer-textures-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => {
                    return Ok(Self {
                        root,
                        files: vec![],
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Cannot create private texture staging".into())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        for p in &self.files {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}
