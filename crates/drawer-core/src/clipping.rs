//! Persistent alpha protection and explicit same-stack clipping bases.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as properties, Target},
    paper::{Graph, Node},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Solo {
    pub folder: u16,
    pub hidden: Vec<(Target, bool)>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub alpha: BTreeSet<Target>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rasters: BTreeMap<u64, Target>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vectors: BTreeMap<u64, Target>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub folders: BTreeMap<u16, Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<Solo>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn graph_active(&self) -> bool {
        !self.rasters.is_empty() || !self.folders.is_empty()
    }
    pub fn valid(&self) -> bool {
        let key = |t: Target| match t {
            Target::Raster(n) | Target::Vector(n) => n > 0,
            Target::Folder(n) => n < 256,
            _ => false,
        };
        self.rasters.len() <= 256
            && self.vectors.len() <= 32
            && self.folders.len() <= 256
            && self.alpha.len() <= 10256
            && self.alpha.iter().all(|t| {
                matches!(*t,Target::Raster(n) if n>0)
                    || matches!(*t,Target::Cell {track,cell} if track<32&&cell<10000)
            })
            && self
                .rasters
                .iter()
                .all(|(n, t)| *n > 0 && key(*t) && *t != Target::Raster(*n))
            && self
                .vectors
                .iter()
                .all(|(n, t)| *n > 0 && matches!(*t,Target::Vector(id) if id>0&&id!=*n))
            && self
                .folders
                .iter()
                .all(|(n, t)| *n < 256 && key(*t) && *t != Target::Folder(*n))
            && self.solo.as_ref().is_none_or(|s| {
                s.folder < 256
                    && s.hidden.len() <= 576
                    && s.hidden
                        .iter()
                        .all(|(t, _)| key(*t) || matches!(*t,Target::Track(n) if n<32))
                    && s.hidden
                        .iter()
                        .map(|(t, _)| *t)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == s.hidden.len()
            })
    }
    pub fn bytes(&self) -> usize {
        self.alpha.len() * 48
            + (self.rasters.len() + self.vectors.len() + self.folders.len()) * 64
            + self.solo.as_ref().map_or(0, |s| s.hidden.len() * 32)
    }
    pub fn binding(&self, t: Target) -> Option<Target> {
        match t {
            Target::Raster(n) => self.rasters.get(&n).copied(),
            Target::Vector(n) => self.vectors.get(&n).copied(),
            Target::Folder(n) => self.folders.get(&n).copied(),
            _ => None,
        }
    }
    pub fn bind(&mut self, t: Target, base: Option<Target>) {
        macro_rules! set {
            ($map:expr,$n:expr) => {
                if let Some(base) = base {
                    $map.insert($n, base);
                } else {
                    $map.remove(&$n);
                }
            };
        }
        match t {
            Target::Raster(n) => set!(self.rasters, n),
            Target::Vector(n) => set!(self.vectors, n),
            Target::Folder(n) => set!(self.folders, n),
            _ => {}
        }
    }
    pub fn remove_owner(&mut self, t: Target) {
        self.alpha.remove(&t);
        self.bind(t, None);
    }
}
pub fn alpha_locked(doc: &Document, n: usize) -> bool {
    doc.scene
        .clipping
        .alpha
        .contains(&properties::raster(doc, n))
}
pub fn preserve(before: [u8; 4], mut after: [u8; 4]) -> [u8; 4] {
    if before[3] == 0 {
        before
    } else {
        after[3] = before[3];
        after
    }
}
pub fn node_key(doc: &Document, n: Node) -> Target {
    match n {
        Node::Raster(n) => properties::raster(doc, n),
        Node::Folder(n) => Target::Folder(n as u16),
    }
}
pub fn node(doc: &Document, t: Target) -> Option<Node> {
    match t {
        Target::Raster(id) => crate::layers::index(doc, id).map(Node::Raster),
        Target::Folder(n) => {
            ((n as usize) < doc.scene.paper.folders.len()).then_some(Node::Folder(n as usize))
        }
        _ => None,
    }
}
pub fn siblings(doc: &Document, t: Target) -> Result<Vec<Target>, String> {
    if let Target::Vector(_) = t {
        return Ok(doc
            .scene
            .lines
            .layers
            .iter()
            .map(|l| Target::Vector(l.id))
            .collect());
    }
    let n = node(doc, t).ok_or("Clipping owner is missing")?;
    let nodes = if let Some(parent) = doc
        .scene
        .paper
        .folders
        .iter()
        .find(|f| f.children.contains(&n))
    {
        parent.children.clone()
    } else {
        doc.scene.paper.root_nodes(doc.layers.len())
    };
    Ok(nodes.into_iter().map(|n| node_key(doc, n)).collect())
}
pub fn below(
    doc: &Document,
    data: &Data,
    t: Target,
    skip: &BTreeSet<Target>,
) -> Result<Target, String> {
    if let Target::Folder(n) = t
        && doc.scene.paper.folders[n as usize].through
    {
        return Err("A through folder cannot be a clipping owner".into());
    }
    let list = siblings(doc, t)?;
    let i = list
        .iter()
        .position(|v| *v == t)
        .ok_or("Place the owner in the layer tree before clipping")?;
    let base = list[..i]
        .iter()
        .rev()
        .find(|n| data.binding(**n).is_none() && !skip.contains(n))
        .copied()
        .ok_or("No unclipped base below this layer in its stack")?;
    if let Target::Folder(n) = base
        && doc.scene.paper.folders[n as usize].through
    {
        return Err("Choose an isolated folder or raster as the clipping base".into());
    }
    if crate::layer_effects::adjustment(doc, base).is_some() {
        return Err("Choose an image or isolated folder as the clipping base".into());
    }
    Ok(base)
}
pub fn active(doc: &Document, t: Target) -> bool {
    let Some(base) = doc.scene.clipping.binding(t) else {
        return false;
    };
    let Ok(list) = siblings(doc, t) else {
        return false;
    };
    let Some(i) = list.iter().position(|n| *n == t) else {
        return false;
    };
    list[..i]
        .iter()
        .rev()
        .find(|n| doc.scene.clipping.binding(**n).is_none())
        .is_some_and(|n| *n == base)
}
fn finish(doc: &Document, state: DocumentState) -> Result<DocumentState, String> {
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || state.memory_bytes().saturating_add(doc.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Clipping/solo state exceeds document limits".into());
    }
    Ok(state)
}
pub fn prepare(
    doc: &Document,
    selected: &[Target],
    alpha: Option<bool>,
    clip: Option<bool>,
) -> Result<DocumentState, String> {
    if doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || selected.is_empty()
        || selected.len() > 11000
        || selected.iter().collect::<BTreeSet<_>>().len() != selected.len()
    {
        return Err("Choose unique layers within the snapshot budget".into());
    }
    let available = properties::targets(doc)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if selected.iter().any(|t| !available.contains(t)) {
        return Err("Clipping targets changed; reopen the draft".into());
    }
    let mut state = doc.snapshot();
    if alpha.is_none() && clip.is_none() {
        return Ok(state);
    }
    state.scene.layer_ids.initialize(doc.layers.len())?;
    let skip = selected.iter().copied().collect::<BTreeSet<_>>();
    for t in selected {
        if let Some(enabled) = alpha
            && matches!(t, Target::Raster(_) | Target::Cell { .. })
        {
            if enabled {
                state.scene.clipping.alpha.insert(*t);
            } else {
                state.scene.clipping.alpha.remove(t);
            }
        }
        if let Some(enabled) = clip
            && matches!(t, Target::Raster(_) | Target::Vector(_) | Target::Folder(_))
        {
            let base = if enabled {
                Some(below(doc, &doc.scene.clipping, *t, &skip)?)
            } else {
                None
            };
            state.scene.clipping.bind(*t, base);
        }
    }
    // Explicitly releasing a middle clipped layer makes it the base of the remaining upper run.
    if clip == Some(false) {
        for owner in selected {
            if let Some(old_base) = doc.scene.clipping.binding(*owner) {
                let list = siblings(doc, *owner)?;
                let i = list.iter().position(|t| t == owner).unwrap();
                for t in &list[i + 1..] {
                    if doc.scene.clipping.binding(*t) != Some(old_base) {
                        break;
                    }
                    if !skip.contains(t) {
                        let base = below(doc, &state.scene.clipping, *t, &BTreeSet::new())?;
                        state.scene.clipping.bind(*t, Some(base));
                    }
                }
            }
        }
    }
    if state.scene.clipping == doc.scene.clipping {
        return Ok(doc.snapshot());
    }
    if state.scene.clipping.graph_active() {
        state.scene.paper.roots = state.scene.paper.root_nodes(state.layers.len());
    }
    finish(doc, state)
}
fn contains_folder(g: &Graph, root: usize, target: usize, depth: usize) -> bool {
    root == target
        || depth < 16
            && g.folders.get(root).is_some_and(|f| {
                f.children
                    .iter()
                    .any(|n| matches!(n,Node::Folder(n) if contains_folder(g,*n,target,depth+1)))
            })
}
pub fn solo(doc: &Document, folder: Option<usize>) -> Result<DocumentState, String> {
    if doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Solo snapshot budget".into());
    }
    let mut state = doc.snapshot();
    state.scene.layer_ids.initialize(doc.layers.len())?;
    if let Some(old) = state.scene.clipping.solo.take() {
        let available = properties::targets(doc)
            .into_iter()
            .collect::<BTreeSet<_>>();
        for (t, hidden) in old.hidden {
            if available.contains(&t) {
                let mut p = properties::properties(doc, t);
                p.hidden = hidden;
                properties::set_state(doc, &mut state, t, p)?;
            }
        }
    }
    if let Some(folder) = folder {
        doc.scene
            .paper
            .folders
            .get(folder)
            .ok_or("Choose an existing folder")?;
        let members = doc.scene.paper.raster_indices(Node::Folder(folder));
        let targets = properties::targets(doc)
            .into_iter()
            .filter(|t| {
                matches!(
                    t,
                    Target::Raster(_) | Target::Vector(_) | Target::Folder(_) | Target::Track(_)
                )
            })
            .collect::<Vec<_>>();
        let mut original = vec![];
        for t in targets {
            let p = properties::state_properties(&state, t);
            original.push((t, p.hidden));
            let mut changed = p;
            match t {
                Target::Raster(id) => {
                    if !crate::layers::index(doc, id).is_some_and(|n| members.contains(&n)) {
                        changed.hidden = true;
                    }
                }
                Target::Folder(n) => {
                    let ancestor = contains_folder(&doc.scene.paper, n as usize, folder, 0);
                    let inside = contains_folder(&doc.scene.paper, folder, n as usize, 0);
                    if ancestor {
                        changed.hidden = false;
                    } else if !inside {
                        changed.hidden = true;
                    }
                }
                Target::Vector(_) | Target::Track(_) => changed.hidden = true,
                _ => {}
            }
            properties::set_state(doc, &mut state, t, changed)?;
        }
        state.scene.clipping.solo = Some(Solo {
            folder: folder as u16,
            hidden: original,
        });
    }
    finish(doc, state)
}

