//! Explicit local file refreshes. Native documents retain their last successful cache.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_search::Key,
    material_layer::{Image, Layer, Source},
    scene::{Affine, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    #[serde(
        default,
        skip_serializing_if = "crate::file_sources::Selection::is_default"
    )]
    pub selection: crate::file_sources::Selection,
    pub target: Key,
    pub path: PathBuf,
    pub cache: Layer,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub entries: Vec<Link>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn valid(&self, missing: bool) -> bool {
        self.entries.len() <= 256
            && self.entries.iter().all(|l| {
                l.selection.valid()
                    && local_path(&l.path).is_ok()
                    && l.cache.valid(missing)
                    && matches!(l.cache.source, Source::Image(_))
                    && match l.target {
                        Key::Layer(
                            crate::layer_properties::Target::Raster(n)
                            | crate::layer_properties::Target::Vector(n),
                        ) => n > 0,
                        Key::Layer(crate::layer_properties::Target::Folder(n)) => n < 256,
                        Key::Track(n) | Key::Object(n) | Key::Gradient(n) | Key::World(n) => n > 0,
                        Key::Cell { track, cell } => track > 0 && cell > 0,
                        _ => false,
                    }
            })
            && self
                .entries
                .iter()
                .map(|l| l.target)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn get(&self, key: Key) -> Option<&Link> {
        self.entries.iter().find(|l| l.target == key)
    }
    pub fn pixel(&self, key: Key, original: [f64; 2]) -> Option<[u8; 4]> {
        self.get(key).map(|l| l.cache.pixel(original))
    }
    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|l| l.cache.bytes() + l.path.as_os_str().len() * 2 + std::mem::size_of::<Link>())
            .sum()
    }
}
/// Reject network locations and schemes before touching the filesystem.
pub fn local_path(path: &Path) -> Result<(), String> {
    let s = path.to_str().ok_or("File path is not Unicode")?;
    let normal = s.strip_prefix(r"\\?\").unwrap_or(s);
    let bytes = normal.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return Err("Choose a local drive path".into());
    }
    if s.len() > 4096
        || s.is_empty()
        || normal.starts_with(r"\")
        || normal.starts_with("//")
        || normal.starts_with("UNC\\")
        || normal.contains("://")
        || !path.is_absolute()
        || normal.chars().any(char::is_control)
    {
        return Err(
            "Choose an absolute local disk path; URLs and network shares are unsupported".into(),
        );
    }
    Ok(())
}
pub fn source_folder(path: &Path) -> Result<PathBuf, String> {
    local_path(path)?;
    let parent = path
        .parent()
        .ok_or("Missing source folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    local_path(&parent)?;
    if !parent.is_dir() {
        return Err("Source folder is unavailable".into());
    }
    Ok(parent)
}
pub fn read(path: &Path, owner: Option<&Path>) -> Result<(PathBuf, Image), String> {
    let snapshot = crate::file_sources::read(path, owner, &Default::default())?;
    Ok((snapshot.path, snapshot.image))
}
pub fn refresh(d: &Document, key: Key, path: Option<&Path>) -> Result<DocumentState, String> {
    let selection = d
        .scene
        .file_objects
        .get(key)
        .ok_or("Choose a linked file layer")?
        .selection
        .clone();
    let prepared =
        crate::file_sources::refresh(d, &[(key, path.map(Path::to_path_buf), selection)])?;
    if let Some((_, error)) = prepared.failed.first() {
        return Err(error.clone());
    }
    Ok(prepared.state)
}
pub fn restore_original(d: &Document, key: Key) -> Result<DocumentState, String> {
    let t = crate::layer_catalog::target(d, key).ok_or("Missing linked layer")?;
    if d.scene.file_objects.get(key).is_none() {
        return Err("Choose a linked layer".into());
    }
    if crate::layer_properties::inherited(d, t).locked
        || crate::layer_properties::properties(d, t).pixels_locked
    {
        return Err("Unlock the linked layer first".into());
    }
    let mut state = d.snapshot();
    state.scene.file_objects.entries.retain(|l| l.target != key);
    validate(d, &state)?;
    Ok(state)
}
pub(crate) fn duplicate(s: &mut DocumentState, old: Key, new: Key) {
    if let Some(mut link) = s.scene.file_objects.get(old).cloned() {
        link.target = new;
        s.scene.file_objects.entries.push(link);
    }
}
/// Rendering/baking may consume a linked cache; stored user locks still apply.
pub(crate) fn stored_flags(
    d: &Document,
    t: crate::layer_properties::Target,
) -> crate::layer_properties::Properties {
    use crate::{layer_properties as p, paper::Node};
    let mut value = p::inherited(d, t);
    let own = p::properties(d, t);
    value.locked = own.locked;
    value.pixels_locked = own.pixels_locked;
    let mut node = match t {
        p::Target::Raster(id) => crate::layers::index(d, id).map(Node::Raster),
        p::Target::Folder(n) => Some(Node::Folder(n as usize)),
        _ => None,
    };
    for _ in 0..16 {
        let Some(parent) = node.and_then(|n| {
            d.scene
                .paper
                .folders
                .iter()
                .position(|f| f.children.contains(&n))
        }) else {
            break;
        };
        let q = p::properties(d, p::Target::Folder(parent as u16));
        value.locked |= q.locked;
        value.pixels_locked |= q.pixels_locked;
        node = Some(Node::Folder(parent));
    }
    if let p::Target::Cell { track, .. } = t {
        let q = p::properties(d, p::Target::Track(track));
        value.locked |= q.locked;
        value.pixels_locked |= q.pixels_locked;
    }
    value
}
pub(crate) fn validation_flags(
    d: &Document,
    after: &crate::scene::Scene,
    t: crate::layer_properties::Target,
) -> crate::layer_properties::Properties {
    use crate::{layer_properties as p, paper::Node};
    let mut keys = vec![crate::layer_catalog::key(d, t)];
    let mut node = match t {
        p::Target::Raster(id) => crate::layers::index(d, id).map(Node::Raster),
        p::Target::Folder(n) => Some(Node::Folder(n as usize)),
        _ => None,
    };
    for _ in 0..16 {
        let Some(parent) = node.and_then(|n| {
            d.scene
                .paper
                .folders
                .iter()
                .position(|f| f.children.contains(&n))
        }) else {
            break;
        };
        keys.push(Key::Layer(p::Target::Folder(parent as u16)));
        node = Some(Node::Folder(parent));
    }
    if let p::Target::Cell { track, .. } = t {
        keys.push(crate::layer_catalog::key(d, p::Target::Track(track)));
    }
    let linked = keys
        .into_iter()
        .filter(|k| d.scene.file_objects.get(*k).is_some())
        .collect::<Vec<_>>();
    if !linked.is_empty() && linked.iter().all(|k| after.file_objects.get(*k).is_none()) {
        stored_flags(d, t)
    } else {
        p::inherited(d, t)
    }
}
pub(crate) fn validate_consumed(
    d: &Document,
    targets: &[crate::layer_properties::Target],
) -> Result<(), String> {
    use crate::{layer_properties as p, paper::Node};
    for t in targets {
        let mut owners = vec![*t];
        let mut node = match t {
            p::Target::Raster(id) => crate::layers::index(d, *id).map(Node::Raster),
            p::Target::Folder(n) => Some(Node::Folder(*n as usize)),
            _ => None,
        };
        for _ in 0..16 {
            let Some(parent) = node.and_then(|n| {
                d.scene
                    .paper
                    .folders
                    .iter()
                    .position(|f| f.children.contains(&n))
            }) else {
                break;
            };
            owners.push(p::Target::Folder(parent as u16));
            node = Some(Node::Folder(parent));
        }
        if let p::Target::Cell { track, .. } = t {
            owners.push(p::Target::Track(*track));
        }
        if owners.into_iter().any(|owner| {
            d.scene
                .file_objects
                .get(crate::layer_catalog::key(d, owner))
                .is_some()
                && !targets.contains(&owner)
        }) {
            return Err(
                "Consume the complete linked folder/track, or restore its original content first"
                    .into(),
            );
        }
    }
    Ok(())
}
pub(crate) fn render_extents(
    d: &Document,
    g: &crate::paper::Graph,
    nodes: &[crate::paper::Node],
    appearance: &crate::appearance::Caches,
) -> Result<Vec<crate::document::PlacedRaster>, String> {
    fn visit(
        d: &Document,
        g: &crate::paper::Graph,
        node: crate::paper::Node,
        a: &crate::appearance::Caches,
        out: &mut Vec<crate::document::PlacedRaster>,
        depth: usize,
    ) -> Result<(), String> {
        use crate::paper::Node;
        if depth > 16 {
            return Err("File layer tree depth limit".into());
        }
        if let Some(l) = d
            .scene
            .file_objects
            .get(Key::Layer(crate::clipping::node_key(d, node)))
        {
            let mut layer = l.cache.clone();
            layer.transform = layer.transform.then(d.scene.transform);
            let b = layer.bounds();
            let origin = b.min.map(f64::floor);
            let size = [0, 1].map(|n| b.max[n].ceil() - origin[n]);
            if size
                .iter()
                .any(|n| !n.is_finite() || !(1. ..=32768.).contains(n))
                || origin.iter().any(|n| n.abs() > 1e9)
            {
                return Err("File layer output extent limit".into());
            }
            out.push(crate::document::PlacedRaster {
                width: size[0] as u32,
                height: size[1] as u32,
                offset: origin.map(|n| n as i64),
            });
            return Ok(());
        }
        match node {
            Node::Raster(n) => {
                let layer = d.layers.get(n).ok_or("Missing raster")?;
                if !layer.valid(d.width, d.height) {
                    return Err("Invalid raster".into());
                }
                let base = a.rasters[n]
                    .as_ref()
                    .map_or_else(|| layer.extent(d.width, d.height), |c| c.extent);
                if let Some(stack) = d
                    .scene
                    .surface_effects
                    .rasters
                    .get(&d.scene.layer_ids.id(n))
                {
                    out.push(crate::surface_effects::extent(base, stack)?);
                    return Ok(());
                }
                out.push(
                    a.rasters[n]
                        .as_ref()
                        .map_or_else(|| layer.extent(d.width, d.height), |c| c.extent),
                );
            }
            Node::Folder(n) => {
                for child in &g.folders[n].children {
                    visit(d, g, *child, a, out, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    let mut out = vec![];
    for node in nodes {
        visit(d, g, *node, appearance, &mut out, 0)?;
    }
    Ok(out)
}
pub fn create(d: &Document, path: &Path, name: &str) -> Result<(DocumentState, u64), String> {
    let (state, id, _) = crate::file_sources::create(d, path, name, Default::default())?;
    Ok((state, id))
}
pub fn validate(d: &Document, s: &DocumentState) -> Result<(), String> {
    for old in &d.scene.file_objects.entries {
        if let Some(t) = crate::layer_catalog::target(d, old.target)
            && let Some(next) = s.scene.file_objects.get(old.target)
        {
            let flags = crate::layer_properties::inherited(d, t);
            let content_changed = old.path != next.path
                || old.selection != next.selection
                || old.cache.source != next.cache.source;
            if old != next && (flags.locked || content_changed && stored_flags(d, t).pixels_locked)
            {
                return Err("The linked layer is locked".into());
            }
            if flags.position_locked
                && (old.cache.transform != next.cache.transform
                    || old.cache.area != next.cache.area
                    || old.cache.mapping.warp != next.cache.mapping.warp)
            {
                return Err("The linked layer position is locked".into());
            }
        }
    }
    if !s.scene.valid()
        || s.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&s.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("File object scene/Undo budget".into());
    }
    crate::layer_properties::validate_edit(d, s)
}
fn live(
    count: usize,
    scene: &crate::scene::Scene,
    animation: Option<&crate::animation::Timeline>,
    key: Key,
) -> bool {
    use crate::layer_properties::Target;
    match key {
        Key::Layer(Target::Raster(id)) => (0..count).any(|n| scene.layer_ids.id(n) == id),
        Key::Layer(Target::Vector(id)) => scene.lines.layers.iter().any(|l| l.id == id),
        Key::Layer(Target::Folder(n)) => scene.paper.folder_active(n as usize),
        Key::Object(id) => scene
            .catalog
            .objects
            .index(id, scene.objects.len())
            .is_some_and(|n| n < scene.objects.len()),
        Key::World(id) => scene
            .catalog
            .worlds
            .index(id, scene.worlds.len())
            .is_some_and(|n| n < scene.worlds.len()),
        Key::Gradient(id) => scene.gradients.layers.iter().any(|l| l.id == id),
        Key::Track(id) => animation.is_some_and(|a| {
            scene
                .catalog
                .tracks
                .index(id, a.tracks.len())
                .is_some_and(|n| n < a.tracks.len())
        }),
        Key::Cell { track, cell } => animation.is_some_and(|a| {
            scene
                .catalog
                .tracks
                .index(track, a.tracks.len())
                .and_then(|n| a.tracks.get(n))
                .is_some_and(|t| {
                    (0..t.cells.len()).any(|n| scene.catalog.cell_id(track, n) == cell)
                })
        }),
        _ => false,
    }
}
pub(crate) fn valid_live(d: &Document) -> bool {
    d.scene
        .file_objects
        .entries
        .iter()
        .all(|l| live(d.layers.len(), &d.scene, d.animation.as_ref(), l.target))
}
pub(crate) fn prune(s: &mut DocumentState) {
    let live = s
        .scene
        .file_objects
        .entries
        .iter()
        .filter(|l| live(s.layers.len(), &s.scene, s.animation.as_ref(), l.target))
        .map(|l| l.target)
        .collect::<std::collections::BTreeSet<_>>();
    s.scene
        .file_objects
        .entries
        .retain(|l| live.contains(&l.target));
}
pub(crate) fn shift(scene: &mut crate::scene::Scene, key: Key, delta: [f64; 2]) {
    let inverse = scene.transform.inverse();
    let d = [
        inverse.0[0] * delta[0] + inverse.0[2] * delta[1],
        inverse.0[1] * delta[0] + inverse.0[3] * delta[1],
    ];
    if let Some(link) = scene
        .file_objects
        .entries
        .iter_mut()
        .find(|l| l.target == key)
    {
        link.cache.transform.0[4] += d[0];
        link.cache.transform.0[5] += d[1];
    }
}

/// The outside file is written only after the page change has passed all preflight checks.
pub struct Conversion {
    pub state: DocumentState,
    pub external: Document,
    pub path: PathBuf,
    pub target: Key,
}
impl Conversion {
    pub fn save(&mut self, overwrite: bool) -> Result<(), String> {
        local_path(&self.path)?;
        if !self
            .path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("drawer"))
        {
            return Err("Choose a .drawer source document".into());
        }
        if self.path.exists() && !overwrite {
            return Err("Source destination already exists; confirm replacement first".into());
        }
        crate::files::save(&mut self.external, &self.path).map_err(|e| e.to_string())
    }
}
fn destination(d: &Document, path: &Path) -> Result<PathBuf, String> {
    local_path(path)?;
    let parent = path
        .parent()
        .ok_or("Missing destination folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    local_path(&parent)?;
    let path = parent.join(path.file_name().ok_or("Missing filename")?);
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("drawer"))
    {
        return Err("Choose a .drawer source document".into());
    }
    let actual = path.canonicalize().unwrap_or_else(|_| path.clone());
    if [d.path.as_ref(), d.source_path.as_ref()]
        .into_iter()
        .flatten()
        .any(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == actual)
    {
        return Err("The source destination would overwrite the current document".into());
    }
    if d.scene
        .file_objects
        .entries
        .iter()
        .any(|l| l.path == actual)
    {
        return Err("Choose a source path different from the page's existing links".into());
    }
    Ok(path)
}
fn cache(image: Image, origin: [f64; 2], transform: Affine) -> Layer {
    Layer {
        area: Rect {
            min: [0.; 2],
            max: [image.width as f64, image.height as f64],
        },
        source: Source::Image(image),
        transform: Affine([1., 0., 0., 1., origin[0], origin[1]]).then(transform.inverse()),
        repeat: [false; 2],
        mapping: Default::default(),
    }
}
pub fn convert(d: &Document, key: Key, path: &Path) -> Result<Conversion, String> {
    use crate::layer_properties::{self as p, Target};
    let path = destination(d, path)?;
    if let Key::Layer(crate::layer_properties::Target::Raster(id)) = key
        && d.scene.surface_effects.rasters.contains_key(&id)
    {
        return Err("Confirm surface effects as a separate raster result before converting to a linked file".into());
    }
    let t = crate::layer_catalog::target(d, key).ok_or("Select a canvas layer")?;
    let flags = p::inherited(d, t);
    if flags.locked
        || flags.pixels_locked
        || d.scene.clipping.solo.is_some()
        || matches!(key, Key::Selection(_))
        || crate::layer_effects::adjustment(d, t).is_some()
    {
        return Err("Unlock a content layer and leave folder solo before converting; correction layers need their backdrop".into());
    }
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || d.width as u64 * d.height as u64 > 4 * 1024 * 1024
    {
        return Err("File conversion snapshot/page limit".into());
    }
    let mut raw = d.snapshot();
    let mut props = p::properties(d, t);
    props.opacity = 100;
    props.hidden = false;
    props.blend = crate::layer_blend::Mode::Normal;
    p::set_state(d, &mut raw, t, props)?;
    raw.scene.clipping.bind(t, None);
    match t {
        Target::Raster(id) => {
            let n = crate::layers::index(d, id).ok_or("Missing raster")?;
            raw.scene.paper.layers.remove(&n);
            raw.scene.layer_effects.rasters.remove(&id);
        }
        Target::Folder(n) => {
            raw.scene.paper.folders[n as usize].through = false;
            raw.scene.paper.folders[n as usize].paper = Default::default();
            raw.scene.layer_effects.folders.remove(&n);
        }
        Target::Vector(id) => {
            raw.scene.layer_effects.vectors.remove(&id);
            raw.scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing vector")?
                .hidden = false;
        }
        Target::Gradient(id) => {
            raw.scene
                .gradients
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing gradient")?
                .hidden = false;
        }
        _ => {}
    }
    let raw = crate::layer_merge::preview(&raw, d.current_frame)?;
    let caches = crate::appearance::Caches::new(d, None)?;
    let shown = match t {
        Target::Raster(id) => crate::layers::index(d, id)
            .and_then(|n| caches.rasters.get(n))
            .and_then(|c| c.as_ref()),
        Target::Vector(id) => d
            .scene
            .lines
            .layers
            .iter()
            .position(|l| l.id == id)
            .and_then(|n| caches.vectors.get(n))
            .and_then(|c| c.as_ref()),
        _ => None,
    }
    .and_then(|c| c.bounds());
    let bounds = shown
        .or_else(|| crate::layer_catalog::bounds(d, key))
        .ok_or("Layer has no bounded canvas content")?;
    let origin = bounds.min.map(f64::floor);
    let size = [0, 1].map(|n| (bounds.max[n].ceil() - origin[n]).max(1.));
    if size.iter().any(|s| !s.is_finite() || *s > 32768.) || size[0] * size[1] > 4. * 1024. * 1024.
    {
        return Err("File object extent exceeds 4 million pixels".into());
    }
    let renderer = crate::render::Renderer::new(&raw);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let float_renderer = (d.color != crate::color_storage::Config::legacy()
        || !d.scene.surface_effects.rasters.is_empty())
    .then(|| crate::float_render::Renderer::new(&raw))
    .transpose()?;
    let display_transform = crate::color_profile::Transform::new(
        &d.color.profile,
        &crate::color_profile::RgbProfile::Srgb,
    )?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(size[0] as usize * size[1] as usize)
        .map_err(|e| e.to_string())?;
    for y in 0..size[1] as u32 {
        for x in 0..size[0] as u32 {
            let point = [origin[0] + x as f64 + 0.5, origin[1] + y as f64 + 0.5];
            let original = raw.scene.transform.inverse().map(point);
            let rgba = match t {
                Target::Raster(id) => renderer.raster_pixel(
                    crate::layers::index(&raw, id).unwrap(),
                    point.map(|n| n.floor() as i64),
                ),
                Target::Vector(id) => renderer.vector_pixel(
                    raw.scene
                        .lines
                        .layers
                        .iter()
                        .position(|l| l.id == id)
                        .unwrap(),
                    point,
                ),
                Target::Object(_) => renderer.object_pixel(
                    crate::layer_catalog::source_index(&raw, key).unwrap(),
                    &raw.scene.objects[crate::layer_catalog::source_index(&raw, key).unwrap()],
                    original,
                ),
                Target::Gradient(id) => raw
                    .scene
                    .gradients
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .unwrap()
                    .pixel(original),
                Target::World(_) => renderer.world_pixel(
                    crate::layer_catalog::source_index(&raw, key).unwrap(),
                    original,
                ),
                Target::Folder(n) => raw.scene.paper.clipped_node_pixel_with(
                    &raw,
                    crate::paper::Node::Folder(n as usize),
                    point.map(|n| n.floor() as i64),
                    &mut |n, _| renderer.raster_pixel(n, point.map(|v| v.floor() as i64)),
                ),
                Target::Cell { track, cell } => {
                    raw.animation.as_ref().unwrap().tracks[track as usize].cells[cell as usize]
                        .pixel(point[0].floor() as i64, point[1].floor() as i64)
                }
                Target::Track(track) => {
                    let (_, n, c) = raw
                        .animation
                        .as_ref()
                        .ok_or("Missing timeline")?
                        .indexed_visible_cells(raw.current_frame)
                        .find(|(n, _, _)| *n == track as usize)
                        .ok_or("Track has no current cell")?;
                    let mut rgba = c.pixel(point[0].floor() as i64, point[1].floor() as i64);
                    rgba[3] = (rgba[3] as f64
                        * p::factor(
                            &raw,
                            Target::Cell {
                                track,
                                cell: n as u16,
                            },
                        ))
                    .round() as u8;
                    rgba
                }
                _ => return Err("Choose a canvas content layer".into()),
            };
            let rgba = if let Some(renderer) = &float_renderer {
                display_transform
                    .map(renderer.source_pixel(key, point)?)?
                    .rgba8()
            } else {
                rgba
            };
            pixels.push(rgba);
        }
    }
    let local = crate::references::LocalImage {
        width: size[0] as u32,
        height: size[1] as u32,
        pixels,
    };
    let external = local
        .document(DEFAULT_DOCUMENT_BUDGET / 8)
        .map_err(|e| e.to_string())?;
    let image = Image {
        width: local.width,
        height: local.height,
        pixels: Arc::new(local.pixels),
    };
    let mut state = d.snapshot();
    state.scene.file_objects.entries.push(Link {
        selection: Default::default(),
        target: key,
        path: path.clone(),
        cache: cache(image, origin, d.scene.transform),
    });
    validate(d, &state)?;
    let after = crate::layer_merge::preview(&state, d.current_frame)?;
    let a = crate::render::Renderer::new(d);
    let b = crate::render::Renderer::new(&after);
    for y in 0..d.height {
        for x in 0..d.width {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            if a.pixel(p)
                .into_iter()
                .zip(b.pixel(p))
                .any(|(a, b)| a.abs_diff(b) > 1)
            {
                return Err("File conversion cannot preserve this backdrop-dependent composition; no files or layers changed".into());
            }
        }
    }
    Ok(Conversion {
        state,
        external,
        path,
        target: key,
    })
}
pub fn from_clipboard(
    d: &Document,
    packet: crate::paste::ClipboardImage,
    path: &Path,
) -> Result<Conversion, String> {
    let path = destination(d, path)?;
    if !packet.image.valid() || packet.image.pixels.len() > 4 * 1024 * 1024 {
        return Err("Invalid or oversized clipboard object".into());
    }
    let origin = packet
        .origin
        .unwrap_or_else(|| crate::paste::center(d, &packet.image))
        .map(|v| v as f64);
    let external = packet
        .image
        .document(DEFAULT_DOCUMENT_BUDGET / 8)
        .map_err(|e| e.to_string())?;
    let image = Image {
        width: packet.image.width,
        height: packet.image.height,
        pixels: Arc::new(packet.image.pixels),
    };
    let layer = cache(image, origin, d.scene.transform);
    let mut placeholder = layer.clone();
    placeholder.source = Source::Solid([0; 4]);
    let (mut state, id) = crate::material_layer::prepare(d, None, "Linked clipboard", placeholder)?;
    let target = Key::Object(id);
    state.scene.file_objects.entries.push(Link {
        selection: Default::default(),
        target,
        path: path.clone(),
        cache: layer,
    });
    validate(d, &state)?;
    Ok(Conversion {
        state,
        external,
        path,
        target,
    })
}
