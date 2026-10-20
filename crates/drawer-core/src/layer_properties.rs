//! Local layer state, edit guards and output-independent identification colors.
use crate::{document::Document, sizing::DocumentState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Target {
    Raster(u64),
    Vector(u64),
    Folder(u16),
    Object(u64),
    Gradient(u64),
    World(u64),
    Selection(u64),
    Track(u16),
    Cell { track: u16, cell: u16 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Properties {
    pub hidden: bool,
    pub opacity: u8,
    pub locked: bool,
    pub pixels_locked: bool,
    pub position_locked: bool,
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "crate::layer_blend::Mode::is_normal")]
    pub blend: crate::layer_blend::Mode,
}
impl Default for Properties {
    fn default() -> Self {
        Self {
            hidden: false,
            opacity: 100,
            locked: false,
            pixels_locked: false,
            position_locked: false,
            color: None,
            blend: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rasters: BTreeMap<u64, Properties>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vectors: BTreeMap<u64, Properties>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub folders: BTreeMap<u16, Properties>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<u16, Properties>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cells: BTreeMap<u64, Properties>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.rasters.len() <= 256
            && self.vectors.len() <= 32
            && self.folders.len() <= 256
            && self.tracks.len() <= 32
            && self.cells.len() <= 10000
            && self.cells.values().all(|p| p.blend.is_normal())
            && self
                .rasters
                .keys()
                .chain(self.vectors.keys())
                .all(|n| *n > 0)
            && self.folders.keys().all(|n| *n < 256)
            && self.tracks.keys().all(|n| *n < 32)
            && self
                .cells
                .keys()
                .all(|n| *n >> 32 < 32 && (*n & 0xffff_ffff) < 10000)
            && self
                .rasters
                .values()
                .chain(self.vectors.values())
                .chain(self.folders.values())
                .chain(self.tracks.values())
                .chain(self.cells.values())
                .all(|p| p.opacity <= 100)
    }
    pub fn bytes(&self) -> usize {
        (self.rasters.len()
            + self.vectors.len()
            + self.folders.len()
            + self.tracks.len()
            + self.cells.len())
            * 64
    }
    pub fn get(&self, t: Target) -> Properties {
        let p = match t {
            Target::Raster(id) => self.rasters.get(&id),
            Target::Vector(id) => self.vectors.get(&id),
            Target::Folder(id) => self.folders.get(&id),
            Target::Track(id) => self.tracks.get(&id),
            Target::Cell { track, cell } => self.cells.get(&((track as u64) << 32 | cell as u64)),
            _ => None,
        };
        p.copied().unwrap_or_default()
    }
    pub fn set(&mut self, t: Target, p: Properties) {
        macro_rules! set {
            ($map:expr,$id:expr) => {
                if p == Properties::default() {
                    $map.remove(&$id);
                } else {
                    $map.insert($id, p);
                }
            };
        }
        match t {
            Target::Raster(id) => set!(self.rasters, id),
            Target::Vector(id) => set!(self.vectors, id),
            Target::Folder(id) => set!(self.folders, id),
            Target::Track(id) => set!(self.tracks, id),
            Target::Cell { track, cell } => set!(self.cells, (track as u64) << 32 | cell as u64),
            _ => {}
        }
    }
}
pub fn raster(doc: &Document, n: usize) -> Target {
    Target::Raster(doc.scene.layer_ids.id(n))
}
pub fn properties(doc: &Document, t: Target) -> Properties {
    let mut p = match t {
        Target::Object(id) => doc.scene.catalog.object_info.get(&id),
        Target::World(id) => doc.scene.catalog.world_info.get(&id),
        Target::Gradient(id) => doc.scene.catalog.gradients.get(&id),
        Target::Selection(id) => doc.scene.catalog.selections.get(&id),
        _ => None,
    }
    .map_or_else(|| doc.scene.layer_properties.get(t), |i| i.properties);
    match t {
        Target::Raster(id) => {
            if let Some(n) = crate::layers::index(doc, id) {
                p.locked = doc.layers[n].locked;
                p.hidden = doc.scene.paper.layers.get(&n).is_some_and(|p| p.hidden);
            }
        }
        Target::Vector(id) => {
            if let Some(l) = doc.scene.lines.layers.iter().find(|l| l.id == id) {
                p.locked = l.locked;
                p.hidden = l.hidden;
            }
        }
        Target::Folder(id) => {
            p.hidden = doc
                .scene
                .paper
                .folders
                .get(id as usize)
                .is_some_and(|f| f.paper.hidden)
        }
        Target::Gradient(id) => {
            p.hidden |= doc
                .scene
                .gradients
                .layers
                .iter()
                .find(|g| g.id == id)
                .is_none_or(|g| g.hidden)
        }
        _ => {}
    }
    p
}
pub fn targets(doc: &Document) -> Vec<Target> {
    (0..doc.layers.len())
        .map(|n| raster(doc, n))
        .chain(doc.scene.lines.layers.iter().map(|l| Target::Vector(l.id)))
        .chain(
            (0..doc.scene.paper.folders.len())
                .filter(|n| doc.scene.paper.folder_active(*n))
                .map(|n| Target::Folder(n as u16)),
        )
        .chain(
            doc.animation
                .iter()
                .flat_map(|a| a.tracks.iter().enumerate())
                .flat_map(|(n, t)| {
                    std::iter::once(Target::Track(n as u16)).chain((0..t.cells.len()).map(
                        move |i| Target::Cell {
                            track: n as u16,
                            cell: i as u16,
                        },
                    ))
                }),
        )
        .chain(
            (0..doc.scene.objects.len()).map(|n| Target::Object(doc.scene.catalog.objects.id(n))),
        )
        .chain(
            doc.scene
                .gradients
                .layers
                .iter()
                .map(|g| Target::Gradient(g.id)),
        )
        .chain((0..doc.scene.worlds.len()).map(|n| Target::World(doc.scene.catalog.worlds.id(n))))
        .chain(
            doc.scene
                .selections
                .saved
                .iter()
                .map(|s| Target::Selection(s.id)),
        )
        .collect()
}
pub fn inherited(doc: &Document, t: Target) -> Properties {
    let mut p = properties(doc, t);
    if doc
        .scene
        .file_objects
        .get(crate::layer_catalog::key(doc, t))
        .is_some()
    {
        p.pixels_locked = true;
    }
    if doc.scene.clipping.solo.is_some()
        && matches!(
            t,
            Target::Object(_) | Target::Gradient(_) | Target::World(_)
        )
    {
        p.hidden = true;
    }
    let mut node = match t {
        Target::Raster(id) => crate::layers::index(doc, id).map(crate::paper::Node::Raster),
        Target::Folder(id) => Some(crate::paper::Node::Folder(id as usize)),
        _ => None,
    };
    for _ in 0..16 {
        let Some(n) = node else {
            break;
        };
        let Some(parent) = doc
            .scene
            .paper
            .folders
            .iter()
            .position(|f| f.children.contains(&n))
        else {
            break;
        };
        let q = properties(doc, Target::Folder(parent as u16));
        p.locked |= q.locked
            || doc
                .scene
                .file_objects
                .get(crate::layer_catalog::key(
                    doc,
                    Target::Folder(parent as u16),
                ))
                .is_some();
        p.pixels_locked |= q.pixels_locked
            || doc
                .scene
                .file_objects
                .get(crate::layer_catalog::key(
                    doc,
                    Target::Folder(parent as u16),
                ))
                .is_some();
        p.position_locked |= q.position_locked;
        p.hidden |= q.hidden;
        node = Some(crate::paper::Node::Folder(parent));
    }
    if let Target::Cell { track, .. } = t {
        let q = properties(doc, Target::Track(track));
        p.locked |= q.locked
            || doc
                .scene
                .file_objects
                .get(crate::layer_catalog::key(doc, Target::Track(track)))
                .is_some();
        p.pixels_locked |= q.pixels_locked
            || doc
                .scene
                .file_objects
                .get(crate::layer_catalog::key(doc, Target::Track(track)))
                .is_some();
        p.position_locked |= q.position_locked;
        p.hidden |= q.hidden;
    }
    p
}
pub fn paintable(doc: &Document, n: usize) -> Result<(), String> {
    if n >= doc.layers.len() {
        return Err("Raster layer is missing".into());
    }
    if crate::layer_effects::adjustment(doc, raster(doc, n)).is_some() {
        return Err(
            "Paint on an image layer; edit this adjustment layer through its parameters".into(),
        );
    }
    let p = inherited(doc, raster(doc, n));
    if p.hidden || !doc.scene.paper.raster_visible(n, doc.layers.len()) {
        return Err("Layer is hidden; show it before editing".into());
    }
    if p.locked || p.pixels_locked {
        return Err("Layer pixels are locked".into());
    }
    Ok(())
}
pub fn movable(doc: &Document, t: Target) -> Result<(), String> {
    let p = inherited(doc, t);
    if p.hidden || p.locked || p.position_locked {
        Err("Layer is hidden or its position is locked".into())
    } else {
        Ok(())
    }
}
pub fn factor(doc: &Document, t: Target) -> f64 {
    let p = properties(doc, t);
    if p.hidden {
        0.0
    } else {
        p.opacity as f64 / 100.0
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Patch {
    pub hidden: Option<bool>,
    pub opacity: Option<u8>,
    pub locked: Option<bool>,
    pub pixels_locked: Option<bool>,
    pub position_locked: Option<bool>,
    pub color: Option<Option<[u8; 3]>>,
    pub blend: Option<crate::layer_blend::Mode>,
}
impl Patch {
    pub fn apply(self, mut p: Properties) -> Properties {
        if let Some(v) = self.hidden {
            p.hidden = v;
        }
        if let Some(v) = self.opacity {
            p.opacity = v;
        }
        if let Some(v) = self.locked {
            p.locked = v;
        }
        if let Some(v) = self.pixels_locked {
            p.pixels_locked = v;
        }
        if let Some(v) = self.position_locked {
            p.position_locked = v;
        }
        if let Some(v) = self.blend {
            p.blend = v;
        }
        if let Some(v) = self.color {
            p.color = v;
        }
        p
    }

    pub fn empty(self) -> bool {
        self.hidden.is_none()
            && self.opacity.is_none()
            && self.locked.is_none()
            && self.pixels_locked.is_none()
            && self.position_locked.is_none()
            && self.color.is_none()
            && self.blend.is_none()
    }
}
pub fn prepare(
    doc: &Document,
    selected: &[Target],
    p: Properties,
) -> Result<DocumentState, String> {
    prepare_patch(
        doc,
        selected,
        Patch {
            hidden: Some(p.hidden),
            opacity: Some(p.opacity),
            locked: Some(p.locked),
            pixels_locked: Some(p.pixels_locked),
            position_locked: Some(p.position_locked),
            color: Some(p.color),
            blend: Some(p.blend),
        },
    )
}
pub fn prepare_patch(
    doc: &Document,
    selected: &[Target],
    patch: Patch,
) -> Result<DocumentState, String> {
    if selected.is_empty()
        || selected.len() > 16384
        || patch.opacity.is_some_and(|n| n > 100)
        || selected
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != selected.len()
    {
        return Err("Choose unique layers and a valid opacity".into());
    }
    if doc.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Layer property snapshot budget".into());
    }
    let available = targets(doc)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if selected.iter().any(|t| !available.contains(t)) {
        return Err("Layer selection changed; reopen properties".into());
    }
    let updated = |t: Target| {
        let old = properties(doc, t);
        let mut p = patch.apply(old);
        if matches!(t, Target::Vector(_)) {
            p.pixels_locked = old.pixels_locked;
        }
        if matches!(t, Target::Cell { .. }) {
            p.blend = old.blend;
        }
        p
    };
    let mut state = doc.snapshot();
    if patch.empty() || selected.iter().all(|t| updated(*t) == properties(doc, *t)) {
        return Ok(state);
    }
    state.scene.layer_ids.initialize(doc.layers.len())?;
    for t in selected {
        let p = updated(*t);
        set_state(doc, &mut state, *t, p)?;
    }
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Layer properties exceed scene limits".into());
    }
    Ok(state)
}
pub(crate) fn set_state(
    doc: &Document,
    state: &mut DocumentState,
    t: Target,
    p: Properties,
) -> Result<(), String> {
    state.scene.layer_properties.set(t, p);
    match t {
        Target::Raster(id) => {
            let n = crate::layers::index(doc, id).unwrap();
            state.layers[n].locked = p.locked;
            if let Some(paper) = state.scene.paper.layers.get_mut(&n) {
                paper.hidden = p.hidden;
                if *paper == Default::default() {
                    state.scene.paper.layers.remove(&n);
                }
            } else if p.hidden {
                state.scene.paper.layers.entry(n).or_default().hidden = true;
            }
        }
        Target::Vector(id) => {
            let l = state
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap();
            l.hidden = p.hidden;
            l.locked = p.locked;
        }
        Target::Folder(n) => {
            state.scene.paper.folders[n as usize].paper.hidden = p.hidden;
            if !p.blend.is_normal() {
                state.scene.paper.folders[n as usize].through = false;
            }
        }
        Target::Object(id) | Target::Gradient(id) | Target::World(id) | Target::Selection(id) => {
            let key = match t {
                Target::Object(_) => crate::layer_search::Key::Object(id),
                Target::Gradient(_) => crate::layer_search::Key::Gradient(id),
                Target::World(_) => crate::layer_search::Key::World(id),
                _ => crate::layer_search::Key::Selection(id),
            };
            let mut info = state.scene.catalog.info(key).cloned().unwrap_or_default();
            info.properties = p;
            state.scene.catalog.set(key, info);
            if let Target::Gradient(id) = t
                && let Some(g) = state.scene.gradients.layers.iter_mut().find(|g| g.id == id)
            {
                g.hidden = p.hidden;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn state_properties(state: &DocumentState, t: Target) -> Properties {
    let mut p = state.scene.layer_properties.get(t);
    match t {
        Target::Raster(id) => {
            if let Some(n) = (0..state.layers.len()).find(|n| state.scene.layer_ids.id(*n) == id) {
                p.hidden = state.scene.paper.layers.get(&n).is_some_and(|p| p.hidden);
                p.locked = state.layers[n].locked;
            }
        }
        Target::Vector(id) => {
            if let Some(l) = state.scene.lines.layers.iter().find(|l| l.id == id) {
                p.hidden = l.hidden;
                p.locked = l.locked;
            }
        }
        Target::Folder(n) => {
            p.hidden = state
                .scene
                .paper
                .folders
                .get(n as usize)
                .is_some_and(|f| f.paper.hidden)
        }
        _ => {}
    }
    p
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    Color([u8; 3]),
    Mixed,
}
pub fn mark(doc: &Document, t: Target) -> Mark {
    if let Target::Folder(n) = t {
        let mut colors = std::collections::BTreeSet::new();
        let own = properties(doc, t).color;
        if let Some(c) = own {
            colors.insert(c);
        }
        fn collect(
            doc: &Document,
            n: crate::paper::Node,
            colors: &mut std::collections::BTreeSet<[u8; 3]>,
            depth: usize,
        ) {
            if depth > 16 {
                return;
            }
            match n {
                crate::paper::Node::Raster(n) => {
                    if let Some(c) = properties(doc, raster(doc, n)).color {
                        colors.insert(c);
                    }
                }
                crate::paper::Node::Folder(n) => {
                    if let Some(c) = properties(doc, Target::Folder(n as u16)).color {
                        colors.insert(c);
                    }
                    if let Some(f) = doc.scene.paper.folders.get(n) {
                        for n in &f.children {
                            collect(doc, *n, colors, depth + 1);
                        }
                    }
                }
            }
        }
        if let Some(f) = doc.scene.paper.folders.get(n as usize) {
            for c in &f.children {
                collect(doc, *c, &mut colors, 0);
            }
        }
        return match colors.len() {
            0 => Mark::None,
            1 => Mark::Color(*colors.first().unwrap()),
            _ => Mark::Mixed,
        };
    }
    properties(doc, t).color.map_or(Mark::None, Mark::Color)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub list: bool,
    pub parents: bool,
    pub canvas: bool,
    pub timeline: bool,
}
impl Default for Display {
    fn default() -> Self {
        Self {
            list: true,
            parents: true,
            canvas: false,
            timeline: true,
        }
    }
}

/// Guards every document-state path, including edits prepared outside the pointer engine.
pub fn validate_edit(doc: &Document, after: &DocumentState) -> Result<(), String> {
    crate::appearance::limits(after.width, after.height, &after.layers, &after.scene)?;
    for (n, old) in doc.layers.iter().enumerate() {
        let p = crate::file_objects::validation_flags(doc, &after.scene, raster(doc, n));
        if !p.locked && !p.pixels_locked && !p.position_locked && !p.hidden {
            continue;
        }
        let id = doc.scene.layer_ids.id(n);
        let next = (0..after.layers.len()).find(|i| after.scene.layer_ids.id(*i) == id);
        let Some(next) = next else {
            if p.locked {
                return Err("A locked layer cannot be deleted".into());
            }
            continue;
        };
        let new = &after.layers[next];
        if (p.locked
            || p.pixels_locked
            || (p.hidden && !crate::group_edits::geometry_matches(doc, after, id)))
            && !old.same_color(new)
        {
            return Err("Layer pixels are hidden or locked".into());
        }
        let same_storage = old.same_color(new)
            && old.extent(doc.width, doc.height).width
                == new.extent(after.width, after.height).width
            && old.extent(doc.width, doc.height).height
                == new.extent(after.width, after.height).height;
        if (p.locked || p.position_locked)
            && same_storage
            && old.extent(doc.width, doc.height).offset
                != new.extent(after.width, after.height).offset
        {
            return Err("Layer position is locked".into());
        }
        if p.locked && old.name != new.name {
            return Err("Layer is locked".into());
        }
    }
    if let Some(animation) = &doc.animation {
        for (track, t) in animation.tracks.iter().enumerate() {
            let tp = crate::file_objects::validation_flags(
                doc,
                &after.scene,
                Target::Track(track as u16),
            );
            let track_id = doc.scene.catalog.tracks.id(track);
            let next_target = crate::layer_catalog::state_target(
                after,
                crate::layer_search::Key::Track(track_id),
            );
            let next = if let Some(Target::Track(n)) = next_target {
                after
                    .animation
                    .as_ref()
                    .and_then(|a| a.tracks.get(n as usize))
            } else {
                None
            };
            if tp.locked
                && next.is_none_or(|n| {
                    n.name != t.name
                        || n.exposures != t.exposures
                        || n.end != t.end
                        || n.cells.len() != t.cells.len()
                })
            {
                return Err("Animation track is locked".into());
            }
            for (cell, c) in t.cells.iter().enumerate() {
                let p = crate::file_objects::validation_flags(
                    doc,
                    &after.scene,
                    Target::Cell {
                        track: track as u16,
                        cell: cell as u16,
                    },
                );
                let cell_id = doc.scene.catalog.cell_id(track_id, cell);
                let new = if let Some(Target::Cell { track, cell }) =
                    crate::layer_catalog::state_target(
                        after,
                        crate::layer_search::Key::Cell {
                            track: track_id,
                            cell: cell_id,
                        },
                    ) {
                    after
                        .animation
                        .as_ref()
                        .and_then(|a| a.tracks.get(track as usize))
                        .and_then(|t| t.cells.get(cell as usize))
                } else {
                    None
                };
                if p.locked && new.is_none() {
                    return Err("Animation cell is locked".into());
                }
                if let Some(n) = new {
                    if (p.locked || p.pixels_locked || p.hidden) && c.pixels != n.pixels {
                        return Err("Animation cell pixels are hidden or locked".into());
                    }
                    if (p.locked || p.position_locked) && c.offset != n.offset {
                        return Err("Animation cell position is locked".into());
                    }
                }
            }
        }
    }
    validate_scene(doc, &after.scene)?;

    Ok(())
}

pub fn validate_scene(doc: &Document, after: &crate::scene::Scene) -> Result<(), String> {
    crate::catalog_guard::validate(doc, after)?;
    for old in &doc.scene.lines.layers {
        let p = crate::file_objects::validation_flags(doc, after, Target::Vector(old.id));
        let next = after.lines.layers.iter().find(|l| l.id == old.id);
        if p.locked && next.is_none_or(|l| l.paths != old.paths || l.name != old.name) {
            return Err("Vector layer is locked".into());
        }
        if p.position_locked
            && let Some(new) = next
        {
            for old_path in &old.paths {
                if let Some(new_path) = new.paths.iter().find(|p| p.id == old_path.id)
                    && new_path.points.len() == old_path.points.len()
                {
                    let delta = [
                        new_path.points[0][0] - old_path.points[0][0],
                        new_path.points[0][1] - old_path.points[0][1],
                    ];
                    if delta[0].hypot(delta[1]) > 1e-8
                        && old_path.points.iter().zip(&new_path.points).all(|(a, b)| {
                            ((b[0] - a[0]) - delta[0]).hypot((b[1] - a[1]) - delta[1]) < 1e-8
                        })
                    {
                        return Err("Vector position is locked".into());
                    }
                }
            }
        }
        if p.hidden && next.is_some_and(|l| l.paths != old.paths) {
            return Err("Vector layer is hidden".into());
        }
    }
    if doc.scene.transform != after.transform
        && targets(doc).into_iter().any(|t| {
            let p = crate::file_objects::validation_flags(doc, after, t);
            p.locked || p.position_locked
        })
    {
        return Err("A locked layer prevents changing page geometry".into());
    }
    for (n, old) in doc.scene.paper.folders.iter().enumerate() {
        let p = crate::file_objects::validation_flags(doc, after, Target::Folder(n as u16));
        if p.locked
            && after
                .paper
                .folders
                .get(n)
                .is_none_or(|f| f.name != old.name || f.children != old.children)
        {
            return Err("Folder is locked".into());
        }
    }
    Ok(())
}

pub fn from_thumbnail(doc: &Document, t: crate::thumbnail::Target) -> Target {
    match t {
        crate::thumbnail::Target::Raster(n) => raster(doc, n),
        crate::thumbnail::Target::Vector(id) => Target::Vector(id),
        crate::thumbnail::Target::Folder(n) => Target::Folder(n as u16),
        crate::thumbnail::Target::Track(n) => Target::Track(n as u16),
        crate::thumbnail::Target::Source(key) => {
            crate::layer_catalog::target(doc, key).unwrap_or(Target::Raster(0))
        }
    }
}
pub fn canvas_mark(doc: &Document, t: crate::thumbnail::Target) -> Mark {
    if let crate::thumbnail::Target::Track(n) = t
        && let Some((_, cell, _)) = doc.animation.as_ref().and_then(|a| {
            a.indexed_visible_cells(doc.current_frame)
                .find(|(track, _, _)| *track == n)
        })
    {
        let target = Target::Cell {
            track: n as u16,
            cell: cell as u16,
        };
        if inherited(doc, target).hidden {
            return Mark::None;
        }
        if let Some(c) = properties(doc, target).color {
            return Mark::Color(c);
        }
    }
    mark(doc, from_thumbnail(doc, t))
}
