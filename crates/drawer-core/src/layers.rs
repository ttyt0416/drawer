//! Stable raster identities and atomic layer management. Graph order is composition order.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer},
    paper::{Graph, Node},
    scene::Scene,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ids {
    pub next: u64,
    pub rasters: Vec<u64>,
}
impl Default for Ids {
    fn default() -> Self {
        Self {
            next: 1,
            rasters: vec![],
        }
    }
}
impl Ids {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.next > 0
            && self.rasters.len() <= 256
            && self.rasters.iter().all(|n| *n > 0 && *n < self.next)
            && self
                .rasters
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.rasters.len()
    }
    pub fn matches(&self, count: usize) -> bool {
        self.rasters.is_empty() || self.rasters.len() == count
    }
    pub fn initialize(&mut self, count: usize) -> Result<(), String> {
        if self.rasters.is_empty() {
            self.rasters = (1..=count as u64).collect();
            self.next = count as u64 + 1;
        }
        if !self.valid() || !self.matches(count) {
            return Err("Raster identity count mismatch".into());
        }
        Ok(())
    }
    pub fn append(&mut self, count: usize) -> Result<(), String> {
        if self.rasters.is_empty() {
            return Ok(());
        }
        if count < self.rasters.len() || count > 256 {
            return Err("Explicit remapping is required when deleting layers".into());
        }
        while self.rasters.len() < count {
            let id = self.next;
            self.next = self.next.checked_add(1).ok_or("Raster identity limit")?;
            self.rasters.push(id);
        }
        Ok(())
    }
    pub fn id(&self, index: usize) -> u64 {
        self.rasters.get(index).copied().unwrap_or(index as u64 + 1)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Raster(u64),
    Vector(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raster,
    Vector,
}
#[derive(Clone, Debug)]
pub enum Command {
    Create {
        kind: Kind,
        name: String,
        parent: Option<usize>,
    },
    Rename {
        target: Target,
        name: String,
    },
    Duplicate(Target),
    Delete(Target),
    Move {
        target: Target,
        parent: Option<usize>,
        position: usize,
    },
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Target,
}
pub fn index(doc: &Document, id: u64) -> Option<usize> {
    (0..doc.layers.len()).find(|n| doc.scene.layer_ids.id(*n) == id)
}
pub fn order(doc: &Document) -> Vec<usize> {
    fn visit(g: &Graph, n: Node, result: &mut Vec<usize>, depth: usize) {
        if depth > 16 {
            return;
        }
        match n {
            Node::Raster(n) => {
                if !result.contains(&n) {
                    result.push(n);
                }
            }
            Node::Folder(n) => {
                if let Some(f) = g.folders.get(n) {
                    for c in &f.children {
                        visit(g, *c, result, depth + 1);
                    }
                }
            }
        }
    }
    let mut result = vec![];
    for n in &doc.scene.paper.roots {
        visit(&doc.scene.paper, *n, &mut result, 0);
    }
    for n in 0..doc.layers.len() {
        if !result.contains(&n) {
            result.push(n);
        }
    }
    result
}
pub fn parent(g: &Graph, n: usize) -> Option<usize> {
    g.folders
        .iter()
        .position(|f| f.children.contains(&Node::Raster(n)))
}
fn materialize(g: &mut Graph, count: usize) {
    let present: std::collections::BTreeSet<_> = g
        .roots
        .iter()
        .chain(g.folders.iter().flat_map(|f| &f.children))
        .filter_map(|n| {
            if let Node::Raster(n) = n {
                Some(*n)
            } else {
                None
            }
        })
        .collect();
    g.roots.extend(
        (0..count)
            .filter(|n| !present.contains(n))
            .map(Node::Raster),
    );
    for n in 0..g.folders.len() {
        if !g.roots.contains(&Node::Folder(n))
            && !g
                .folders
                .iter()
                .any(|f| f.children.contains(&Node::Folder(n)))
        {
            g.roots.push(Node::Folder(n));
        }
    }
}
fn insert(g: &mut Graph, n: usize, p: Option<usize>, position: usize) -> Result<(), String> {
    let list = if let Some(p) = p {
        &mut g
            .folders
            .get_mut(p)
            .ok_or("Missing destination folder")?
            .children
    } else {
        &mut g.roots
    };
    if position > list.len() {
        return Err("Layer destination position is outside the folder".into());
    }
    list.insert(position, Node::Raster(n));
    Ok(())
}
fn detach(g: &mut Graph, n: usize) {
    g.roots.retain(|v| *v != Node::Raster(n));
    for f in &mut g.folders {
        f.children.retain(|v| *v != Node::Raster(n));
    }
}
fn name(value: &str) -> Result<String, String> {
    let s = value.trim();
    if s.is_empty() || s.chars().count() > 64 || s.chars().any(char::is_control) {
        Err("Layer name must contain 1–64 printable characters".into())
    } else {
        Ok(s.into())
    }
}
pub fn remap(scene: &mut Scene, map: &[Option<usize>]) {
    let node = |n: Node| match n {
        Node::Raster(i) => map.get(i).copied().flatten().map(Node::Raster),
        n => Some(n),
    };
    scene.paper.roots = scene.paper.roots.iter().copied().filter_map(node).collect();
    for f in &mut scene.paper.folders {
        f.children = f.children.iter().copied().filter_map(node).collect();
    }
    scene.paper.layers = std::mem::take(&mut scene.paper.layers)
        .into_iter()
        .filter_map(|(n, p)| map.get(n).copied().flatten().map(|n| (n, p)))
        .collect();
    remap_references(scene, map);
    if !scene.layer_ids.rasters.is_empty() {
        let old = scene.layer_ids.rasters.clone();
        let mut ids = vec![0; map.iter().flatten().copied().max().map_or(0, |n| n + 1)];
        for (n, to) in map.iter().enumerate() {
            if let Some(to) = to {
                ids[*to] = old[n];
            }
        }
        scene.layer_ids.rasters = ids;
    }
}
pub fn remap_references(scene: &mut Scene, map: &[Option<usize>]) {
    let target = |t: crate::line_tools::Target| match t {
        crate::line_tools::Target::Raster(n) => map
            .get(n)
            .copied()
            .flatten()
            .map(crate::line_tools::Target::Raster),
        t => Some(t),
    };
    scene.lines.settings.references = scene
        .lines
        .settings
        .references
        .iter()
        .copied()
        .filter_map(target)
        .collect();
    scene.lines.settings.erase = scene
        .lines
        .settings
        .erase
        .iter()
        .copied()
        .filter_map(target)
        .collect();
    scene.mouse_strokes.retain_mut(|s| {
        if let Some(n) = map.get(s.layer).copied().flatten() {
            s.layer = n;
            true
        } else {
            false
        }
    });
    let mut removed = vec![];
    for g in &mut scene.rulers.guides {
        if let crate::ruler_scope::Scope::Layer(t) = g.scope {
            if let Some(t) = target(t) {
                g.scope = crate::ruler_scope::Scope::Layer(t);
            } else {
                removed.push(g.id);
            }
        }
        if let Some(t) = g.link {
            if let Some(t) = target(t) {
                g.link = Some(t);
            } else {
                removed.push(g.id);
            }
        }
    }
    scene.rulers.guides.retain(|g| !removed.contains(&g.id));
    if scene.rulers.active.is_some_and(|id| removed.contains(&id)) {
        scene.rulers.active = None;
    }
    let item = |i: crate::alignment::Item| match i {
        crate::alignment::Item::Raster(n) => map
            .get(n)
            .copied()
            .flatten()
            .map(crate::alignment::Item::Raster),
        i => Some(i),
    };
    scene.alignment.selected = scene
        .alignment
        .selected
        .iter()
        .copied()
        .filter_map(item)
        .collect();
    if let crate::alignment::Reference::Key(i) = scene.alignment.reference {
        scene.alignment.reference = item(i).map_or(
            crate::alignment::Reference::Selection,
            crate::alignment::Reference::Key,
        );
    }
}

pub fn prepare(doc: &Document, command: Command) -> Result<Prepared, String> {
    if !doc.scene.valid()
        || !doc.scene.layer_ids.matches(doc.layers.len())
        || !doc.scene.paper.valid_for(doc.layers.len())
        || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid layer document or snapshot memory limit".into());
    }
    if let Command::Rename { target, name: n } = &command {
        let value = name(n)?;
        let same = match *target {
            Target::Raster(id) => {
                index(doc, id).is_some_and(|n| !doc.layers[n].locked && doc.layers[n].name == value)
            }
            Target::Vector(id) => doc
                .scene
                .lines
                .layers
                .iter()
                .any(|l| l.id == id && !l.locked && l.name == value),
        };
        if same {
            return Ok(Prepared {
                state: doc.snapshot(),
                selected: *target,
            });
        }
    }
    let mut state = doc.snapshot();
    state.scene.layer_ids.initialize(state.layers.len())?;
    let raster = |id: u64| index(doc, id).ok_or("Raster layer no longer exists");
    let mut selected = Target::Raster(state.scene.layer_ids.id(0));
    match command {
        Command::Create {
            kind: Kind::Raster,
            name: n,
            parent: p,
        } => {
            if state.layers.len() >= 256 {
                return Err("Raster layer limit (256)".into());
            }
            let n = name(&n)?;
            materialize(&mut state.scene.paper, state.layers.len());
            let index = state.layers.len();
            state.layers.push(RasterLayer {
                locked: false,
                name: n,
                pixels: vec![[0; 4]],
                placement: Some(PlacedRaster {
                    width: 1,
                    height: 1,
                    offset: [0; 2],
                }),
            });
            state.scene.layer_ids.append(state.layers.len())?;
            let position = p.map_or(state.scene.paper.roots.len(), |p| {
                state
                    .scene
                    .paper
                    .folders
                    .get(p)
                    .map_or(usize::MAX, |f| f.children.len())
            });
            insert(&mut state.scene.paper, index, p, position)?;
            selected = Target::Raster(state.scene.layer_ids.id(index));
        }
        Command::Create {
            kind: Kind::Vector,
            name: n,
            parent: p,
        } => {
            if p.is_some() {
                return Err("Vector layers use their own vector stack".into());
            }
            selected = Target::Vector(state.scene.lines.add_layer(&name(&n)?)?);
        }
        Command::Rename {
            target: Target::Raster(id),
            name: n,
        } => {
            let i = raster(id)?;
            if state.layers[i].locked {
                return Err("Layer is locked".into());
            }
            state.layers[i].name = name(&n)?;
            selected = Target::Raster(id);
        }
        Command::Rename {
            target: Target::Vector(id),
            name: n,
        } => {
            let l = state
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing vector layer")?;
            if l.locked {
                return Err("Layer is locked".into());
            }
            l.name = name(&n)?;
            selected = Target::Vector(id);
        }
        Command::Duplicate(Target::Raster(id)) => {
            if state.layers.len() >= 256 {
                return Err("Raster layer limit (256)".into());
            }
            let i = raster(id)?;
            materialize(&mut state.scene.paper, state.layers.len());
            let p = parent(&state.scene.paper, i);
            let list = p.map_or(&state.scene.paper.roots, |p| {
                &state.scene.paper.folders[p].children
            });
            let pos = list.iter().position(|n| *n == Node::Raster(i)).unwrap() + 1;
            let to = state.layers.len();
            state.layers.push(state.layers[i].clone());
            state.scene.layer_ids.append(state.layers.len())?;
            if let Some(attrs) = state.scene.paper.layers.get(&i).copied() {
                state.scene.paper.layers.insert(to, attrs);
            }
            insert(&mut state.scene.paper, to, p, pos)?;
            let copy_id = state.scene.layer_ids.id(to);
            if state
                .scene
                .clipping
                .alpha
                .contains(&crate::layer_properties::Target::Raster(id))
            {
                state
                    .scene
                    .clipping
                    .alpha
                    .insert(crate::layer_properties::Target::Raster(copy_id));
            }
            if let Some(base) = state
                .scene
                .clipping
                .binding(crate::layer_properties::Target::Raster(id))
            {
                state
                    .scene
                    .clipping
                    .bind(crate::layer_properties::Target::Raster(copy_id), Some(base));
            }
            if let Some(p) = state.scene.layer_properties.rasters.get(&id).copied() {
                state.scene.layer_properties.rasters.insert(copy_id, p);
            }
            if let Some(e) = state.scene.layer_effects.rasters.get(&id).cloned() {
                state.scene.layer_effects.rasters.insert(copy_id, e);
            }
            if let Some(s) = state.scene.appearance.rasters.get(&id).cloned() {
                state.scene.appearance.rasters.insert(copy_id, s);
            }
            if let Some(info) = state.scene.catalog.rasters.get(&id).cloned() {
                state.scene.catalog.rasters.insert(copy_id, info);
            }
            crate::file_objects::duplicate(
                &mut state,
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(id)),
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(copy_id)),
            );
            crate::render_order::duplicate(&mut state.scene, id, copy_id);
            selected = Target::Raster(copy_id);
        }
        Command::Duplicate(Target::Vector(id)) => {
            let source = state
                .scene
                .lines
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Missing vector layer")?
                .clone();
            let to = state.scene.lines.add_layer(&source.name)?;
            let mut copy = source;
            copy.id = to;
            for p in &mut copy.paths {
                p.id = state.scene.lines.id()?;
            }
            let index = state
                .scene
                .lines
                .layers
                .iter()
                .position(|l| l.id == id)
                .unwrap();
            state.scene.lines.layers.pop();
            state.scene.lines.layers.insert(index + 1, copy);
            if let Some(base) = state
                .scene
                .clipping
                .binding(crate::layer_properties::Target::Vector(id))
            {
                state
                    .scene
                    .clipping
                    .bind(crate::layer_properties::Target::Vector(to), Some(base));
            }
            if let Some(p) = state.scene.layer_properties.vectors.get(&id).copied() {
                state.scene.layer_properties.vectors.insert(to, p);
            }
            if let Some(e) = state.scene.layer_effects.vectors.get(&id).cloned() {
                state.scene.layer_effects.vectors.insert(to, e);
            }
            if let Some(s) = state.scene.appearance.vectors.get(&id).cloned() {
                state.scene.appearance.vectors.insert(to, s);
            }
            if let Some(info) = state.scene.catalog.vectors.get(&id).cloned() {
                state.scene.catalog.vectors.insert(to, info);
            }
            crate::file_objects::duplicate(
                &mut state,
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(id)),
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(to)),
            );
            selected = Target::Vector(to);
        }
        Command::Delete(Target::Raster(id)) => {
            let i = raster(id)?;
            if state.layers.len() == 1 {
                return Err("Keep at least one raster layer".into());
            }
            if state.layers[i].locked {
                return Err("Layer is locked".into());
            }
            let map: Vec<_> = (0..state.layers.len())
                .map(|n| {
                    if n == i {
                        None
                    } else {
                        Some(if n > i { n - 1 } else { n })
                    }
                })
                .collect();
            state.layers.remove(i);
            state.scene.layer_properties.rasters.remove(&id);
            state.scene.layer_effects.rasters.remove(&id);
            state.scene.appearance.rasters.remove(&id);
            state.scene.catalog.rasters.remove(&id);
            state
                .scene
                .clipping
                .remove_owner(crate::layer_properties::Target::Raster(id));
            remap(&mut state.scene, &map);
            selected = Target::Raster(state.scene.layer_ids.id(i.min(state.layers.len() - 1)));
        }
        Command::Delete(Target::Vector(id)) => {
            let i = state
                .scene
                .lines
                .layers
                .iter()
                .position(|l| l.id == id)
                .ok_or("Missing vector layer")?;
            if state.scene.lines.layers[i].locked {
                return Err("Layer is locked".into());
            }
            let paths: Vec<_> = state.scene.lines.layers[i]
                .paths
                .iter()
                .map(|p| p.id)
                .collect();
            state.scene.lines.layers.remove(i);
            state.scene.layer_properties.vectors.remove(&id);
            state.scene.layer_effects.vectors.remove(&id);
            state.scene.appearance.vectors.remove(&id);
            state.scene.catalog.vectors.remove(&id);
            state
                .scene
                .clipping
                .remove_owner(crate::layer_properties::Target::Vector(id));
            state.scene.lines.prune_selection();
            state
                .scene
                .lines
                .settings
                .intersections
                .retain(|n| *n != id);
            state
                .scene
                .lines
                .settings
                .references
                .retain(|t| *t != crate::line_tools::Target::Vector(id));
            state
                .scene
                .lines
                .settings
                .erase
                .retain(|t| *t != crate::line_tools::Target::Vector(id));
            state.scene.rulers.guides.retain(|g| {
                g.link != Some(crate::line_tools::Target::Vector(id))
                    && g.scope
                        != crate::ruler_scope::Scope::Layer(crate::line_tools::Target::Vector(id))
            });
            if state
                .scene
                .rulers
                .active
                .is_some_and(|id| !state.scene.rulers.guides.iter().any(|g| g.id == id))
            {
                state.scene.rulers.active = None;
            }
            state
                .scene
                .alignment
                .selected
                .retain(|i| !matches!(i,crate::alignment::Item::Vector(id) if paths.contains(id)));
            if matches!(state.scene.alignment.reference,crate::alignment::Reference::Key(crate::alignment::Item::Vector(id)) if paths.contains(&id))
            {
                state.scene.alignment.reference = Default::default();
            }
        }
        Command::Move {
            target: Target::Raster(id),
            parent: p,
            position,
        } => {
            let i = raster(id)?;
            if state.layers[i].locked {
                return Err("Layer is locked".into());
            }
            materialize(&mut state.scene.paper, state.layers.len());
            detach(&mut state.scene.paper, i);
            insert(&mut state.scene.paper, i, p, position)?;
            state.scene.render_order.entries.retain(|e| {
                e.key
                    != crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(id))
            });
            selected = Target::Raster(id);
        }
        Command::Move {
            target: Target::Vector(id),
            parent: p,
            position,
        } => {
            if p.is_some() {
                return Err("Vector layers use their own vector stack".into());
            }
            let i = state
                .scene
                .lines
                .layers
                .iter()
                .position(|l| l.id == id)
                .ok_or("Missing vector layer")?;
            if state.scene.lines.layers[i].locked || position >= state.scene.lines.layers.len() {
                return Err("Locked layer or invalid vector position".into());
            }
            let l = state.scene.lines.layers.remove(i);
            state.scene.lines.layers.insert(position, l);
            crate::render_order::move_vector(&mut state.scene);
            selected = Target::Vector(id);
        }
    }
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || !state.scene.layer_ids.matches(state.layers.len())
        || state.memory_bytes().saturating_add(doc.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Layer change exceeds document/history limits".into());
    }
    Ok(Prepared { state, selected })
}
