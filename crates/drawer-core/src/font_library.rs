//! Local read-only font discovery. Documents keep content identity and a bounded display name.
use crate::text_layout::Font;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub id: [u8; 32],
    pub index: u32,
    name: [u8; 128],
    length: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceData {
    id: [u8; 32],
    index: u32,
    name: String,
}
impl Reference {
    pub fn new(id: [u8; 32], index: u32, name: &str) -> Self {
        let mut clean: String = name.chars().filter(|c| !c.is_control()).take(128).collect();
        while clean.len() > 128 {
            clean.pop();
        }
        if clean.is_empty() {
            clean = "Local font".into();
        }
        let mut bytes = [0; 128];
        bytes[..clean.len()].copy_from_slice(clean.as_bytes());
        Self {
            id,
            index,
            name: bytes,
            length: clean.len() as u8,
        }
    }
    pub fn valid(self) -> bool {
        self.index < 64 && self.length > 0 && self.length <= 128
    }
    pub fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..usize::from(self.length)]).unwrap_or("Local font")
    }
}
impl Serialize for Reference {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut s = s.serialize_struct("FontReference", 3)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("index", &self.index)?;
        s.serialize_field("name", self.name())?;
        s.end()
    }
}
impl<'de> Deserialize<'de> for Reference {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = ReferenceData::deserialize(d)?;
        if v.index >= 64
            || v.name.is_empty()
            || v.name.len() > 128
            || v.name.chars().any(char::is_control)
        {
            return Err(serde::de::Error::custom("Invalid local font reference"));
        }
        Ok(Self::new(v.id, v.index, &v.name))
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub font: Font,
    pub name: String,
    pub path: Option<PathBuf>,
}
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub failures: usize,
}
struct Record {
    reference: Reference,
    paths: Vec<PathBuf>,
}
struct Cached {
    path: PathBuf,
    len: u64,
    stamp: SystemTime,
    data: Arc<Vec<u8>>,
    prefix: Sha256,
}
static RECORDS: Mutex<Vec<Record>> = Mutex::new(Vec::new());
static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
fn local(path: &Path) -> bool {
    path.is_absolute() && !path.to_string_lossy().starts_with("\\\\")
}
fn prefix(bytes: &[u8]) -> Sha256 {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash
}
fn identity(prefix: &Sha256, index: u32) -> [u8; 32] {
    let mut hash = prefix.clone();
    hash.update(index.to_le_bytes());
    hash.finalize().into()
}
fn read_font(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut data = Vec::new();
    f.take(32 * 1024 * 1024 + 1).read_to_end(&mut data)?;
    if data.len() > 32 * 1024 * 1024 {
        return Err(std::io::Error::other("Font size limit32MiB"));
    }
    Ok(data)
}
fn register(reference: Reference, path: &Path) {
    if let Ok(mut records) = RECORDS.lock() {
        if let Some(r) = records.iter_mut().find(|r| r.reference.id == reference.id) {
            if !r.paths.iter().any(|p| p == path) && r.paths.len() < 16 {
                r.paths.push(path.into());
            }
        } else if records.len() < 4096 {
            records.push(Record {
                reference,
                paths: vec![path.into()],
            });
        }
    }
}
pub fn default_roots(extra: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Some(root) = std::env::var_os("WINDIR") {
        roots.push(PathBuf::from(root).join("Fonts"));
    }
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(&root).join("Microsoft/Windows/Fonts"));
        if extra.is_none() {
            roots.push(PathBuf::from(root).join("Drawer/fonts"));
        }
    }
    if let Some(root) = extra {
        roots.push(root.join("fonts"));
    } else if let Some(root) = std::env::var_os("DRAWER_DATA_DIR") {
        roots.push(PathBuf::from(root).join("fonts"));
    }
    roots.retain(|p| local(p));
    roots.sort();
    roots.dedup();
    roots
}
impl Catalog {
    pub fn discover(roots: &[PathBuf], cancel: impl Fn() -> bool) -> Self {
        let mut result = Self {
            entries: Font::ALL
                .into_iter()
                .map(|font| Entry {
                    font,
                    name: font.label(),
                    path: None,
                })
                .collect(),
            failures: 0,
        };
        let mut files = vec![];
        for root in roots {
            if !local(root) {
                result.failures += 1;
                continue;
            }
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                if files.len() >= 2048 {
                    break;
                }
                if entry.file_type().is_ok_and(|f| f.is_file())
                    && entry.path().extension().is_some_and(|e| {
                        ["ttf", "otf", "ttc", "otc"]
                            .iter()
                            .any(|v| e.eq_ignore_ascii_case(v))
                    })
                {
                    files.push(entry.path());
                }
            }
        }
        files.sort();
        files.dedup();
        for path in files {
            if cancel() || result.entries.len() >= 1024 {
                break;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                result.failures += 1;
                continue;
            };
            if meta.len() > 32 * 1024 * 1024 {
                result.failures += 1;
                continue;
            }
            let Ok(bytes) = read_font(&path) else {
                result.failures += 1;
                continue;
            };
            let prefix = prefix(&bytes);
            let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1).min(64);
            let mut valid = false;
            for index in 0..count {
                let Ok(face) = ttf_parser::Face::parse(&bytes, index) else {
                    continue;
                };
                let name = face
                    .names()
                    .into_iter()
                    .filter(|n| {
                        matches!(
                            n.name_id,
                            ttf_parser::name_id::FULL_NAME
                                | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                        )
                    })
                    .find_map(|n| n.to_string())
                    .unwrap_or_else(|| {
                        path.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    });
                let reference = Reference::new(identity(&prefix, index), index, &name);
                register(reference, &path);
                let font = Font::Local(reference);
                if !result.entries.iter().any(|e| e.font == font) {
                    result.entries.push(Entry {
                        font,
                        name: reference.name().into(),
                        path: Some(path.clone()),
                    });
                }
                valid = true;
                if result.entries.len() >= 1024 {
                    break;
                }
            }
            if !valid {
                result.failures += 1;
            }
        }
        result.entries.sort_by_key(|e| e.name.to_lowercase());
        result
    }
    pub fn filtered(&self, query: &str) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&q))
            .collect()
    }
}
fn cached(path: &Path, reference: Reference) -> Option<Arc<Vec<u8>>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 32 * 1024 * 1024 {
        return None;
    }
    let stamp = meta.modified().ok()?;
    if let Ok(cache) = CACHE.lock()
        && let Some(c) = cache
            .iter()
            .find(|c| c.path == path && c.len == meta.len() && c.stamp == stamp)
    {
        return (identity(&c.prefix, reference.index) == reference.id).then(|| c.data.clone());
    }
    let data = read_font(path).ok()?;
    let prefix = prefix(&data);
    if identity(&prefix, reference.index) != reference.id
        || ttf_parser::Face::parse(&data, reference.index).is_err()
    {
        return None;
    }
    let data = Arc::new(data);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|c| c.path != path);
        while cache.len() >= 32
            || cache.iter().map(|c| c.data.len()).sum::<usize>() + data.len() > 64 * 1024 * 1024
        {
            cache.remove(0);
        }
        cache.push(Cached {
            path: path.into(),
            len: meta.len(),
            stamp,
            data: data.clone(),
            prefix,
        });
    }
    Some(data)
}
pub fn invalidate_data() {
    if let Ok(mut cache) = CACHE.lock() {
        cache.clear();
    }
}
pub fn data(reference: Reference) -> Option<Arc<Vec<u8>>> {
    let paths = RECORDS
        .lock()
        .ok()?
        .iter()
        .find(|r| r.reference.id == reference.id && r.reference.index == reference.index)
        .map(|r| r.paths.clone());
    if let Some(paths) = paths {
        for path in paths {
            if let Some(data) = cached(&path, reference) {
                return Some(data);
            }
        }
        return None;
    }
    static SCANNED: std::sync::Once = std::sync::Once::new();
    SCANNED.call_once(|| {
        Catalog::discover(&default_roots(None), || false);
    });
    let paths = RECORDS
        .lock()
        .ok()?
        .iter()
        .find(|r| r.reference.id == reference.id && r.reference.index == reference.index)?
        .paths
        .clone();
    paths.iter().find_map(|p| cached(p, reference))
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    #[default]
    Ui,
    OwnFont,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub id: u64,
    pub name: String,
    pub members: Vec<Font>,
    pub display: Display,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub next_id: u64,
    pub active: Option<u64>,
    pub lists: Vec<List>,
    pub favorites: Vec<Font>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            next_id: 1,
            active: None,
            lists: vec![],
            favorites: vec![],
        }
    }
}
fn name_valid(n: &str) -> bool {
    !n.trim().is_empty() && n.chars().count() <= 64 && !n.chars().any(char::is_control)
}
impl Settings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn valid(&self) -> bool {
        self.next_id > 0
            && self.lists.len() <= 32
            && self.favorites.len() <= 1024
            && self.favorites.iter().all(|f| f.valid())
            && self
                .active
                .is_none_or(|id| self.lists.iter().any(|l| l.id == id))
            && self.lists.iter().enumerate().all(|(n, l)| {
                l.id > 0
                    && l.id < self.next_id
                    && name_valid(&l.name)
                    && l.members.len() <= 1024
                    && l.members.iter().all(|f| f.valid())
                    && !self.lists[..n].iter().any(|other| {
                        other.id == l.id
                            || other.name.trim().to_lowercase() == l.name.trim().to_lowercase()
                    })
                    && l.members
                        .iter()
                        .enumerate()
                        .all(|(n, f)| !l.members[..n].contains(f))
            })
            && self
                .favorites
                .iter()
                .enumerate()
                .all(|(n, f)| !self.favorites[..n].contains(f))
    }
    fn unique(&self, name: &str, except: Option<u64>) -> Result<(), String> {
        if !name_valid(name)
            || self.lists.iter().any(|l| {
                Some(l.id) != except && l.name.trim().to_lowercase() == name.trim().to_lowercase()
            })
        {
            return Err("Use a unique nonempty font-list name up to64 characters; existing lists are not overwritten".into());
        }
        Ok(())
    }
    pub fn create(&mut self, name: String) -> Result<u64, String> {
        self.unique(&name, None)?;
        if self.lists.len() >= 32 || self.next_id == u64::MAX {
            return Err("Font-list limit32".into());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.lists.push(List {
            id,
            name,
            members: vec![],
            display: Display::Ui,
        });
        self.active = Some(id);
        Ok(id)
    }
    pub fn rename(&mut self, id: u64, name: String) -> Result<(), String> {
        self.unique(&name, Some(id))?;
        self.lists
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or("Choose a font list")?
            .name = name;
        Ok(())
    }
    pub fn duplicate(&mut self, id: u64, name: String) -> Result<u64, String> {
        let old = self
            .lists
            .iter()
            .find(|l| l.id == id)
            .cloned()
            .ok_or("Choose a font list")?;
        let id = self.create(name)?;
        let l = self.lists.last_mut().unwrap();
        l.members = old.members;
        l.display = old.display;
        Ok(id)
    }
    pub fn delete(&mut self, id: u64, confirmation: &str) -> Result<(), String> {
        let l = self
            .lists
            .iter()
            .find(|l| l.id == id)
            .ok_or("Choose a font list")?;
        if l.name != confirmation {
            return Err("Type the exact font-list name to delete it".into());
        }
        self.lists.retain(|l| l.id != id);
        if self.active == Some(id) {
            self.active = None;
        }
        Ok(())
    }
    pub fn include(&mut self, id: u64, font: Font, included: bool) -> Result<(), String> {
        let l = self
            .lists
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or("Choose a font list")?;
        if included && !l.members.contains(&font) {
            if l.members.len() >= 1024 {
                return Err("Font-list member limit1024".into());
            }
            l.members.push(font);
        } else if !included {
            l.members.retain(|f| *f != font);
        }
        Ok(())
    }
    pub fn bulk(
        &mut self,
        id: u64,
        catalog: &Catalog,
        query: &str,
        included: bool,
    ) -> Result<(), String> {
        for entry in catalog.filtered(query) {
            self.include(id, entry.font, included)?;
        }
        Ok(())
    }
    pub fn choices(&self, catalog: &Catalog) -> Vec<Font> {
        if let Some(l) = self
            .active
            .and_then(|id| self.lists.iter().find(|l| l.id == id))
        {
            l.members.clone()
        } else {
            catalog.entries.iter().map(|e| e.font).collect()
        }
    }
    pub fn favorite(&mut self, font: Font, value: bool) {
        if value && !self.favorites.contains(&font) && self.favorites.len() < 1024 {
            self.favorites.push(font);
        } else if !value {
            self.favorites.retain(|f| *f != font);
        }
    }
}
pub fn character(font: Font, code: u32) -> Result<(char, bool), String> {
    let c = char::from_u32(code)
        .filter(|c| !c.is_control())
        .ok_or("Enter a printable Unicode scalar value; surrogate code points are invalid")?;
    let supported = match font {
        Font::Local(r) => data(r).is_some_and(|b| {
            ttf_parser::Face::parse(&b, r.index)
                .is_ok_and(|f| crate::text_layout::glyph_supported(&f, c))
        }),
        _ => crate::text_layout::font_supports(font, c),
    };
    Ok((c, supported))
}
