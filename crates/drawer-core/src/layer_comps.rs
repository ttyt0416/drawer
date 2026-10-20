//! Named snapshots of each layer's own display properties, addressed by stable IDs.
use crate::sizing::DocumentState;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_blend::Mode,
    layer_catalog as catalog,
    layer_properties::{self as properties, Target},
    layer_search::Key,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub comps: Vec<Comp>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comp {
    pub id: u64,
    pub name: String,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub key: Key,
    pub hidden: bool,
    pub opacity: u8,
    pub blend: Mode,
    pub through: Option<bool>,
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}
fn key_valid(key: Key) -> bool {
    match key {
        Key::Layer(Target::Raster(id) | Target::Vector(id))
        | Key::Object(id)
        | Key::Gradient(id)
        | Key::World(id)
        | Key::Track(id) => id > 0,
        Key::Layer(Target::Folder(id)) => id < 256,
        Key::Cell { track, cell } => track > 0 && cell > 0,
        _ => false,
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.comps.len() <= 64
            && self.comps.iter().map(|c| c.entries.len()).sum::<usize>() <= 32768
            && self
                .comps
                .iter()
                .map(|c| c.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.comps.len()
            && self.comps.iter().all(|c| {
                c.id > 0
                    && c.id <= self.next
                    && name_valid(&c.name)
                    && c.entries.len() <= 16384
                    && c.entries
                        .iter()
                        .map(|e| e.key)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == c.entries.len()
                    && c.entries.iter().all(|e| {
                        key_valid(e.key)
                            && e.opacity <= 100
                            && (matches!(e.key, Key::Layer(Target::Folder(_)))
                                == e.through.is_some())
                            && (!e.through.unwrap_or(false) || e.blend.is_normal())
                            && (!matches!(e.key, Key::Cell { .. }) || e.blend.is_normal())
                    })
            })
    }
    pub fn bytes(&self) -> usize {
        self.comps
            .iter()
            .map(|c| {
                std::mem::size_of::<Comp>()
                    + c.name.len()
                    + c.entries.len() * std::mem::size_of::<Entry>()
            })
            .sum()
    }
}
pub enum Command {
    Capture { name: String, replace: Option<u64> },
    Rename { id: u64, name: String },
    Delete(u64),
    Apply(u64),
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Option<u64>,
    pub missing: Vec<Key>,
    pub unrecorded: usize,
    pub changed: usize,
}
fn live(d: &Document) -> Result<BTreeMap<Key, Target>, String> {
    Ok(catalog::rows(d)?
        .into_iter()
        .filter(|r| key_valid(r.key))
        .filter_map(|r| catalog::target(d, r.key).map(|t| (r.key, t)))
        .collect())
}
pub fn prepare(d: &Document, command: Command) -> Result<Prepared, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Layer comp snapshot exceeds document budget".into());
    }
    if matches!(command, Command::Capture { .. } | Command::Apply(_))
        && d.scene.clipping.solo.is_some()
    {
        return Err("Restore folder solo before saving or applying a comp".into());
    }
    let mut result = Prepared {
        state: d.snapshot(),
        selected: None,
        missing: vec![],
        unrecorded: 0,
        changed: 0,
    };
    result.state.scene.layer_ids.initialize(d.layers.len())?;
    catalog::reconcile(d, &mut result.state)?;
    match command {
        Command::Capture { name, replace } => {
            if !name_valid(&name) {
                return Err("Use a printable comp name of 1 to 64 characters".into());
            }
            let entries = live(d)?
                .into_iter()
                .map(|(key, t)| {
                    let p = properties::properties(d, t);
                    Entry {
                        key,
                        hidden: p.hidden,
                        opacity: p.opacity,
                        blend: p.blend,
                        through: if let Target::Folder(n) = t {
                            Some(d.scene.paper.folders[n as usize].through)
                        } else {
                            None
                        },
                    }
                })
                .collect();
            let data = &mut result.state.scene.layer_comps;
            let id = if let Some(id) = replace {
                let c = data
                    .comps
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or("Comp was deleted")?;
                *c = Comp { id, name, entries };
                id
            } else {
                let id = data.next.checked_add(1).ok_or("Comp ID limit")?;
                data.next = id;
                data.comps.push(Comp { id, name, entries });
                id
            };
            result.selected = Some(id);
        }
        Command::Rename { id, name } => {
            if !name_valid(&name) {
                return Err("Use a printable comp name of 1 to 64 characters".into());
            }
            result
                .state
                .scene
                .layer_comps
                .comps
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Comp was deleted")?
                .name = name;
            result.selected = Some(id);
        }
        Command::Delete(id) => {
            let data = &mut result.state.scene.layer_comps;
            let n = data
                .comps
                .iter()
                .position(|c| c.id == id)
                .ok_or("Comp was deleted")?;
            data.comps.remove(n);
        }
        Command::Apply(id) => {
            let comp = d
                .scene
                .layer_comps
                .comps
                .iter()
                .find(|c| c.id == id)
                .ok_or("Choose a saved comp")?;
            let live = live(d)?;
            let recorded = comp.entries.iter().map(|e| e.key).collect::<BTreeSet<_>>();
            result.unrecorded = live.keys().filter(|k| !recorded.contains(k)).count();
            for entry in &comp.entries {
                let Some(&t) = live.get(&entry.key) else {
                    result.missing.push(entry.key);
                    continue;
                };
                let old = properties::properties(d, t);
                let mut p = old;
                p.hidden = entry.hidden;
                p.opacity = entry.opacity;
                p.blend = entry.blend;
                let through_changed = if let Target::Folder(n) = t {
                    entry.through != Some(d.scene.paper.folders[n as usize].through)
                } else {
                    false
                };
                if p != old || through_changed {
                    result.changed += 1;
                }
                properties::set_state(d, &mut result.state, t, p)?;
                if let (Target::Folder(n), Some(through)) = (t, entry.through) {
                    if through
                        && (d.scene.clipping.binding(t).is_some()
                            || live.values().any(|other| {
                                crate::clipping::active(d, *other)
                                    && d.scene.clipping.binding(*other) == Some(t)
                            }))
                    {
                        return Err(
                            "Unclip the folder and its run before restoring a through comp".into(),
                        );
                    }
                    result.state.scene.paper.folders[n as usize].through = through;
                }
            }
            result.selected = Some(id);
        }
    }
    if !result.state.scene.valid()
        || !result
            .state
            .scene
            .paper
            .valid_for(result.state.layers.len())
        || serde_json::to_vec(&result.state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Comp exceeds scene limits or conflicts with folder paper/isolation".into());
    }
    properties::validate_edit(d, &result.state)?;
    Ok(result)
}
