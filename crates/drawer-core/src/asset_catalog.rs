//! Offline folder catalog. Index metadata and document history have separate lifetimes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub const MAX_ENTRIES: usize = 4096;
pub const FILE_LIMIT: usize = 32 * 1024 * 1024;
const INDEX_LIMIT: usize = 32 * 1024 * 1024;
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub fn canceled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Local asset job canceled; catalog and artwork unchanged".into())
    } else {
        Ok(())
    }
}
fn label(s: &str, max: usize, empty: bool) -> bool {
    (empty || !s.is_empty())
        && s == s.trim()
        && s.chars().count() <= max
        && !s.chars().any(char::is_control)
}
fn relative(p: &Path) -> bool {
    !p.as_os_str().is_empty()
        && p.to_str().is_some_and(|s| s.len() <= 1024)
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}
pub(super) fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}
/// Both reads and moves reject links and paths outside the chosen local root.
pub fn checked(root: &Path, rel: &Path, exists: bool) -> Result<PathBuf, String> {
    if !relative(rel) {
        return Err("Asset path must stay relative to its local folder".into());
    }
    let canonical = crate::file_browser::folder(root)?;
    if !crate::font_library::local(&canonical)
        || linked(&fs::symlink_metadata(root).map_err(|e| e.to_string())?)
    {
        return Err("Choose a local asset folder without links".into());
    }
    let mut path = canonical.clone();
    let count = rel.components().count();
    for (n, c) in rel.components().enumerate() {
        path.push(c);
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                if linked(&m) {
                    return Err("Asset links/reparse points are not followed".into());
                }
            }
            Err(e) if !exists && n + 1 == count && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let parent = path
        .parent()
        .ok_or("Missing asset parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !parent.starts_with(&canonical) {
        return Err("Asset resolved outside its chosen folder".into());
    }
    if exists {
        let resolved = path.canonicalize().map_err(|e| e.to_string())?;
        if !resolved.starts_with(&canonical) {
            return Err("Asset resolved outside its chosen folder".into());
        }
        Ok(resolved)
    } else {
        Ok(path)
    }
}
pub(super) fn bytes(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = crate::file_browser::checked(path)?;
    let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !m.is_file() || linked(&m) || m.len() > limit as u64 {
        return Err(format!(
            "Choose a regular local asset up to{}MiB",
            limit / (1024 * 1024)
        ));
    }
    let mut b = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > limit {
        return Err("Asset grew beyond its read budget".into());
    }
    Ok(b)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Image,
    Brush,
    BrushBundle,
    Palette,
    Gradient,
    #[default]
    Unknown,
}
impl Kind {
    pub const ALL: [Self; 6] = [
        Self::Image,
        Self::Brush,
        Self::BrushBundle,
        Self::Palette,
        Self::Gradient,
        Self::Unknown,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Brush => "Own brush",
            Self::BrushBundle => "Tool folder",
            Self::Palette => "Palette",
            Self::Gradient => "Gradient",
            Self::Unknown => "Unavailable type",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    pub folder: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub favorite: bool,
    pub uses: u64,
}
impl Metadata {
    pub fn valid(&self) -> bool {
        label(&self.name, 64, false)
            && label(&self.folder, 256, true)
            && self.tags.len() <= 16
            && self
                .tags
                .iter()
                .all(|s| label(s, 64, false) && !s.contains(','))
            && self
                .tags
                .iter()
                .map(|s| s.to_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                == self.tags.len()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: PathBuf,
    pub sha256: String,
    pub kind: Kind,
    pub metadata: Metadata,
    pub dimensions: [u32; 2],
    pub thumbnail: Vec<u8>,
    pub reason: Option<String>,
    pub duplicate_of: Option<PathBuf>,
}
impl Entry {
    pub fn valid(&self) -> bool {
        relative(&self.path)
            && self.metadata.valid()
            && (valid_hash(&self.sha256) || self.sha256.is_empty() && self.reason.is_some())
            && (self.thumbnail.is_empty() || self.thumbnail.len() == 16 * 16 * 4)
            && self
                .reason
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 2048)
            && self.duplicate_of.as_ref().is_none_or(|p| relative(p))
            && self.dimensions.iter().all(|d| *d <= 32768)
    }
    pub fn available(&self) -> bool {
        self.reason.is_none() && self.kind != Kind::Unknown
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub version: u32,
    pub root: PathBuf,
    pub recursive: bool,
    pub entries: Vec<Entry>,
    pub ignored: BTreeSet<PathBuf>,
    pub notes: Vec<String>,
}
impl Index {
    pub fn valid(&self) -> bool {
        self.version == 1
            && crate::font_library::local(&self.root)
            && self.entries.len() <= MAX_ENTRIES
            && self.ignored.len() <= MAX_ENTRIES
            && self.ignored.iter().all(|p| relative(p))
            && self.entries.iter().all(Entry::valid)
            && self
                .entries
                .iter()
                .map(|e| &e.path)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
            && self.notes.len() <= 128
            && self.notes.iter().all(|n| n.len() <= 2048)
            && self.resident_bytes() <= 16 * 1024 * 1024
    }
    pub fn resident_bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|e| {
                e.thumbnail.len()
                    + e.path.as_os_str().len() * 2
                    + e.sha256.len()
                    + e.metadata.name.len()
                    + e.metadata.folder.len()
                    + e.metadata.tags.iter().map(String::len).sum::<usize>()
                    + e.reason.as_ref().map_or(0, String::len)
                    + 512
            })
            .sum::<usize>()
            + self
                .ignored
                .iter()
                .map(|p| p.as_os_str().len() * 2 + 64)
                .sum::<usize>()
            + self.notes.iter().map(String::len).sum::<usize>()
    }
    pub fn get(&self, path: &Path) -> Result<&Entry, String> {
        self.entries
            .iter()
            .find(|e| e.path == path)
            .ok_or_else(|| "Select an indexed local asset".into())
    }
    pub fn contents(&self, path: &Path, cancel: &AtomicBool) -> Result<(PathBuf, Vec<u8>), String> {
        canceled(cancel)?;
        let e = self.get(path)?;
        if !e.available() {
            return Err(e
                .reason
                .clone()
                .unwrap_or_else(|| "Unsupported asset type".into()));
        }
        let absolute = checked(&self.root, path, true)?;
        let b = bytes(&absolute, FILE_LIMIT)?;
        canceled(cancel)?;
        if hash(&b) != e.sha256 {
            return Err(
                "Asset contents changed; refresh the index and review the new version".into(),
            );
        }
        Ok((absolute, b))
    }
    pub fn edited(&self, path: &Path, metadata: Metadata) -> Result<Self, String> {
        if !metadata.valid() {
            return Err(
                "Names/folders/tags exceed local limits or contain duplicates/controls".into(),
            );
        }
        let mut next = self.clone();
        next.entries
            .iter_mut()
            .find(|e| e.path == path)
            .ok_or("Missing asset")?
            .metadata = metadata;
        if !next.valid() {
            return Err("Asset index limit".into());
        }
        Ok(next)
    }
    pub fn used(&self, path: &Path) -> Result<Self, String> {
        let mut m = self.get(path)?.metadata.clone();
        m.uses = m.uses.saturating_add(1);
        self.edited(path, m)
    }
    pub fn read(path: &Path) -> Result<(Self, String), String> {
        let b = bytes(path, INDEX_LIMIT)?;
        let absolute = path.canonicalize().map_err(|e| e.to_string())?;
        if !crate::font_library::local(&absolute) {
            return Err("Choose a local index".into());
        }
        let index: Self = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        if !index.valid() {
            return Err("Unsupported or invalid local asset index; existing file preserved".into());
        }
        Ok((index, hash(&b)))
    }
    pub fn write(
        &self,
        path: &Path,
        expected: Option<&str>,
        reset: bool,
    ) -> Result<String, String> {
        if !self.valid() {
            return Err("Invalid asset index".into());
        }
        let parent = path
            .parent()
            .ok_or("Missing index directory")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent) {
            return Err("Index storage must be local".into());
        }
        let path = parent.join(path.file_name().ok_or("Missing index filename")?);
        let b = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if b.len() > INDEX_LIMIT {
            return Err("Local index exceeds32MiB".into());
        }
        let check = || -> Result<bool, String> {
            match fs::symlink_metadata(&path) {
                Ok(m) => {
                    if !m.is_file() || linked(&m) {
                        return Err("Index destination is not a regular file".into());
                    }
                    if !reset && expected != Some(hash(&bytes(&path, INDEX_LIMIT)?).as_str()) {
                        return Err("Index changed on disk; reload before editing".into());
                    }
                    Ok(true)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if expected.is_some() && !reset {
                        return Err("Index disappeared; reload or confirm reset".into());
                    }
                    Ok(false)
                }
                Err(e) => Err(e.to_string()),
            }
        };
        check()?;
        let temp = path.with_extension("asset-index-write.tmp");
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&b).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            if check()? {
                fs::rename(&temp, &path).map_err(|e| e.to_string())?;
            } else {
                fs::hard_link(&temp, &path).map_err(|e| e.to_string())?;
            }
            Ok(hash(&b))
        })();
        let _ = fs::remove_file(temp);
        result
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    Path,
    Uses,
}
impl Index {
    pub fn search(
        &self,
        query: &str,
        kind: Option<Kind>,
        folder: &str,
        children: bool,
        favorites: bool,
        sort: Sort,
    ) -> Vec<usize> {
        let words = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut rows = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                kind.is_none_or(|k| e.kind == k)
                    && (!favorites || e.metadata.favorite)
                    && (folder.is_empty()
                        || e.metadata.folder == folder
                        || children && e.metadata.folder.starts_with(&format!("{folder}/")))
                    && {
                        let text = format!(
                            "{} {} {} {}",
                            e.metadata.name,
                            e.metadata.folder,
                            e.path.display(),
                            e.metadata.tags.join(" ")
                        )
                        .to_lowercase();
                        words.iter().all(|w| text.contains(w))
                    }
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            let (a, b) = (&self.entries[*a], &self.entries[*b]);
            b.metadata
                .pinned
                .cmp(&a.metadata.pinned)
                .then(b.metadata.favorite.cmp(&a.metadata.favorite))
                .then_with(|| match sort {
                    Sort::Uses => b.metadata.uses.cmp(&a.metadata.uses),
                    Sort::Path => a.path.cmp(&b.path),
                    Sort::Name => a
                        .metadata
                        .name
                        .to_lowercase()
                        .cmp(&b.metadata.name.to_lowercase()),
                })
                .then(a.path.cmp(&b.path))
        });
        rows
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub source_hash: String,
    pub snapshot_hash: String,
    pub name: String,
}
impl Version {
    pub fn valid(&self) -> bool {
        valid_hash(&self.source_hash)
            && valid_hash(&self.snapshot_hash)
            && label(&self.name, 64, false)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialVersion {
    pub id: u32,
    pub kind: crate::material_library::Kind,
    pub hash: String,
}
pub fn material_versions(
    style: &crate::painting::Style,
    library: &crate::material_library::Library,
) -> Vec<MaterialVersion> {
    let mut seen = BTreeSet::new();
    crate::tip_texture::references(
        &style.sequence,
        &style.variation.dual.controls,
        style.texture,
    )
    .into_iter()
    .filter(|(id, kind)| seen.insert((*id, *kind as u8)))
    .filter_map(|(id, kind)| {
        library.get(id, kind).map(|a| MaterialVersion {
            id,
            kind,
            hash: hash(&serde_json::to_vec(a).unwrap_or_default()),
        })
    })
    .collect()
}
#[path = "asset_scan.rs"]
mod scan;
pub use scan::{refresh_one, scan};
#[path = "asset_transfer.rs"]
mod transfer;
pub use transfer::{Prepared, ReplaceSize, prepare, reference_count, register};
#[path = "asset_history.rs"]
mod history;
pub use history::{History, Movement, forget, restore_record, rollback};
#[path = "asset_catalog_verify.rs"]
pub mod verify;

pub fn requires_v122(scene: &crate::scene::Scene) -> bool {
    scene
        .mouse_strokes
        .iter()
        .any(|m| !m.material_versions.is_empty())
        || scene
            .catalog
            .rasters
            .values()
            .chain(scene.catalog.vectors.values())
            .chain(scene.catalog.folders.values())
            .chain(scene.catalog.track_info.values())
            .chain(scene.catalog.cell_info.values().flat_map(|m| m.values()))
            .chain(scene.catalog.object_info.values())
            .chain(scene.catalog.world_info.values())
            .chain(scene.catalog.gradients.values())
            .chain(scene.catalog.selections.values())
            .any(|i| i.asset_version.is_some())
}

/// Cleanup is restricted to an exclusively-created direct child and rejects every link.
pub fn cleanup_verification(root: &Path, dir: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(root.as_path())
        || linked(&fs::symlink_metadata(&dir).map_err(|e| e.to_string())?)
    {
        return Err("Verification cleanup escaped its named root".into());
    }
    let mut pending = vec![dir.clone()];
    while let Some(folder) = pending.pop() {
        for child in fs::read_dir(folder).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            let m = fs::symlink_metadata(child.path()).map_err(|e| e.to_string())?;
            if linked(&m) {
                return Err("Verification cleanup refuses links/reparse points".into());
            }
            if m.is_dir() {
                pending.push(child.path());
            }
        }
    }
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}
