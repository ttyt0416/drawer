//! Versioned local palette copies. Imported IDs never overwrite unrelated entries.
use crate::color_tables::{Settings, Table};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};
const LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    pub tables: Vec<Table>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Copy,
    ReplaceNames,
}
impl Bundle {
    pub fn valid(&self) -> bool {
        let max = self
            .tables
            .iter()
            .flat_map(|t| std::iter::once(t.id).chain(t.colors.iter().map(|c| c.id)))
            .max()
            .unwrap_or(0);
        self.version == 1
            && !self.tables.is_empty()
            && max < u64::MAX
            && Settings {
                next_id: max + 1,
                tables: self.tables.clone(),
                selected: None,
                ..Default::default()
            }
            .valid()
    }
    pub fn capture(current: &Settings, selected: bool) -> Result<Self, String> {
        if !current.valid() {
            return Err("Invalid palette settings".into());
        }
        let tables = current
            .tables
            .iter()
            .filter(|t| !selected || Some(t.id) == current.selected)
            .cloned()
            .collect();
        let b = Self { version: 1, tables };
        if !b.valid() {
            return Err("Choose at least one valid palette to export".into());
        }
        Ok(b)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > LIMIT {
            return Err("Palette exchange exceeds2MiB".into());
        }
        let b: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !b.valid() {
            return Err("Invalid palette version, IDs, names, colors or capacity".into());
        }
        Ok(b)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !m.is_file() || m.file_type().is_symlink() || m.len() > LIMIT as u64 {
            return Err("Choose a regular palette file up to2MiB".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if !crate::font_library::local(&path) {
            return Err("Choose a local palette file".into());
        }
        let mut bytes = vec![];
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::decode(&bytes)
    }
    pub fn write(&self, path: &Path, replace: bool) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid palette bundle".into());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Palette exchange exceeds2MiB".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent) {
            return Err("Choose a local palette folder".into());
        }
        let path = parent.join(path.file_name().ok_or("Missing palette filename")?);
        let check = || -> Result<(), String> {
            if let Ok(m) = fs::symlink_metadata(&path) {
                if m.file_type().is_symlink() || !m.is_file() {
                    return Err("Choose a regular palette destination".into());
                }
                if !replace {
                    return Err("Confirm replacement of the existing palette export".into());
                }
            }
            Ok(())
        };
        check()?;
        let mut name = path.file_name().unwrap().to_owned();
        name.push(".palette-write.tmp");
        let temp = path.with_file_name(name);
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            check()?;
            if replace {
                fs::rename(&temp, &path).map_err(|e| e.to_string())
            } else {
                fs::hard_link(&temp, &path).map_err(|e| e.to_string())
            }
        })();
        let _ = fs::remove_file(&temp);
        result
    }
    pub fn prepare(&self, current: &Settings, policy: Policy) -> Result<Prepared, String> {
        if !self.valid() || !current.valid() {
            return Err("Invalid palette source / destination".into());
        }
        if policy == Policy::ReplaceNames
            && self
                .tables
                .iter()
                .map(|t| t.name.to_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                != self.tables.len()
        {
            return Err("Replacement requires unique incoming palette names".into());
        }
        let mut next = current.clone();
        let mut replaced = vec![];
        let mut mapping = vec![];
        let allocate = |s: &mut Settings| -> Result<u64, String> {
            let id = s.next_id;
            s.next_id = id.checked_add(1).ok_or("Palette ID limit")?;
            Ok(id)
        };
        for source in &self.tables {
            let mut table = source.clone();
            let matching = next
                .tables
                .iter()
                .enumerate()
                .filter(|(_, t)| t.name.to_lowercase() == source.name.to_lowercase())
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            let target = if policy == Policy::ReplaceNames {
                if matching.len() > 1 {
                    return Err("Ambiguous existing palette name; use independent copies".into());
                }
                matching.first().copied()
            } else {
                None
            };
            table.id = if let Some(i) = target {
                replaced.push(next.tables[i].name.clone());
                next.tables[i].id
            } else {
                allocate(&mut next)?
            };
            mapping.push((source.id, table.id));
            if target.is_none() {
                let mut suffix = 2;
                while next
                    .tables
                    .iter()
                    .any(|t| t.name.to_lowercase() == table.name.to_lowercase())
                {
                    let tail = format!(" ({suffix})");
                    table.name = format!(
                        "{}{}",
                        source
                            .name
                            .chars()
                            .take(64 - tail.chars().count())
                            .collect::<String>(),
                        tail
                    );
                    suffix += 1;
                }
            }
            for c in &mut table.colors {
                let old = c.id;
                c.id = allocate(&mut next)?;
                mapping.push((old, c.id));
            }
            let id = table.id;
            if let Some(i) = target {
                next.tables[i] = table;
            } else {
                next.tables.push(table);
            }
            next.selected = Some(id);
        }
        if !next.valid() {
            return Err("Palette table/color capacity exceeded; nothing imported".into());
        }
        Ok(Prepared {
            before: current.clone(),
            after: next,
            replaced,
            mapping,
        })
    }
}
pub struct Prepared {
    before: Settings,
    pub after: Settings,
    pub replaced: Vec<String>,
    pub mapping: Vec<(u64, u64)>,
}
impl Prepared {
    pub fn check(&self, current: &Settings, confirmed: bool) -> Result<(), String> {
        if !self.after.valid() {
            return Err("Invalid staged palette settings".into());
        }
        if current != &self.before {
            return Err("Palette settings changed; prepare import again".into());
        }
        if !self.replaced.is_empty() && !confirmed {
            return Err("Confirm replacement of listed existing palettes".into());
        }
        Ok(())
    }
}
