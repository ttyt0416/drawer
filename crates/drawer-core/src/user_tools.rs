//! Local user tools are independent preset instances with stable IDs and transactional edits.
use crate::{
    assets::BrushTip,
    brush_presets::Preset,
    tool_properties::{Property, State, Tool},
    workspaces::Snapshot,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::Path,
};
const FILE_LIMIT: usize = 4 * 1024 * 1024;
fn name_valid(s: &str) -> bool {
    !s.is_empty() && s == s.trim() && s.chars().count() <= 64 && !s.chars().any(char::is_control)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: u64,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub category: u64,
    pub preset: Preset,
    pub default: Preset,
    pub default_properties: Vec<Property>,
    pub properties: Vec<Property>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cached {
    preset: Preset,
    properties: BTreeMap<Property, State>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    version: u32,
    next_id: u64,
    pub categories: Vec<Category>,
    pub tools: Vec<Entry>,
    pub active: Option<u64>,
    defaults: Box<[Option<Preset>; 7]>,
    cache: Box<[Option<Cached>; 7]>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 2,
            categories: vec![Category {
                id: 1,
                name: "My tools".into(),
            }],
            tools: vec![],
            active: None,
            defaults: Box::new(std::array::from_fn(|_| None)),
            cache: Box::new(std::array::from_fn(|_| None)),
        }
    }
}
fn slot(tip: BrushTip) -> usize {
    BrushTip::ALL.iter().position(|t| *t == tip).unwrap()
}
pub fn command_id(s: &str) -> Option<u64> {
    s.strip_prefix("local-tool:")?
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
}
pub fn command(id: u64) -> String {
    format!("local-tool:{id}")
}
impl Library {
    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.tools.iter().find(|t| t.id == id)
    }
    pub fn active_entry(&self) -> Option<&Entry> {
        self.active.and_then(|id| self.entry(id))
    }
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let mut cats = HashSet::new();
        let mut names = HashSet::new();
        self.version == 1
            && self.next_id > 0
            && self.categories.len() <= 32
            && self.tools.len() <= 64
            && self.categories.iter().all(|c| {
                c.id > 0
                    && c.id < self.next_id
                    && ids.insert(c.id)
                    && name_valid(&c.name)
                    && cats.insert(c.name.to_lowercase())
            })
            && self.tools.iter().all(|t| {
                t.id > 0
                    && t.id < self.next_id
                    && ids.insert(t.id)
                    && self.categories.iter().any(|c| c.id == t.category)
                    && names.insert((t.category, t.preset.name.to_lowercase()))
                    && t.preset.validate().is_ok()
                    && t.default.validate().is_ok()
                    && t.preset.name == t.default.name
                    && t.preset.tip == t.default.tip
                    && t.default_properties.len() <= 7
                    && t.default_properties
                        .iter()
                        .enumerate()
                        .all(|(n, p)| !t.default_properties[..n].contains(p))
                    && t.properties.len() <= 7
                    && t.properties
                        .iter()
                        .enumerate()
                        .all(|(n, p)| !t.properties[..n].contains(p))
            })
            && self.defaults.iter().enumerate().all(|(n, p)| {
                p.as_ref()
                    .is_none_or(|p| p.validate().is_ok() && slot(p.tip) == n)
            })
            && self.active.is_none_or(|id| self.entry(id).is_some())
            && self.cache.iter().enumerate().all(|(n, c)| match c {
                Some(c) => {
                    self.active_entry().is_some_and(|a| slot(a.preset.tip) == n)
                        && c.preset.validate().is_ok()
                        && slot(c.preset.tip) == n
                        && c.properties.len() == 7
                        && Property::ALL.iter().enumerate().all(|(i, p)| {
                            c.properties
                                .get(p)
                                .is_some_and(|state| state.value == c.preset.values[i])
                        })
                }
                None => self.active_entry().is_none_or(|a| slot(a.preset.tip) != n),
            })
    }
    fn allocate(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or("User tool ID limit")?;
        Ok(id)
    }
    fn edit<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if !self.valid() {
            return Err("Invalid user tool library".into());
        }
        let mut next = self.clone();
        let value = f(&mut next)?;
        for old in self.tools.iter().filter(|t| t.preset.locked) {
            if let Some(new) = next.entry(old.id) {
                let mut expected = old.preset.clone();
                expected.locked = new.preset.locked;
                if new.preset != expected
                    || new.default != old.default
                    || new.properties != old.properties
                    || new.default_properties != old.default_properties
                {
                    return Err("Unlock this stored user tool before changing its preset".into());
                }
            }
        }
        if !next.valid() {
            return Err(
                "Duplicate/invalid name, reference or tool/category capacity exceeded".into(),
            );
        }
        *self = next;
        Ok(value)
    }
    pub fn create_category(&mut self, name: &str) -> Result<u64, String> {
        self.edit(|next| {
            let id = next.allocate()?;
            next.categories.push(Category {
                id,
                name: name.trim().into(),
            });
            Ok(id)
        })
    }
    pub fn rename_category(&mut self, id: u64, name: &str) -> Result<(), String> {
        self.edit(|next| {
            next.categories
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Category missing")?
                .name = name.trim().into();
            Ok(())
        })
    }
    pub fn create_tool(
        &mut self,
        category: u64,
        preset: Preset,
        properties: Vec<Property>,
    ) -> Result<u64, String> {
        self.edit(|next| {
            let id = next.allocate()?;
            next.tools.push(Entry {
                id,
                category,
                default: preset.clone(),
                preset,
                default_properties: properties.clone(),
                properties,
            });
            Ok(id)
        })
    }
    pub fn rename_tool(&mut self, id: u64, name: &str) -> Result<(), String> {
        self.edit(|next| {
            let t = next
                .tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Tool missing")?;
            t.preset.name = name.trim().into();
            t.default.name = name.trim().into();
            Ok(())
        })
    }
    pub fn capabilities(&mut self, id: u64, properties: Vec<Property>) -> Result<(), String> {
        self.edit(|next| {
            next.tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Tool missing")?
                .properties = properties;
            Ok(())
        })
    }
    fn unique_tool_name(&self, category: u64, base: &str) -> String {
        for n in 1..=128 {
            let suffix = if n == 1 {
                String::new()
            } else {
                format!(" ({n})")
            };
            let prefix = base
                .chars()
                .take(64 - suffix.chars().count())
                .collect::<String>();
            let name = format!("{prefix}{suffix}");
            if !self.tools.iter().any(|t| {
                t.category == category && t.preset.name.to_lowercase() == name.to_lowercase()
            }) {
                return name;
            }
        }
        unreachable!("64 tools leave a free name among 128 candidates")
    }
    pub fn duplicate(&mut self, ids: &[u64], category: u64) -> Result<Vec<u64>, String> {
        if ids.iter().any(|id| self.entry(*id).is_none())
            || ids.len() > 64
            || ids.iter().copied().collect::<HashSet<_>>().len() != ids.len()
        {
            return Err("Select up to64 distinct tools".into());
        }
        self.edit(|next| {
            let mut created = vec![];
            for id in ids {
                let mut t = next.entry(*id).ok_or("Tool missing")?.clone();
                t.id = next.allocate()?;
                let name = next.unique_tool_name(category, &t.preset.name);
                t.category = category;
                t.preset.name = name.clone();
                t.default.name = name;
                created.push(t.id);
                next.tools.push(t);
            }
            Ok(created)
        })
    }
    pub fn move_tools(&mut self, ids: &[u64], category: u64) -> Result<(), String> {
        self.edit(|next| {
            for id in ids {
                next.tools
                    .iter_mut()
                    .find(|t| t.id == *id)
                    .ok_or("Tool missing")?
                    .category = category;
            }
            Ok(())
        })
    }
    pub fn delete_tools(&mut self, ids: &[u64]) -> Result<(), String> {
        self.edit(|next| {
            if ids.iter().any(|id| next.entry(*id).is_none()) {
                return Err("Tool missing".into());
            }
            next.tools.retain(|t| !ids.contains(&t.id));
            if next.active.is_some_and(|id| ids.contains(&id)) {
                next.active = None;
                next.cache.fill(None);
            }
            Ok(())
        })
    }
    pub fn delete_category(&mut self, id: u64, move_to: Option<u64>) -> Result<(), String> {
        self.edit(|next| {
            if !next.categories.iter().any(|c| c.id == id) || move_to == Some(id) {
                return Err("Choose an existing different category".into());
            }
            if let Some(target) = move_to {
                if !next.categories.iter().any(|c| c.id == target) {
                    return Err("Destination category missing".into());
                }
                for t in &mut next.tools {
                    if t.category == id {
                        t.category = target;
                    }
                }
            } else {
                let ids = next
                    .tools
                    .iter()
                    .filter(|t| t.category == id)
                    .map(|t| t.id)
                    .collect::<Vec<_>>();
                next.delete_tools(&ids)?;
            }
            next.categories.retain(|c| c.id != id);
            Ok(())
        })
    }
    /// Drop before a stable target. Copy assigns a new ID and independent preset/default.
    pub fn drop_before(&mut self, source: u64, target: u64, copy: bool) -> Result<u64, String> {
        self.edit(|next| {
            let category = next.entry(target).ok_or("Drop target missing")?.category;
            next.entry(source).ok_or("Drag source missing")?;
            if source == target {
                return Ok(source);
            }
            let id = if copy {
                next.duplicate(&[source], category)?[0]
            } else {
                source
            };
            let n = next.tools.iter().position(|t| t.id == id).unwrap();
            let mut entry = next.tools.remove(n);
            entry.category = category;
            let to = next.tools.iter().position(|t| t.id == target).unwrap();
            next.tools.insert(to, entry);
            Ok(id)
        })
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        if !self.valid() {
            return Err(io::Error::other("Invalid tools"));
        }
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("Tool group exceeds 4 MiB"));
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("Tool group exceeds 4 MiB"));
        }
        let library: Self = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        if !library.valid() {
            return Err(io::Error::other("Invalid tool group"));
        }
        Ok(library)
    }
    pub fn reorder_category(&mut self, id: u64, delta: isize) -> Result<(), String> {
        self.edit(|next| {
            let n = next
                .categories
                .iter()
                .position(|c| c.id == id)
                .ok_or("Category missing")?;
            let target = n
                .checked_add_signed(delta)
                .filter(|n| *n < next.categories.len())
                .ok_or("Category order edge")?;
            next.categories.swap(n, target);
            Ok(())
        })
    }
    pub fn reorder_tool(&mut self, id: u64, delta: isize) -> Result<(), String> {
        self.edit(|next| {
            let t = next.entry(id).ok_or("Tool missing")?;
            let rows = next
                .tools
                .iter()
                .enumerate()
                .filter(|(_, v)| v.category == t.category)
                .map(|(n, _)| n)
                .collect::<Vec<_>>();
            let n = rows.iter().position(|n| next.tools[*n].id == id).unwrap();
            let target = n
                .checked_add_signed(delta)
                .filter(|n| *n < rows.len())
                .ok_or("Tool order edge")?;
            next.tools.swap(rows[n], rows[target]);
            Ok(())
        })
    }
    pub fn register_default(&mut self, id: u64) -> Result<(), String> {
        self.edit(|next| {
            let t = next
                .tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Tool missing")?;
            t.default = t.preset.clone();
            t.default_properties = t.properties.clone();
            Ok(())
        })
    }
    pub fn reset_tool(&mut self, id: u64) -> Result<(), String> {
        if self.entry(id).is_some_and(|t| t.preset.locked) {
            return Err("Unlock this tool before resetting its preset".into());
        }
        self.edit(|next| {
            let t = next
                .tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Tool missing")?;
            t.preset = t.default.clone();
            t.properties = t.default_properties.clone();
            Ok(())
        })
    }
    pub fn register_base_default(&mut self, preset: Preset) -> Result<(), String> {
        self.edit(|next| {
            let n = slot(preset.tip);
            next.defaults[n] = Some(preset);
            Ok(())
        })
    }
    pub fn base_default(&self, current: &Snapshot, tip: BrushTip) -> Result<Preset, String> {
        if let Some(p) = &self.defaults[slot(tip)] {
            return Ok(p.clone());
        }
        let mut base = current.clone();
        crate::basic_brush::restore_input(&mut base, tip);
        crate::basic_brush::restore_values(
            base.properties
                .as_mut()
                .ok_or("Initialize tool properties")?,
            tip,
        )?;
        Preset::capture(&base, tip, &format!("Default {}", tip.label()))
    }
    pub fn lock_tool(
        &mut self,
        id: u64,
        locked: bool,
        preset: Option<Preset>,
    ) -> Result<(), String> {
        self.edit(|next| {
            let t = next
                .tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Tool missing")?;
            if locked
                && !t.preset.locked
                && let Some(p) = preset
            {
                if p.tip != t.preset.tip {
                    return Err("Tool base mismatch".into());
                }
                t.preset = p;
            }
            t.preset.locked = locked;
            Ok(())
        })
    }
    pub fn update_active(&mut self, preset: Preset) -> Result<(), String> {
        self.edit(|next| {
            let id = next.active.ok_or("No active user tool")?;
            let t = next.tools.iter_mut().find(|t| t.id == id).unwrap();
            if t.preset.locked {
                return Err("Locked user preset: temporary settings were not saved".into());
            }
            if t.preset.tip != preset.tip {
                return Err("Active tool base changed".into());
            }
            t.preset = preset;
            Ok(())
        })
    }
    pub fn deactivate(&self, current: &Snapshot) -> Result<Snapshot, String> {
        if !self.valid() {
            return Err("Invalid tool library".into());
        }
        let mut next = current.clone();
        if let Some(t) = self.active_entry() {
            let n = slot(t.preset.tip);
            let cached = self.cache[n]
                .as_ref()
                .ok_or("Missing builtin brush backup")?
                .clone();
            next.user_tools.active = None;
            next.user_tools.cache.fill(None);
            next = cached.preset.apply(&next)?;
            next.properties
                .as_mut()
                .unwrap()
                .tools
                .insert(Tool::brush(cached.preset.tip), cached.properties);
        }
        Ok(next)
    }
    pub fn activate(&self, current: &Snapshot, id: u64) -> Result<Snapshot, String> {
        let mut next = self.deactivate(current)?;
        let t = next
            .user_tools
            .entry(id)
            .ok_or("User tool unavailable")?
            .clone();
        let n = slot(t.preset.tip);
        let saved = Preset::capture(
            &next,
            t.preset.tip,
            &format!("Builtin {}", t.preset.tip.label()),
        )?;
        let properties =
            next.properties.as_ref().unwrap().tools[&Tool::brush(t.preset.tip)].clone();
        next.user_tools.active = Some(id);
        next.user_tools.cache[n] = Some(Cached {
            preset: saved,
            properties,
        });
        let mut effective = t.preset.clone();
        for (n, p) in Property::ALL.iter().enumerate() {
            if !t.properties.contains(p) {
                effective.values[n] = crate::basic_brush::value(Tool::brush(t.preset.tip), *p);
            }
        }
        next = effective.apply(&next)?;
        let props = next
            .properties
            .as_mut()
            .unwrap()
            .tools
            .get_mut(&Tool::brush(t.preset.tip))
            .unwrap();
        for p in Property::ALL {
            props.get_mut(&p).unwrap().visible = t.properties.contains(&p);
        }
        if !next.valid() {
            return Err("User tool activation invalid".into());
        }
        Ok(next)
    }
    pub fn apply_to(&self, current: &Snapshot) -> Result<Snapshot, String> {
        if !self.valid() {
            return Err("Invalid user tools draft".into());
        }
        let previous = current.user_tools.active;
        let desired = self.active;
        let mut next = current.user_tools.deactivate(current)?;
        next.user_tools = self.clone();
        next.user_tools.active = None;
        next.user_tools.cache.fill(None);
        if let Some(id) = desired {
            next = next.user_tools.activate(&next, id)?;
        } else if previous.is_some() {
            let p = Self::default().base_default(&next, BrushTip::Pen)?;
            next = p.apply(&next)?;
        }
        Ok(next)
    }
    pub fn commands(&self) -> Vec<String> {
        self.tools.iter().map(|t| command(t.id)).collect()
    }
    pub fn has_command(&self, s: &str) -> bool {
        command_id(s).is_some_and(|id| self.entry(id).is_some())
    }
    pub fn command_label(&self, s: &str) -> Option<String> {
        self.entry(command_id(s)?).map(|t| {
            format!(
                "{} / {}",
                self.categories
                    .iter()
                    .find(|c| c.id == t.category)
                    .unwrap()
                    .name,
                t.preset.name
            )
        })
    }
    pub fn cycle(
        &self,
        cycle: &crate::actions::Cycle,
        current: &str,
        reverse: bool,
    ) -> Result<String, String> {
        let choices = cycle
            .tools
            .iter()
            .filter(|c| BrushTip::ALL.iter().any(|t| t.name() == *c) || self.has_command(c))
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return Err("No available tools in this cycle".into());
        }
        let n = match choices.iter().position(|c| c.as_str() == current) {
            Some(n) if reverse => (n + choices.len() - 1) % choices.len(),
            Some(n) => (n + 1) % choices.len(),
            None if reverse => choices.len() - 1,
            None => 0,
        };
        Ok(choices[n].clone())
    }
    pub fn missing_refs(&self, snapshot: &Snapshot) -> Vec<String> {
        let mut refs = snapshot
            .bindings
            .iter()
            .map(|b| (&b.command, "Keyboard".to_string()))
            .collect::<Vec<_>>();
        for cycle in &snapshot.actions.cycles {
            for c in &cycle.tools {
                refs.push((c, cycle.name.clone()));
            }
        }
        for set in &snapshot.actions.access {
            for item in &set.items {
                if let crate::actions::Item::Command(c) | crate::actions::Item::Tool(c) = item {
                    refs.push((c, set.name.clone()));
                }
            }
        }
        for r in &snapshot.actions.radials {
            for c in r.slots.iter().flatten() {
                refs.push((c, r.name.clone()));
            }
        }
        for d in &snapshot.actions.devices {
            refs.push((&d.command, format!("{} / {}", d.device, d.action)));
        }
        refs.into_iter()
            .filter(|(c, _)| command_id(c).is_some() && !self.has_command(c))
            .map(|(c, label)| format!("{label}: missing user tool {c}"))
            .collect()
    }
    pub fn import_group(&mut self, source: &Self) -> Result<Vec<(u64, u64)>, String> {
        if !source.valid() {
            return Err("Invalid group version/IDs/references".into());
        }
        self.edit(|next| {
            let mut mapping = vec![];
            for c in &source.categories {
                let mut name = c.name.clone();
                let mut suffix = 2;
                while next
                    .categories
                    .iter()
                    .any(|old| old.name.to_lowercase() == name.to_lowercase())
                {
                    let tail = format!(" ({suffix})");
                    name = format!(
                        "{}{}",
                        c.name.chars().take(64 - tail.len()).collect::<String>(),
                        tail
                    );
                    suffix += 1;
                }
                let id = next.create_category(&name)?;
                mapping.push((c.id, id));
            }
            for t in &source.tools {
                let mut t = t.clone();
                let old = t.id;
                t.id = next.allocate()?;
                t.category = mapping
                    .iter()
                    .find(|(old, _)| *old == t.category)
                    .unwrap()
                    .1;
                mapping.push((old, t.id));
                next.tools.push(t);
            }
            Ok(mapping)
        })
    }
    pub(crate) fn portable(&self) -> Self {
        let mut group = self.clone();
        group.active = None;
        group.cache.fill(None);
        group.defaults.fill(None);
        group
    }
    pub fn group(&self, ids: &[u64], category: Option<u64>) -> Result<Self, String> {
        let mut group = self.clone();
        group.active = None;
        group.cache.fill(None);
        group.defaults.fill(None);
        group
            .tools
            .retain(|t| ids.contains(&t.id) || category == Some(t.category));
        group
            .categories
            .retain(|c| category == Some(c.id) || group.tools.iter().any(|t| t.category == c.id));
        if !group.valid() {
            return Err("Invalid exported group".into());
        }
        Ok(group)
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let file = fs::File::open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > FILE_LIMIT as u64 {
            return Err(io::Error::other("User tool group exceeds 4 MiB"));
        }
        let mut bytes = vec![];
        file.take(FILE_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("User tool group grew beyond limit"));
        }
        let lib: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !lib.valid() {
            return Err(io::Error::other(
                "Invalid tool group version/IDs/references",
            ));
        }
        Ok(lib)
    }
    pub fn save(&self, path: &Path, overwrite: bool) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid user tool group"));
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("User tool group exceeds 4 MiB"));
        }
        let temp = path.with_extension("tools.write.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            if overwrite {
                fs::rename(&temp, path)
            } else {
                fs::hard_link(&temp, path)?;
                fs::remove_file(&temp)
            }
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