/// Verify alpha-preserving content edits while allowing whole-layer/page geometry changes.
pub fn validate_state(doc: &Document, after: &DocumentState) -> Result<(), String> {
    if doc.scene.clipping.alpha.is_empty()
        || doc.width != after.width
        || doc.height != after.height
        || doc.scene.transform != after.scene.transform
    {
        return Ok(());
    }
    let mut work = 0usize;
    for (n, old) in doc.layers.iter().enumerate() {
        if !alpha_locked(doc, n) {
            continue;
        }
        let id = doc.scene.layer_ids.id(n);
        let Some(new) = (0..after.layers.len())
            .find(|i| after.scene.layer_ids.id(*i) == id)
            .map(|i| &after.layers[i])
        else {
            continue;
        };
        if old.pixels == new.pixels {
            continue;
        }
        if !new.valid(after.width, after.height) {
            return Err("Invalid alpha-protected raster".into());
        }
        work = work.saturating_add(old.pixels.len() + new.pixels.len());
        if work > 64 * 1024 * 1024 {
            return Err("Alpha protection work limit".into());
        }
        for (layer, other) in [(old, new), (new, old)] {
            let e = layer.extent(doc.width, doc.height);
            for (i, p) in layer.pixels.iter().enumerate() {
                let x = e.offset[0] + (i % e.width as usize) as i64;
                let y = e.offset[1] + (i / e.width as usize) as i64;
                if p[3] != other.pixel(doc.width, doc.height, x, y)[3] {
                    return Err(
                        "Layer alpha is protected; unlock transparency before this content edit"
                            .into(),
                    );
                }
            }
        }
    }
    for t in &doc.scene.clipping.alpha {
        if let Target::Cell { track, cell } = *t {
            let old = doc
                .animation
                .as_ref()
                .and_then(|a| a.tracks.get(track as usize))
                .and_then(|t| t.cells.get(cell as usize));
            let track_id = doc.scene.catalog.tracks.id(track as usize);
            let cell_id = doc.scene.catalog.cell_id(track_id, cell as usize);
            let new = if let Some(Target::Cell { track, cell }) = crate::layer_catalog::state_target(
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
            if let (Some(old), Some(new)) = (old, new)
                && old.width == new.width
                && old.height == new.height
                && old.offset == new.offset
                && old
                    .pixels
                    .iter()
                    .zip(new.pixels.iter())
                    .any(|(a, b)| a[3] != b[3])
            {
                return Err("Cell alpha is protected".into());
            }
        }
    }
    Ok(())
}
