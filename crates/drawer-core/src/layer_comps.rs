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
    #[serde(default, skip_serializing_if = "Policy::is_all")]
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    #[default]
    All,
    Current,
}
impl Policy {
    fn is_all(&self) -> bool {
        *self == Self::All
    }
    pub fn load(storage: &crate::storage::UserStorage) -> Result<Self, String> {
        let p = storage.root().join("settings/layer-comp-policy.json");
        if !p.exists() {
            return Ok(Self::All);
        }
        let bytes = crate::asset_catalog::bytes(&p, 1024)?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }
    pub fn save(self, storage: &crate::storage::UserStorage) -> Result<(), String> {
        use std::io::Write;
        let p = storage.root().join("settings/layer-comp-policy.json");
        let temp = p.with_extension("json.write.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(&self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temp, p).map_err(|e| e.to_string())
        })();
        if temp.exists() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
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
        self.current
            .is_none_or(|id| self.comps.iter().any(|c| c.id == id))
            && self.comps.len() <= 64
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
    if d.snapshot_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Layer comp snapshot exceeds document budget".into());
    }
    if matches!(command, Command::Capture { .. } | Command::Apply(_))
        && (d.scene.clipping.solo.is_some() || d.scene.layer_groups.solo.is_some())
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
            result.state.scene.layer_comps.current = Some(id);
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
            if data.current == Some(id) {
                data.current = None;
            }
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
            result.state.scene.layer_comps.current = Some(id);
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
fn identities(
    scene: &crate::scene::Scene,
    count: usize,
    animation: Option<&crate::animation::Timeline>,
) -> BTreeSet<Key> {
    let mut keys = (0..count)
        .map(|n| Key::Layer(Target::Raster(scene.layer_ids.id(n))))
        .collect::<BTreeSet<_>>();
    keys.extend(
        scene
            .lines
            .layers
            .iter()
            .map(|l| Key::Layer(Target::Vector(l.id))),
    );
    keys.extend(
        (0..scene.paper.folders.len())
            .filter(|n| scene.paper.folder_active(*n))
            .map(|n| Key::Layer(Target::Folder(n as u16))),
    );
    keys.extend((0..scene.objects.len()).map(|n| Key::Object(scene.catalog.objects.id(n))));
    keys.extend((0..scene.worlds.len()).map(|n| Key::World(scene.catalog.worlds.id(n))));
    keys.extend(scene.gradients.layers.iter().map(|l| Key::Gradient(l.id)));
    if let Some(a) = animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let track = scene.catalog.tracks.id(n);
            keys.insert(Key::Track(track));
            for (i, _) in t.cells.iter().enumerate() {
                keys.insert(Key::Cell {
                    track,
                    cell: scene
                        .catalog
                        .cells
                        .get(&track)
                        .map_or(i as u64 + 1, |ids| ids.id(i)),
                });
            }
        }
    }
    keys
}
/// Register creation identities centrally, including new folders, duplicates and paste.
pub(crate) fn reconcile(d: &Document, state: &mut DocumentState) -> Result<(), String> {
    if state.scene.layer_comps.comps.is_empty() {
        return Ok(());
    }
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || !state
            .scene
            .catalog
            .valid_animation(state.animation.as_ref())
    {
        return Err("Invalid layer-comp creation source".into());
    }
    let before = identities(&d.scene, d.layers.len(), d.animation.as_ref());
    let next = identities(&state.scene, state.layers.len(), state.animation.as_ref());
    if next.is_subset(&before) {
        return Ok(());
    }
    let after = crate::layer_merge::preview(state, d.current_frame)?;
    let mut added = vec![];
    for (key, t) in live(&after)? {
        if !before.contains(&key) {
            let p = properties::properties(&after, t);
            added.push(Entry {
                key,
                hidden: false,
                opacity: p.opacity,
                blend: p.blend,
                through: if let Target::Folder(n) = t {
                    Some(after.scene.paper.folders[n as usize].through)
                } else {
                    None
                },
            });
        }
    }
    if added.is_empty() {
        return Ok(());
    }
    let data = &mut state.scene.layer_comps;
    if data.policy == Policy::Current && data.current.is_none() {
        return Err(
            "Apply a saved comp before creating layers with Current-comp-only policy".into(),
        );
    }
    for comp in &mut data.comps {
        for entry in &added {
            let mut entry = entry.clone();
            entry.hidden = data.policy == Policy::Current && data.current != Some(comp.id);
            if let Some(old) = comp.entries.iter_mut().find(|v| v.key == entry.key) {
                *old = entry;
            } else {
                comp.entries.push(entry);
            }
        }
    }
    if !data.valid() {
        return Err("Layer-comp creation entries exceed limits".into());
    }
    Ok(())
}
