//! Stable, non-hierarchical layer linkage. Every prepared edit remains atomic.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_search::Key,
    paper::Node,
    scene::Scene,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transforms: Vec<Frame>,
    pub next: u64,
    pub groups: Vec<Group>,
    pub solo: Option<Vec<(Key, bool)>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub key: Key,
    pub matrix: crate::scene::Affine,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub members: Vec<Key>,
    pub linked: [bool; 3],
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Move,
    Transform,
    Filter,
    Visibility,
}
impl Operation {
    fn index(self) -> usize {
        match self {
            Self::Move => 0,
            Self::Transform => 1,
            Self::Filter => 2,
            Self::Visibility => 0,
        }
    }
}
pub fn canonical(k: Key) -> bool {
    match k {
        Key::Layer(Target::Raster(id) | Target::Vector(id))
        | Key::Object(id)
        | Key::Gradient(id)
        | Key::World(id)
        | Key::Track(id) => id > 0,
        Key::Layer(Target::Folder(n)) => n < 256,
        Key::Cell { track, cell } => track > 0 && cell > 0,
        _ => false,
    }
}
impl Data {
    pub fn frame(&self, key: Key) -> Option<crate::scene::Affine> {
        self.transforms
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.matrix)
    }
    pub(crate) fn set_frame(&mut self, key: Key, matrix: crate::scene::Affine) {
        if let Some(f) = self.transforms.iter_mut().find(|f| f.key == key) {
            f.matrix = matrix;
        } else {
            self.transforms.push(Frame { key, matrix });
        }
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        self.transforms.len() <= 256
            && self
                .transforms
                .iter()
                .map(|f| f.key)
                .collect::<BTreeSet<_>>()
                .len()
                == self.transforms.len()
            && self
                .transforms
                .iter()
                .all(|f| canonical(f.key) && f.matrix.valid())
            && self.groups.len() <= 64
            && self.groups.iter().all(|g| {
                g.id > 0
                    && g.id <= self.next
                    && ids.insert(g.id)
                    && !g.name.trim().is_empty()
                    && g.name.chars().count() <= 64
                    && !g.name.chars().any(char::is_control)
                    && (2..=256).contains(&g.members.len())
                    && g.members.iter().all(|k| canonical(*k) && keys.insert(*k))
            })
            && self.solo.as_ref().is_none_or(|s| {
                s.len() <= 16384
                    && s.iter().map(|(k, _)| *k).collect::<BTreeSet<_>>().len() == s.len()
                    && s.iter().all(|(k, _)| canonical(*k))
            })
    }
    pub fn bytes(&self) -> usize {
        self.transforms.len()
            * (std::mem::size_of::<Key>() + std::mem::size_of::<crate::scene::Affine>() + 32)
            + self
                .groups
                .iter()
                .map(|g| 128 + g.name.len() + g.members.len() * std::mem::size_of::<Key>())
                .sum::<usize>()
            + self
                .solo
                .as_ref()
                .map_or(0, |s| s.len() * std::mem::size_of::<(Key, bool)>())
    }
}
pub enum Command {
    Save {
        id: Option<u64>,
        name: String,
        members: Vec<Key>,
        linked: [bool; 3],
    },
    Delete(u64),
    Visibility {
        members: Vec<Key>,
        hidden: bool,
    },
    Solo(Vec<Key>),
    Restore,
}
pub fn live(d: &Document) -> Result<BTreeMap<Key, Target>, String> {
    Ok(c::target_map(d)
        .into_iter()
        .filter(|(k, _)| canonical(*k))
        .collect())
}