/// Session-only list history. It never touches the drawing document's undo stack.
#[derive(Clone, Default)]
pub struct History {
    undo: Vec<Library>,
    redo: Vec<Library>,
}
impl History {
    pub fn record(&mut self, before: Library, after: &Library) {
        if before == *after {
            return;
        }
        if self.undo.len() == 16 {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn undo(&mut self, current: &mut Library) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        true
    }
    pub fn redo(&mut self, current: &mut Library) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        }
    }
    fn preset(name: &str, size: f64) -> Preset {
        let mut p = Preset::capture(&snapshot(), BrushTip::Pen, name).unwrap();
        p.values[0] = size;
        p
    }
    #[test]
    fn independent_ids_names_capabilities_defaults_and_builtin_restoration() {
        let mut s = snapshot();
        s.properties
            .as_mut()
            .unwrap()
            .tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .visible = false;
        s.properties
            .as_mut()
            .unwrap()
            .tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .control = crate::tool_properties::Control::Number;
        let original = s.clone();
        let a = s
            .user_tools
            .create_tool(1, preset("A", 21.0), Property::ALL.to_vec())
            .unwrap();
        let b = s
            .user_tools
            .create_tool(1, preset("B", 35.0), vec![Property::Opacity])
            .unwrap();
        s = s.user_tools.activate(&s, a).unwrap();
        assert_eq!(s.panels.brush_size, 21.0);
        let mut changed = Preset::capture(&s, BrushTip::Pen, "A").unwrap();
        changed.values[0] = 29.0;
        s.user_tools.update_active(changed).unwrap();
        s = s.user_tools.activate(&s, b).unwrap();
        assert_eq!(s.panels.brush_size, 12.0);
        assert!(!s.properties.as_ref().unwrap().tools[&Tool::Pen][&Property::Size].visible);
        assert_eq!(s.user_tools.entry(a).unwrap().preset.values[0], 29.0);
        s = s.user_tools.deactivate(&s).unwrap();
        assert_eq!(s.panels.brush_size, original.panels.brush_size);
        assert_eq!(s.properties, original.properties);
        assert!(s.user_tools.active.is_none());
        let copies = s.user_tools.duplicate(&[a], 1).unwrap();
        assert_ne!(copies[0], a);
        let copy = s.user_tools.entry(copies[0]).unwrap().clone();
        assert_ne!(copy.preset.name, "A");
        s.user_tools.rename_tool(a, "Renamed").unwrap();
        s.user_tools.reset_tool(a).unwrap();
        assert_eq!(s.user_tools.entry(a).unwrap().preset.values[0], 21.0);
        assert_eq!(s.user_tools.entry(copies[0]).unwrap(), &copy);
        s.user_tools.register_default(a).unwrap();
        s.user_tools.capabilities(a, vec![]).unwrap();
        assert!(s.user_tools.valid());
        let old = s.user_tools.clone();
        assert!(s.user_tools.duplicate(&vec![a; 129], 1).is_err());
        assert_eq!(s.user_tools, old);
    }
    #[test]
    fn empty_categories_collisions_bulk_move_delete_order_and_capacity_are_atomic() {
        let mut l = Library::default();
        let a = l.create_tool(1, preset("A", 12.0), vec![]).unwrap();
        let cat = l.create_category("Empty").unwrap();
        assert!(l.rename_category(cat, "my tools").is_err());
        let copy = l.duplicate(&[a], cat).unwrap()[0];
        let before = l.clone();
        assert!(l.delete_category(1, Some(cat)).is_err());
        assert_eq!(l, before);
        assert!(l.move_tools(&[a], cat).is_err());
        assert_eq!(l, before);
        l.rename_tool(copy, "Other").unwrap();
        l.delete_category(1, Some(cat)).unwrap();
        assert_eq!(l.tools.len(), 2);
        assert!(l.tools.iter().all(|t| t.category == cat));
        l.reorder_tool(a, 1).unwrap();
        l.delete_tools(&[a, copy]).unwrap();
        l.delete_category(cat, None).unwrap();
        assert!(l.tools.is_empty() && l.categories.is_empty());
        for n in 0..32 {
            l.create_category(&format!("Category {n}")).unwrap();
        }
        let old = l.clone();
        assert!(l.create_category("Overflow").is_err());
        assert_eq!(l, old);
        let category = l.categories[0].id;
        for n in 0..64 {
            l.create_tool(category, preset(&format!("Tool {n}"), 12.0), vec![])
                .unwrap();
        }
        let old = l.clone();
        assert!(
            l.create_tool(category, preset("Overflow", 12.0), vec![])
                .is_err()
        );
        assert_eq!(l, old);
    }
    #[test]
    fn groups_remap_ids_preserve_empty_categories_and_reject_corrupt_duplicate_or_foreign_temp() {
        let mut source = Library::default();
        let a = source
            .create_tool(1, preset("A", 17.0), Property::ALL.to_vec())
            .unwrap();
        let empty = source.create_category("Empty").unwrap();
        let group = source.group(&[a], Some(empty)).unwrap();
        let mut dest = source.clone();
        let map = dest.import_group(&group).unwrap();
        let mapped = map.iter().find(|(old, _)| *old == a).unwrap().1;
        assert_ne!(mapped, a);
        assert_eq!(dest.entry(mapped).unwrap().preset.values[0], 17.0);
        assert_eq!(dest.categories.len(), 4);
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("tools.json");
        group.save(&path, false).unwrap();
        assert_eq!(Library::read(&path).unwrap(), group);
        let bytes = fs::read(&path).unwrap();
        assert!(dest.save(&path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(path.with_extension("tools.write.tmp"), b"foreign").unwrap();
        assert!(dest.save(&path, true).is_err());
        assert_eq!(
            fs::read(path.with_extension("tools.write.tmp")).unwrap(),
            b"foreign"
        );
        fs::remove_file(path.with_extension("tools.write.tmp")).unwrap();
        let mut broken = group.clone();
        broken.tools[0].id = broken.categories[0].id;
        let before = dest.clone();
        assert!(dest.import_group(&broken).is_err());
        assert_eq!(dest, before);
        broken = group.clone();
        broken.version = 99;
        assert!(dest.import_group(&broken).is_err());
        fs::write(&path, b"{broken").unwrap();
        assert!(Library::read(&path).is_err());
        fs::write(&path, vec![0; FILE_LIMIT + 1]).unwrap();
        assert!(Library::read(&path).is_err());
        assert_eq!(dest, before);
    }
    #[test]
    fn active_deletion_fallback_reports_refs_and_keeps_saved_presets() {
        let mut s = snapshot();
        s.brushes.put(preset("Keep", 43.0), false).unwrap();
        let a = s
            .user_tools
            .create_tool(1, preset("A", 21.0), Property::ALL.to_vec())
            .unwrap();
        let cmd = command(a);
        s.bindings.push(crate::workspaces::Binding {
            key: "F4".into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: cmd.clone(),
        });
        s.actions.access[0]
            .items
            .push(crate::actions::Item::Command(cmd.clone()));
        s.actions.cycles[0].tools.push(cmd.clone());
        assert!(s.missing_commands().iter().all(|m| !m.contains(&cmd)));
        assert!(s.commands().contains(&cmd));
        let (merged, _) = snapshot()
            .merge(&s, crate::workspaces::Scope::ALL, &[])
            .unwrap();
        assert!(merged.bindings.iter().any(|b| b.command == cmd));
        s = s.user_tools.activate(&s, a).unwrap();
        let before = s.clone();
        let mut draft = s.user_tools.clone();
        draft.delete_tools(&[a]).unwrap();
        let next = draft.apply_to(&s).unwrap();
        assert!(next.user_tools.active.is_none());
        assert_eq!(next.tool, "Pen");
        assert_eq!(next.brushes, s.brushes);
        assert!(next.missing_commands().iter().any(|m| m.contains(&cmd)));
        let restored =
            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&before).unwrap()).unwrap();
        assert!(restored.valid());
        assert!(restored.user_tools.has_command(&cmd));
        let mut lib = restored.user_tools.clone();
        lib.register_base_default(preset("Pen default", 19.0))
            .unwrap();
        assert_eq!(
            lib.base_default(&restored, BrushTip::Pen).unwrap().values[0],
            19.0
        );
    }
}
