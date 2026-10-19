//! Stable identities and searchable metadata for editable layers and page sources.
use crate::{
    document::Document,
    layer_properties::{self as p, Properties, Target},
    layer_search::{Expression, Key, Kind, Row},
    scene::{PageObject, Scene},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ids {
    pub next: u64,
    pub values: Vec<u64>,
}
impl Default for Ids {
    fn default() -> Self {
        Self {
            next: 1,
            values: vec![],
        }
    }
}
impl Ids {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn id(&self, n: usize) -> u64 {
        self.values
            .get(n)
            .copied()
            .unwrap_or_else(|| if self.is_default() { n as u64 + 1 } else { 0 })
    }
    pub fn index(&self, id: u64, count: usize) -> Option<usize> {
        if self.is_default() {
            (id > 0 && (id as usize) <= count).then(|| id as usize - 1)
        } else {
            self.values.iter().position(|n| *n == id)
        }
    }
    pub fn valid(&self) -> bool {
        self.next > 0
            && self.values.len() <= 10000
            && self.values.iter().all(|n| *n > 0 && *n < self.next)
            && self.values.iter().collect::<BTreeSet<_>>().len() == self.values.len()
    }
    pub fn matches(&self, count: usize) -> bool {
        self.is_default() || self.values.len() == count
    }
    pub fn initialize(&mut self, count: usize) -> Result<(), String> {
        if self.is_default() && count > 0 {
            self.values = (1..=count as u64).collect();
            self.next = count as u64 + 1;
        }
        if !self.valid() || !self.matches(count) {
            return Err("Source identity count mismatch".into());
        }
        Ok(())
    }
    pub fn append(&mut self, count: usize) -> Result<(), String> {
        if count < self.values.len() || count > 10000 {
            return Err("Explicit source identity remapping is required".into());
        }
        while self.values.len() < count {
            let id = self.next;
            self.next = id.checked_add(1).ok_or("Source identity limit")?;
            self.values.push(id);
        }
        Ok(())
    }
    pub fn remove(&mut self, n: usize, count: usize) -> Result<u64, String> {
        self.initialize(count)?;
        if n >= self.values.len() {
            return Err("Source identity index out of range".into());
        }
        Ok(self.values.remove(n))
    }
    pub fn clear(&mut self, count: usize) -> Result<(), String> {
        self.initialize(count)?;
        self.values.clear();
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub name: Option<String>,
    pub kind: Option<Kind>,
    pub expression: Expression,
    pub tags: Vec<String>,
    pub properties: Properties,
    pub reference: bool,
    pub draft: bool,
}
impl Info {
    pub fn valid(&self) -> bool {
        self.name.as_ref().is_none_or(|s| {
            !s.trim().is_empty() && s.chars().count() <= 64 && !s.chars().any(char::is_control)
        }) && self.tags.len() <= 16
            && self
                .tags
                .iter()
                .all(|s| !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            && self.properties.opacity <= 100
            && self.kind.is_none_or(|k| {
                !matches!(k, Kind::Mask | Kind::Ruler | Kind::Draft | Kind::Broader)
            })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub expression: Expression,
    pub objects: Ids,
    pub worlds: Ids,
    pub tracks: Ids,
    pub cells: BTreeMap<u64, Ids>,
    pub rasters: BTreeMap<u64, Info>,
    pub vectors: BTreeMap<u64, Info>,
    pub folders: BTreeMap<u16, Info>,
    pub track_info: BTreeMap<u64, Info>,
    pub cell_info: BTreeMap<u64, BTreeMap<u64, Info>>,
    pub object_info: BTreeMap<u64, Info>,
    pub world_info: BTreeMap<u64, Info>,
    pub gradients: BTreeMap<u64, Info>,
    pub selections: BTreeMap<u64, Info>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        let entries = self
            .rasters
            .values()
            .chain(self.vectors.values())
            .chain(self.folders.values())
            .chain(self.track_info.values())
            .chain(self.cell_info.values().flat_map(|m| m.values()))
            .chain(self.object_info.values())
            .chain(self.world_info.values())
            .chain(self.gradients.values())
            .chain(self.selections.values());
        self.rasters
            .keys()
            .chain(self.vectors.keys())
            .chain(self.track_info.keys())
            .chain(self.object_info.keys())
            .chain(self.world_info.keys())
            .chain(self.gradients.keys())
            .chain(self.selections.keys())
            .all(|n| *n > 0)
            && self.folders.keys().all(|n| *n < 256)
            && self
                .cell_info
                .iter()
                .all(|(n, m)| *n > 0 && m.keys().all(|n| *n > 0))
            && self.objects.valid()
            && self.worlds.valid()
            && self.tracks.valid()
            && self.cells.len() <= 32
            && self.cells.values().all(Ids::valid)
            && self.rasters.len() <= 256
            && self.vectors.len() <= 32
            && self.folders.len() <= 256
            && self.track_info.len() <= 32
            && self.cell_info.len() <= 32
            && self.cell_info.values().map(|m| m.len()).sum::<usize>() <= 10000
            && self.object_info.len() <= 4096
            && self.world_info.len() <= 256
            && self.gradients.len() <= 256
            && self.selections.len() <= 128
            && entries.clone().all(Info::valid)
            && self
                .rasters
                .values()
                .chain(self.vectors.values())
                .chain(self.folders.values())
                .chain(self.track_info.values())
                .chain(self.cell_info.values().flat_map(|m| m.values()))
                .all(|i| i.name.is_none() && i.properties == Properties::default())
    }
    pub fn bytes(&self) -> usize {
        (self.objects.values.len()
            + self.worlds.values.len()
            + self.tracks.values.len()
            + self.cells.values().map(|i| i.values.len()).sum::<usize>())
            * 8
            + self
                .rasters
                .values()
                .chain(self.vectors.values())
                .chain(self.folders.values())
                .chain(self.track_info.values())
                .chain(self.cell_info.values().flat_map(|m| m.values()))
                .chain(self.object_info.values())
                .chain(self.world_info.values())
                .chain(self.gradients.values())
                .chain(self.selections.values())
                .map(|i| {
                    128 + i.tags.len() * std::mem::size_of::<String>()
                        + i.name.as_ref().map_or(0, String::len)
                        + i.tags.iter().map(String::len).sum::<usize>()
                })
                .sum::<usize>()
    }
    pub fn cell_id(&self, track_id: u64, cell: usize) -> u64 {
        self.cells
            .get(&track_id)
            .map_or(cell as u64 + 1, |i| i.id(cell))
    }
    pub fn info(&self, key: Key) -> Option<&Info> {
        match key {
            Key::Layer(Target::Raster(n)) => self.rasters.get(&n),
            Key::Layer(Target::Vector(n)) => self.vectors.get(&n),
            Key::Layer(Target::Folder(n)) => self.folders.get(&n),
            Key::Track(n) => self.track_info.get(&n),
            Key::Cell { track, cell } => self.cell_info.get(&track).and_then(|m| m.get(&cell)),
            Key::Object(n) => self.object_info.get(&n),
            Key::World(n) => self.world_info.get(&n),
            Key::Gradient(n) => self.gradients.get(&n),
            Key::Selection(n) => self.selections.get(&n),
            _ => None,
        }
    }
    pub fn set(&mut self, key: Key, value: Info) {
        macro_rules! set {
            ($map:expr,$n:expr) => {
                if value == Info::default() {
                    $map.remove(&$n);
                } else {
                    $map.insert($n, value);
                }
            };
        }
        match key {
            Key::Layer(Target::Raster(n)) => set!(self.rasters, n),
            Key::Layer(Target::Vector(n)) => set!(self.vectors, n),
            Key::Layer(Target::Folder(n)) => set!(self.folders, n),
            Key::Track(n) => set!(self.track_info, n),
            Key::Cell { track, cell } => {
                let map = self.cell_info.entry(track).or_default();
                set!(map, cell);
                if map.is_empty() {
                    self.cell_info.remove(&track);
                }
            }
            Key::Object(n) => set!(self.object_info, n),
            Key::World(n) => set!(self.world_info, n),
            Key::Gradient(n) => set!(self.gradients, n),
            Key::Selection(n) => set!(self.selections, n),
            _ => {}
        }
    }
    pub fn valid_animation(&self, a: Option<&crate::animation::Timeline>) -> bool {
        let count = a.map_or(0, |a| a.tracks.len());
        self.tracks.matches(count)
            && a.is_none_or(|a| {
                a.tracks.iter().enumerate().all(|(n, t)| {
                    self.cells
                        .get(&self.tracks.id(n))
                        .is_none_or(|i| i.matches(t.cells.len()))
                })
            })
    }
}
fn reconcile_ids(
    old: &Ids,
    after: &mut Ids,
    before_count: usize,
    after_count: usize,
) -> Result<(), String> {
    if old == after && before_count == after_count {
        return Ok(());
    }
    if old != after && !after.is_default() && after.matches(after_count) && after.valid() {
        return Ok(());
    }
    after.initialize(before_count)?;
    after.append(after_count)
}
pub fn reconcile_scene(before: &Scene, after: &mut Scene) -> Result<(), String> {
    reconcile_ids(
        &before.catalog.objects,
        &mut after.catalog.objects,
        before.objects.len(),
        after.objects.len(),
    )?;
    reconcile_ids(
        &before.catalog.worlds,
        &mut after.catalog.worlds,
        before.worlds.len(),
        after.worlds.len(),
    )
}
pub fn reconcile(before: &Document, after: &mut DocumentState) -> Result<(), String> {
    reconcile_scene(&before.scene, &mut after.scene)?;
    let old = before.animation.as_ref();
    let new = after.animation.as_ref();
    reconcile_ids(
        &before.scene.catalog.tracks,
        &mut after.scene.catalog.tracks,
        old.map_or(0, |a| a.tracks.len()),
        new.map_or(0, |a| a.tracks.len()),
    )?;
    if let Some(a) = new {
        for (n, t) in a.tracks.iter().enumerate() {
            let id = after.scene.catalog.tracks.id(n);
            let old_count = old
                .and_then(|a| {
                    before
                        .scene
                        .catalog
                        .tracks
                        .index(id, a.tracks.len())
                        .and_then(|n| a.tracks.get(n))
                })
                .map_or(0, |t| t.cells.len());
            let original = before
                .scene
                .catalog
                .cells
                .get(&id)
                .cloned()
                .unwrap_or_default();
            let current = after
                .scene
                .catalog
                .cells
                .get(&id)
                .cloned()
                .unwrap_or_default();
            let mut updated = current.clone();
            reconcile_ids(&original, &mut updated, old_count, t.cells.len())?;
            if updated != current {
                after.scene.catalog.cells.insert(id, updated);
            }
        }
    }
    Ok(())
}
pub fn clear_rendered(d: &Document, state: &mut DocumentState) -> Result<(), String> {
    let c = &mut state.scene.catalog;
    state.scene.appearance = Default::default();
    c.objects.clear(d.scene.objects.len())?;
    c.worlds.clear(d.scene.worlds.len())?;
    c.tracks
        .clear(d.animation.as_ref().map_or(0, |a| a.tracks.len()))?;
    c.cells.clear();
    c.rasters.clear();
    c.vectors.clear();
    c.folders.clear();
    c.track_info.clear();
    c.cell_info.clear();
    c.object_info.clear();
    c.world_info.clear();
    c.gradients.clear();
    Ok(())
}
fn resolve(animation: Option<&crate::animation::Timeline>, c: &Data, key: Key) -> Option<Target> {
    match key {
        Key::Layer(t) => Some(t),
        Key::Object(id) => Some(Target::Object(id)),
        Key::Gradient(id) => Some(Target::Gradient(id)),
        Key::World(id) => Some(Target::World(id)),
        Key::Selection(id) => Some(Target::Selection(id)),
        Key::Track(id) => animation
            .and_then(|a| c.tracks.index(id, a.tracks.len()))
            .map(|n| Target::Track(n as u16)),
        Key::Cell { track, cell } => {
            let a = animation?;
            let n = c.tracks.index(track, a.tracks.len())?;
            let index = c.cells.get(&track).map_or_else(
                || {
                    (cell > 0 && (cell as usize) <= a.tracks[n].cells.len())
                        .then(|| cell as usize - 1)
                },
                |ids| ids.index(cell, a.tracks[n].cells.len()),
            );
            index.map(|cell| Target::Cell {
                track: n as u16,
                cell: cell as u16,
            })
        }
    }
}
pub fn target(d: &Document, key: Key) -> Option<Target> {
    resolve(d.animation.as_ref(), &d.scene.catalog, key)
}
pub fn state_target(d: &DocumentState, key: Key) -> Option<Target> {
    resolve(d.animation.as_ref(), &d.scene.catalog, key)
}
pub fn key(d: &Document, target: Target) -> Key {
    match target {
        Target::Track(n) => Key::Track(d.scene.catalog.tracks.id(n as usize)),
        Target::Cell { track, cell } => {
            let id = d.scene.catalog.tracks.id(track as usize);
            Key::Cell {
                track: id,
                cell: d.scene.catalog.cell_id(id, cell as usize),
            }
        }
        Target::Object(id) => Key::Object(id),
        Target::Gradient(id) => Key::Gradient(id),
        Target::World(id) => Key::World(id),
        Target::Selection(id) => Key::Selection(id),
        t => Key::Layer(t),
    }
}
pub fn thumbnail(d: &Document, key: Key) -> Option<crate::thumbnail::Target> {
    use crate::thumbnail::Target as T;
    let target = target(d, key)?;
    if !p::targets(d).contains(&target) {
        return None;
    }
    match target {
        Target::Raster(id) => crate::layers::index(d, id).map(T::Raster),
        Target::Vector(id) => Some(T::Vector(id)),
        Target::Folder(n) => Some(T::Folder(n as usize)),
        Target::Track(n) => Some(T::Track(n as usize)),
        _ => Some(T::Source(key)),
    }
}
pub fn from_thumbnail(d: &Document, t: crate::thumbnail::Target) -> Option<Key> {
    let k = match t {
        crate::thumbnail::Target::Source(k) => k,
        t => key(d, p::from_thumbnail(d, t)),
    };
    thumbnail(d, k).map(|_| k)
}
fn flags(d: &Document, t: Target) -> [bool; 7] {
    let v = p::inherited(d, t);
    let tags = crate::layer_roles::flags(d, t);
    [
        !v.hidden,
        v.locked,
        d.scene.clipping.alpha.contains(&t),
        v.position_locked,
        v.pixels_locked,
        tags.reference,
        tags.draft,
    ]
}
pub fn rows(d: &Document) -> Result<Vec<Row>, String> {
    if !d.scene.catalog.valid_animation(d.animation.as_ref()) {
        return Err("Invalid animation source identities".into());
    }
    let c = &d.scene.catalog;
    let mut rows = vec![];
    for target in p::targets(d).into_iter().filter(|t| {
        matches!(
            t,
            Target::Raster(_)
                | Target::Vector(_)
                | Target::Folder(_)
                | Target::Track(_)
                | Target::Cell { .. }
        )
    }) {
        let key = key(d, target);
        let info = c.info(key).cloned().unwrap_or_default();
        let (name, kind, parent) = match target {
            Target::Raster(id) => {
                let n = crate::layers::index(d, id).ok_or("Missing raster source")?;
                let kind = if crate::layer_effects::adjustment(d, target).is_some() {
                    Kind::Correction
                } else {
                    match info.expression {
                        Expression::Color => Kind::RasterColor,
                        Expression::Gray => Kind::RasterGray,
                        Expression::Mono => Kind::RasterMono,
                    }
                };
                (
                    d.layers[n].name.clone(),
                    kind,
                    crate::layers::parent(&d.scene.paper, n)
                        .map(|n| Key::Layer(Target::Folder(n as u16))),
                )
            }
            Target::Vector(id) => {
                let l = d
                    .scene
                    .lines
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .ok_or("Missing vector source")?;
                (
                    l.name.clone(),
                    match info.expression {
                        Expression::Color => Kind::VectorColor,
                        Expression::Gray => Kind::VectorGray,
                        Expression::Mono => Kind::VectorMono,
                    },
                    None,
                )
            }
            Target::Folder(n) => {
                let f = &d.scene.paper.folders[n as usize];
                (
                    f.name.clone(),
                    Kind::Folder,
                    d.scene
                        .paper
                        .folders
                        .iter()
                        .position(|f| f.children.contains(&crate::paper::Node::Folder(n as usize)))
                        .map(|n| Key::Layer(Target::Folder(n as u16))),
                )
            }
            Target::Track(n) => {
                let a = d.animation.as_ref().ok_or("Missing timeline")?;
                (
                    a.tracks[n as usize].name.clone(),
                    Kind::AnimationFolder,
                    None,
                )
            }
            Target::Cell { track, cell } => {
                let a = d.animation.as_ref().ok_or("Missing timeline")?;
                (
                    a.tracks[track as usize].cells[cell as usize].name.clone(),
                    Kind::Cell,
                    Some(Key::Track(c.tracks.id(track as usize))),
                )
            }
            _ => return Err("Unexpected catalog layer target".into()),
        };
        let mut extras = BTreeSet::new();
        if d.scene
            .layer_effects
            .get(target)
            .is_some_and(|e| e.mask.is_some())
        {
            extras.insert(Kind::Mask);
        }
        let rulers = d.scene.rulers.guides.iter().any(|g| match target {
            Target::Raster(id) => crate::layers::index(d, id).is_some_and(|n| {
                g.link == Some(crate::line_tools::Target::Raster(n))
                    || g.scope
                        == crate::ruler_scope::Scope::Layer(crate::line_tools::Target::Raster(n))
            }),
            Target::Vector(id) => {
                g.link == Some(crate::line_tools::Target::Vector(id))
                    || g.scope
                        == crate::ruler_scope::Scope::Layer(crate::line_tools::Target::Vector(id))
            }
            Target::Folder(n) => g.scope == crate::ruler_scope::Scope::Folder(n),
            _ => false,
        });
        if rulers {
            extras.insert(Kind::Ruler);
        }
        let states = flags(d, target);
        if states[6] {
            extras.insert(Kind::Draft);
        }
        let actual = if kind == Kind::Correction {
            kind
        } else {
            info.kind.unwrap_or(kind)
        };
        if actual == Kind::Tone {
            extras.insert(Kind::Tone);
        }
        rows.push(Row {
            key,
            name,
            kind: actual,
            expression: info.expression,
            extras,
            tags: info.tags,
            states,
            parent,
        });
    }
    for (n, o) in d.scene.objects.iter().enumerate() {
        let key = Key::Object(c.objects.id(n));
        let (name, kind) = match o {
            PageObject::Text { text, .. } => (
                format!("Text {}", text.chars().take(40).collect::<String>()),
                Kind::Text,
            ),
            PageObject::Tone { .. } => (format!("Tone {}", n + 1), Kind::Tone),
            _ => (format!("Page object {}", c.objects.id(n)), Kind::Image),
        };
        rows.push(extra_row(c, key, name, kind));
    }
    for g in &d.scene.gradients.layers {
        let mut row = extra_row(c, Key::Gradient(g.id), g.name.clone(), Kind::Gradient);
        row.states[0] &= !g.hidden;
        rows.push(row);
    }
    for n in 0..d.scene.worlds.len() {
        rows.push(extra_row(
            c,
            Key::World(c.worlds.id(n)),
            format!("3D object {}", c.worlds.id(n)),
            Kind::World,
        ));
    }
    for s in &d.scene.selections.saved {
        rows.push(extra_row(
            c,
            Key::Selection(s.id),
            s.name.clone(),
            Kind::Selection,
        ));
    }
    let mut order = vec![];
    if d.scene.clipping.solo.is_some() {
        for r in &mut rows {
            if matches!(r.key, Key::Object(_) | Key::Gradient(_) | Key::World(_)) {
                r.states[0] = false;
            }
        }
    }
    fn visit(d: &Document, n: crate::paper::Node, out: &mut Vec<Key>, depth: usize) {
        if depth > 16 {
            return;
        }
        match n {
            crate::paper::Node::Raster(n) => out.push(Key::Layer(p::raster(d, n))),
            crate::paper::Node::Folder(n) => {
                out.push(Key::Layer(Target::Folder(n as u16)));
                for c in &d.scene.paper.folders[n].children {
                    visit(d, *c, out, depth + 1);
                }
            }
        }
    }
    for n in d.scene.paper.root_nodes(d.layers.len()) {
        visit(d, n, &mut order, 0);
    }
    if let Some(a) = &d.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let id = c.tracks.id(n);
            order.push(Key::Track(id));
            order.extend((0..t.cells.len()).map(|i| Key::Cell {
                track: id,
                cell: c.cell_id(id, i),
            }));
        }
    }
    order.extend(d.scene.gradients.layers.iter().map(|g| Key::Gradient(g.id)));
    order.extend((0..d.scene.objects.len()).map(|n| Key::Object(c.objects.id(n))));
    order.extend(
        d.scene
            .lines
            .layers
            .iter()
            .map(|l| Key::Layer(Target::Vector(l.id))),
    );
    order.extend((0..d.scene.worlds.len()).map(|n| Key::World(c.worlds.id(n))));
    order.extend(
        d.scene
            .selections
            .saved
            .iter()
            .map(|s| Key::Selection(s.id)),
    );
    let positions = order
        .into_iter()
        .enumerate()
        .map(|(i, k)| (k, i))
        .collect::<BTreeMap<_, _>>();
    rows.sort_by_key(|r| positions.get(&r.key).copied().unwrap_or(usize::MAX));
    if rows.len() > 16384 {
        return Err("Layer catalog record limit".into());
    }
    Ok(rows)
}
fn extra_row(c: &Data, key: Key, name: String, kind: Kind) -> Row {
    let i = c.info(key).cloned().unwrap_or_default();
    let v = i.properties;
    let kind = i.kind.unwrap_or(kind);
    let mut extras = BTreeSet::new();
    if kind == Kind::Tone {
        extras.insert(Kind::Tone);
    }
    if i.draft {
        extras.insert(Kind::Draft);
    }
    Row {
        key,
        name: i.name.unwrap_or(name),
        kind,
        expression: i.expression,
        extras,
        tags: i.tags,
        states: [
            !v.hidden,
            v.locked,
            false,
            v.position_locked,
            v.pixels_locked,
            i.reference,
            i.draft,
        ],
        parent: None,
    }
}
pub fn properties(d: &Document, key: Key) -> Properties {
    let mut value = target(d, key).map_or_else(
        || {
            d.scene
                .catalog
                .info(key)
                .map_or_else(Properties::default, |i| i.properties)
        },
        |t| p::properties(d, t),
    );
    if let Key::Gradient(id) = key {
        value.hidden |= d
            .scene
            .gradients
            .layers
            .iter()
            .find(|g| g.id == id)
            .is_none_or(|g| g.hidden);
    }
    value
}
pub fn target_map(d: &Document) -> BTreeMap<Key, Target> {
    p::targets(d).into_iter().map(|t| (key(d, t), t)).collect()
}
pub(crate) fn compose(d: &Document, key: Key, back: [u8; 4], top: [u8; 4]) -> [u8; 4] {
    let p = properties(d, key);
    if p.hidden || p.opacity == 0 {
        return back;
    }
    if p.opacity == 100 && p.blend.is_normal() {
        return crate::render::over(back, top);
    }
    p.blend.compose(back, top, p.opacity as f64 / 100.)
}
pub fn source_index(d: &Document, key: Key) -> Option<usize> {
    match key {
        Key::Object(id) => d.scene.catalog.objects.index(id, d.scene.objects.len()),
        Key::World(id) => d.scene.catalog.worlds.index(id, d.scene.worlds.len()),
        Key::Gradient(id) => d.scene.gradients.layers.iter().position(|g| g.id == id),
        Key::Selection(id) => d.scene.selections.saved.iter().position(|s| s.id == id),
        _ => None,
    }
}

pub fn bounds(d: &Document, key: Key) -> Option<crate::scene::Rect> {
    use crate::scene::{Affine, Rect};
    let transformed = |r: Rect, t: Affine| {
        let points = [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]].map(|p| t.map(p));
        Rect {
            min: std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min)),
            max: std::array::from_fn(|i| {
                points
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        }
    };
    match key {
        Key::Object(_) => Some(transformed(
            d.scene.objects.get(source_index(d, key)?)?.bounds(),
            d.scene.transform,
        )),
        Key::World(_) => d
            .scene
            .camera
            .as_ref()
            .map(|c| transformed(c.page_bounds, d.scene.transform)),
        Key::Gradient(_) => {
            let g = d.scene.gradients.layers.get(source_index(d, key)?)?;
            let mut min = [f64::INFINITY; 2];
            let mut max = [f64::NEG_INFINITY; 2];
            for run in g.areas.iter().flat_map(|a| {
                a.paint
                    .runs
                    .iter()
                    .chain(a.black.runs.iter())
                    .chain(a.transparent.runs.iter())
            }) {
                min[0] = min[0].min(run.x as f64);
                min[1] = min[1].min(run.y as f64);
                max[0] = max[0].max(run.x as f64 + run.length as f64);
                max[1] = max[1].max(run.y as f64 + 1.);
            }
            min[0].is_finite().then(|| {
                transformed(
                    Rect { min, max },
                    g.creation.inverse().then(d.scene.transform),
                )
            })
        }
        Key::Selection(id) => {
            let shape = &d.scene.selections.saved.iter().find(|s| s.id == id)?.shape;
            let r = if let Some(mask) = &shape.mask {
                let mut min = [f64::INFINITY; 2];
                let mut max = [f64::NEG_INFINITY; 2];
                for run in &mask.runs {
                    min[0] = min[0].min(run.x as f64);
                    min[1] = min[1].min(run.y as f64);
                    max[0] = max[0].max(run.x as f64 + run.length as f64);
                    max[1] = max[1].max(run.y as f64 + 1.);
                }
                if !min[0].is_finite() {
                    return None;
                }
                Rect { min, max }
            } else {
                shape.rect?
            };
            let mut r = transformed(r, d.scene.transform);
            for a in 0..2 {
                r.min[a] += shape.translation[a];
                r.max[a] += shape.translation[a];
            }
            Some(r)
        }
        _ => match target(d, key)? {
            Target::Raster(id) => Some(crate::thumbnail::bounds(
                d,
                crate::thumbnail::Target::Raster(crate::layers::index(d, id)?),
                true,
            )),
            Target::Folder(_) => Some(Rect {
                min: [0.; 2],
                max: [d.width as f64, d.height as f64],
            }),
            Target::Vector(id) => {
                let l = d.scene.lines.layers.iter().find(|l| l.id == id)?;
                let mut min = [f64::INFINITY; 2];
                let mut max = [f64::NEG_INFINITY; 2];
                for path in &l.paths {
                    let r = transformed(path.bounds(), d.scene.transform);
                    for a in 0..2 {
                        min[a] = min[a].min(r.min[a]);
                        max[a] = max[a].max(r.max[a]);
                    }
                }
                min[0].is_finite().then_some(Rect { min, max })
            }
            Target::Track(n) => d.animation.as_ref().and_then(|a| {
                a.indexed_visible_cells(d.current_frame)
                    .find(|(t, _, _)| *t == n as usize)
                    .map(|(_, _, c)| Rect {
                        min: c.offset.map(|n| n as f64),
                        max: [
                            c.offset[0] as f64 + c.width as f64,
                            c.offset[1] as f64 + c.height as f64,
                        ],
                    })
            }),
            Target::Cell { track, cell } => {
                let c = d
                    .animation
                    .as_ref()?
                    .tracks
                    .get(track as usize)?
                    .cells
                    .get(cell as usize)?;
                Some(Rect {
                    min: c.offset.map(|n| n as f64),
                    max: [
                        c.offset[0] as f64 + c.width as f64,
                        c.offset[1] as f64 + c.height as f64,
                    ],
                })
            }
            _ => None,
        },
    }
}