pub fn expanded(d: &Document, selected: &[Key], op: Operation) -> Result<Vec<Key>, String> {
    let live = live(d)?;
    let mut out = selected.iter().copied().collect::<BTreeSet<_>>();
    if out.is_empty() || selected.len() != out.len() || out.iter().any(|k| !live.contains_key(k)) {
        return Err("Choose unique live layers".into());
    }
    for g in &d.scene.layer_groups.groups {
        if (op == Operation::Visibility || g.linked[op.index()])
            && g.members.iter().any(|k| out.contains(k))
        {
            if g.members.iter().any(|k| !live.contains_key(k)) {
                return Err("A linked member was removed; refresh the group".into());
            }
            out.extend(g.members.iter().copied());
        }
    }
    if out.len() > 256 {
        return Err("Linked operation exceeds 256 explicit members".into());
    }
    let mut ordered = selected.to_vec();
    ordered.extend(out.into_iter().filter(|k| !selected.contains(k)));
    Ok(ordered)
}
fn inside(d: &Document, parent: Key, child: Key) -> bool {
    if parent == child || crate::comics::frame_contains(d, parent, child) {
        return true;
    }
    if let (Key::Track(a), Key::Cell { track: b, .. }) = (parent, child) {
        return a == b;
    }
    let Key::Layer(Target::Folder(n)) = parent else {
        return false;
    };
    let Key::Layer(t) = child else {
        return false;
    };
    let Some(node) = crate::clipping::node(d, t) else {
        return false;
    };
    fn visit(d: &Document, n: Node, child: Node, depth: usize) -> bool {
        n == child
            || depth < 16
                && matches!(n,Node::Folder(i)if d.scene.paper.folders[i].children.iter().any(|n|visit(d,*n,child,depth+1)))
    }
    visit(d, Node::Folder(n as usize), node, 0)
}
pub fn roots(d: &Document, keys: &[Key]) -> Vec<Key> {
    keys.iter()
        .copied()
        .filter(|k| !keys.iter().any(|p| p != k && inside(d, *p, *k)))
        .collect()
}
pub fn content_members(d: &Document, keys: &[Key]) -> Result<Vec<Key>, String> {
    let all = live(d)?;
    Ok(all
        .keys()
        .copied()
        .filter(|k| {
            keys.iter()
                .any(|p| inside(d, *p, *k) && (*p == *k || d.scene.file_objects.get(*p).is_none()))
        })
        .collect())
}
pub fn preflight(d: &Document, keys: &[Key], op: Operation) -> Result<(), String> {
    if !d.scene.valid() || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid linked document or snapshot limit".into());
    }
    for key in content_members(d, keys)? {
        let t = c::target(d, key).ok_or("Missing group member")?;
        let f = if op == Operation::Transform {
            crate::file_objects::stored_flags(d, t)
        } else {
            p::inherited(d, t)
        };
        if (f.hidden && (op == Operation::Filter || keys.contains(&key)))
            || f.locked
            || (op != Operation::Filter && f.position_locked)
            || (op != Operation::Move && f.pixels_locked)
        {
            return Err(format!(
                "Linked member {key:?} is hidden or locked; no members changed"
            ));
        }
    }
    Ok(())
}
pub fn move_linked(d: &Document, keys: &[Key], delta: [i64; 2]) -> Result<DocumentState, String> {
    let keys = expanded(d, keys, Operation::Move)?;
    preflight(d, &keys, Operation::Move)?;
    if delta == [0; 2] {
        return Ok(d.snapshot());
    }
    let mut state = d.snapshot();
    for key in roots(d, &keys) {
        let working = crate::layer_merge::preview(&state, d.current_frame)?;
        state = crate::search_move::prepare_single(&working, key, delta)?;
    }
    finish(d, &state)?;
    Ok(state)
}
pub fn finish(d: &Document, state: &DocumentState) -> Result<(), String> {
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Linked operation exceeds geometry/document/Undo limits".into());
    }
    crate::clipping::validate_state(d, state)?;
    p::validate_edit(d, state)
}
pub fn prepare(d: &Document, command: Command) -> Result<DocumentState, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Group snapshot limit".into());
    }
    let available = live(d)?;
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    c::reconcile(d, &mut state)?;
    match command {
        Command::Save {
            id,
            name,
            members,
            linked,
        } => {
            if d.scene.layer_groups.solo.is_some() || d.scene.clipping.solo.is_some() {
                return Err("Restore solo visibility before changing groups".into());
            }
            if members.len() != members.iter().collect::<BTreeSet<_>>().len()
                || members.iter().any(|k| !available.contains_key(k))
            {
                return Err("Choose unique live group members".into());
            }
            let mut all = members.clone();
            for g in &state.scene.layer_groups.groups {
                if Some(g.id) != id {
                    all.extend(g.members.iter().copied());
                }
            }
            if all.iter().collect::<BTreeSet<_>>().len() != all.len()
                || all
                    .iter()
                    .filter(|p| matches!(p, Key::Layer(Target::Folder(_)) | Key::Track(_)))
                    .any(|parent| {
                        all.iter()
                            .any(|child| child != parent && inside(d, *parent, *child))
                    })
            {
                return Err("A layer/subtree can belong to only one link group; do not select a folder and its child separately".into());
            }
            let data = &mut state.scene.layer_groups;
            if let Some(id) = id {
                let g = data
                    .groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or("Group was removed")?;
                *g = Group {
                    id,
                    name,
                    members,
                    linked,
                };
            } else {
                let id = data.next.checked_add(1).ok_or("Group ID limit")?;
                data.next = id;
                data.groups.push(Group {
                    id,
                    name,
                    members,
                    linked,
                });
            }
        }
        Command::Delete(id) => {
            let data = &mut state.scene.layer_groups;
            let n = data
                .groups
                .iter()
                .position(|g| g.id == id)
                .ok_or("Group was removed")?;
            data.groups.remove(n);
        }
        Command::Visibility { members, hidden } => {
            if d.scene.layer_groups.solo.is_some() || d.scene.clipping.solo.is_some() {
                return Err("Restore solo visibility first".into());
            }
            let keys = expanded(d, &members, Operation::Visibility)?;
            for k in keys {
                let t = available[&k];
                let mut props = p::properties(d, t);
                props.hidden = hidden;
                p::set_state(d, &mut state, t, props)?;
            }
        }
        Command::Restore | Command::Solo(_) => {
            if d.scene.clipping.solo.is_some() {
                return Err("Restore folder solo first".into());
            }
            if let Some(old) = state.scene.layer_groups.solo.take() {
                for (k, hidden) in old {
                    if let Some(t) = available.get(&k) {
                        let mut props = p::properties(d, *t);
                        props.hidden = hidden;
                        p::set_state(d, &mut state, *t, props)?;
                    }
                }
            }
            if let Command::Solo(selected) = command {
                let keys = expanded(d, &selected, Operation::Visibility)?;
                let working = crate::layer_merge::preview(&state, d.current_frame)?;
                let mut original = vec![];
                for (k, t) in available {
                    let mut props = p::properties(&working, t);
                    original.push((k, props.hidden));
                    if keys.contains(&k) || keys.iter().any(|p| inside(d, k, *p)) {
                        props.hidden = false;
                    } else if !keys.iter().any(|p| inside(d, *p, k)) {
                        props.hidden = true;
                    }
                    p::set_state(d, &mut state, t, props)?;
                }
                state.scene.layer_groups.solo = Some(original);
            }
        }
    }
    finish(d, &state)?;
    Ok(state)
}
pub(crate) fn prune(state: &mut DocumentState) -> Result<(), String> {
    crate::comics::prune(state);
    if state.scene.layer_groups.groups.is_empty() && state.scene.layer_groups.transforms.is_empty()
    {
        return Ok(());
    }
    let d = crate::layer_merge::preview(state, 0)?;
    let keys = live(&d)?.into_keys().collect::<BTreeSet<_>>();
    state
        .scene
        .layer_groups
        .transforms
        .retain(|f| keys.contains(&f.key));
    for g in &mut state.scene.layer_groups.groups {
        g.members.retain(|k| keys.contains(k));
    }
    state
        .scene
        .layer_groups
        .groups
        .retain(|g| g.members.len() >= 2);
    Ok(())
}
pub fn requires_v59(s: &Scene) -> bool {
    !s.layer_groups.is_default()
        || s.paper.layers.values().any(|p| !p.space.is_identity())
        || s.paper.folders.iter().any(|f| !f.paper.space.is_identity())
        || s.layer_effects
            .rasters
            .values()
            .chain(s.layer_effects.vectors.values())
            .chain(s.layer_effects.folders.values())
            .any(|e| e.mask.as_ref().is_some_and(|m| !m.space.is_identity()))
        || s.appearance
            .rasters
            .values()
            .chain(s.appearance.vectors.values())
            .any(|a| !a.scope_space.is_identity())
        || s.rulers.guides.iter().any(|g| !g.space.is_identity())
}

pub fn valid_live(d: &Document) -> bool {
    live(d).is_ok_and(|live| {
        state_transforms_live(&d.scene.layer_groups, &live)
            && d.scene
                .layer_groups
                .groups
                .iter()
                .all(|g| g.members.iter().all(|k| live.contains_key(k)))
    })
}

fn state_transforms_live(data: &Data, live: &BTreeMap<Key, Target>) -> bool {
    data.transforms.iter().all(|f| live.contains_key(&f.key))
}
