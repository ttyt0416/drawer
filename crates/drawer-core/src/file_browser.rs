//! Local-only folder selection, metadata views and reviewed physical operations.
use crate::{asset_catalog as assets, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
    time::SystemTime,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    #[default]
    Details,
    Thumbnails,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Name,
    Extension,
    Modified,
    ExtensionModified,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    All,
    #[default]
    Supported,
    Native,
    Images,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub folder: Option<PathBuf>,
    pub favorites: Vec<PathBuf>,
    pub view: View,
    pub sort: Sort,
    pub descending: bool,
    pub filter: Filter,
    pub name: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            folder: None,
            favorites: vec![],
            view: View::Details,
            sort: Sort::Name,
            descending: false,
            filter: Filter::Supported,
            name: String::new(),
        }
    }
}
pub fn lexical(p: &Path) -> Result<(), String> {
    crate::file_objects::local_path(p)?;
    if p.components().any(|c| matches!(c, Component::ParentDir))
        || p.to_str()
            .is_some_and(|s| s.split(['\\', '/']).any(|part| part == ".."))
    {
        return Err("Use an absolute local path without parent traversal".into());
    }
    Ok(())
}
/// Inspect each component before resolving it; a local junction must not trigger remote access.
pub fn checked(p: &Path) -> Result<PathBuf, String> {
    local_drive(p)?;
    let mut current = PathBuf::new();
    for c in p.components() {
        current.push(c);
        if matches!(c, Component::Prefix(_)) || !current.is_absolute() {
            continue;
        }
        let m = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if assets::linked(&m) {
            return Err("Links/reparse points are not followed by the local browser".into());
        }
    }
    let path = p.canonicalize().map_err(|e| e.to_string())?;
    lexical(&path)?;
    Ok(path)
}
/// Classify the drive without opening a path on it. Cached reference metadata uses lexical only.
pub fn local_drive(p: &Path) -> Result<(), String> {
    lexical(p)?;
    #[cfg(windows)]
    {
        let text = p
            .to_str()
            .ok_or("Non-Unicode path")?
            .trim_start_matches(r"\\?\");
        let root = format!("{}:\\", text.as_bytes()[0] as char);
        drive_kind(winsafe::GetDriveType(Some(&root)))?;
    }
    Ok(())
}
#[cfg(windows)]
fn drive_kind(kind: winsafe::co::DRIVE) -> Result<(), String> {
    use winsafe::co::DRIVE;
    match kind {
        DRIVE::REMOVABLE | DRIVE::FIXED | DRIVE::CDROM | DRIVE::RAMDISK => Ok(()),
        DRIVE::REMOTE => Err("Mapped network drives are not accessed".into()),
        _ => Err("Local drive unavailable or disconnected".into()),
    }
}
/// Admit an existing path or a new suffix before any create/resolve operation.
pub fn new_path(p: &Path) -> Result<PathBuf, String> {
    local_drive(p)?;
    let mut current = PathBuf::new();
    let mut missing = false;
    for c in p.components() {
        if let Component::Normal(name) = c {
            component(name.to_str().ok_or("Non-Unicode path component")?)?;
        }
        current.push(c);
        if missing || matches!(c, Component::Prefix(_)) || !current.is_absolute() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(m) if assets::linked(&m) => {
                return Err("Links/reparse points are not followed by local storage".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(e) => return Err(e.to_string()),
        }
    }
    if missing {
        Ok(p.to_owned())
    } else {
        checked(p)
    }
}
pub fn folder(p: &Path) -> Result<PathBuf, String> {
    let path = checked(p)?;
    if !path.is_dir() {
        return Err("Choose a local folder".into());
    }
    Ok(path)
}
pub fn component(name: &str) -> Result<(), String> {
    let device = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .replace('¹', "1")
        .replace('²', "2")
        .replace('³', "3")
        .to_ascii_uppercase();
    if name.is_empty()
        || name != name.trim()
        || name.ends_with('.')
        || name.encode_utf16().count() > 240
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || [
            ".", "..", "CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$",
        ]
        .contains(&device.as_str())
        || (1..=9).any(|n| device == format!("COM{n}") || device == format!("LPT{n}"))
    {
        return Err("Choose a name of1..240 UTF-16 units without reserved characters, devices or trailing spaces/dots".into());
    }
    Ok(())
}
impl Settings {
    pub fn valid(&self) -> bool {
        let mut keys = BTreeSet::new();
        self.version == 1
            && self.name.len() <= 256
            && self.favorites.len() <= 64
            && self.folder.as_ref().is_none_or(|p| lexical(p).is_ok())
            && self
                .favorites
                .iter()
                .all(|p| lexical(p).is_ok() && keys.insert(key(p)))
    }
    pub fn load(s: &UserStorage) -> Result<Self, String> {
        let p = s.root().join("settings/file-browser.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let bytes = assets::bytes(&p, 65536)?;
        let s: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err("Invalid local browser settings; original preserved".into());
        }
        Ok(s)
    }
    pub fn save(&self, s: &UserStorage) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid browser settings".into());
        }
        let p = s.root().join("settings/file-browser.json");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Browser settings size limit".into());
        }
        let temp = p.with_extension("json.write.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            fs::rename(&temp, &p).map_err(|e| e.to_string())
        })();
        if temp.exists() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn favorite(&mut self, p: &Path) -> Result<(), String> {
        let p = folder(p)?;
        if let Some(n) = self.favorites.iter().position(|q| same(q, &p)) {
            self.favorites.remove(n);
        } else {
            if self.favorites.len() == 64 {
                return Err("Favorite folder limit64".into());
            }
            self.favorites.push(p);
        }
        Ok(())
    }
}
pub fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}
fn key(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}
pub fn remap(p: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let a = key(p);
    let b = key(old);
    if a == b {
        return Some(new.into());
    }
    let prefix = b.trim_end_matches('\\').to_owned() + "\\";
    if a.starts_with(&prefix) {
        let original = p
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\");
        let count = old
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .chars()
            .count();
        let rest = original.chars().skip(count + 1).collect::<String>();
        Some(new.join(rest))
    } else {
        None
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub readonly: bool,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub unavailable: Option<String>,
}
impl Entry {
    pub fn extension(&self) -> String {
        self.path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_lowercase()
    }
}
pub struct Listing {
    pub folder: PathBuf,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
pub fn image(p: &Path) -> bool {
    p.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        ["png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp"]
            .iter()
            .any(|q| q.eq_ignore_ascii_case(s))
    })
}
pub fn supported(p: &Path) -> bool {
    image(p)
        || p.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            ["drawer", "psd", "psb"]
                .iter()
                .any(|q| q.eq_ignore_ascii_case(s))
        })
}
pub fn scan(path: &Path, s: &Settings, c: &AtomicBool) -> Result<Listing, String> {
    let root = folder(path)?;
    let mut entries = vec![];
    let mut truncated = false;
    let mut metadata_bytes = 0usize;
    for item in fs::read_dir(&root).map_err(|e| e.to_string())? {
        assets::canceled(c)?;
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        let m = fs::symlink_metadata(&path);
        let directory = m.as_ref().is_ok_and(|m| m.is_dir());
        if !name.to_lowercase().contains(&s.name.to_lowercase())
            || !directory
                && !match s.filter {
                    Filter::All => true,
                    Filter::Supported => supported(&path),
                    Filter::Images => image(&path),
                    Filter::Native => path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("drawer")),
                }
        {
            continue;
        }
        metadata_bytes =
            metadata_bytes.saturating_add(path.to_string_lossy().len() + name.len() + 256);
        if entries.len() == 8192 || metadata_bytes > 16 * 1024 * 1024 {
            truncated = true;
            break;
        }
        let (readonly, bytes, modified, unavailable) = match m {
            Ok(m) => (
                m.permissions().readonly(),
                if m.is_file() { m.len() } else { 0 },
                m.modified().ok(),
                if assets::linked(&m) {
                    Some("Link/reparse point: not followed".into())
                } else if path.to_str().is_none() {
                    Some("Non-Unicode path is unavailable".into())
                } else {
                    None
                },
            ),
            Err(e) => (true, 0, None, Some(e.to_string())),
        };
        entries.push(Entry {
            path,
            name,
            directory,
            readonly,
            bytes,
            modified,
            unavailable,
        });
    }
    sort(&mut entries, s.sort, s.descending);
    assets::canceled(c)?;
    Ok(Listing {
        folder: root,
        entries,
        truncated,
    })
}
pub fn sort(entries: &mut [Entry], sort: Sort, descending: bool) {
    entries.sort_by(|a, b| {
        let directories = b.directory.cmp(&a.directory);
        if directories != Ordering::Equal {
            return directories;
        }
        let order = match sort {
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Extension => a.extension().cmp(&b.extension()),
            Sort::Modified => a.modified.cmp(&b.modified),
            Sort::ExtensionModified => a
                .extension()
                .cmp(&b.extension())
                .then(a.modified.cmp(&b.modified)),
        }
        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then(a.name.cmp(&b.name));
        if descending { order.reverse() } else { order }
    });
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub path: PathBuf,
    pub directory: bool,
    pub bytes: u64,
    pub modified: SystemTime,
}
impl Stamp {
    pub fn read(p: &Path) -> Result<Self, String> {
        let path = checked(p)?;
        if path.parent().is_none() {
            return Err("A drive root cannot be managed here".into());
        }
        let m = fs::metadata(&path).map_err(|e| e.to_string())?;
        if !m.is_file() && !m.is_dir() {
            return Err("Not a regular local file/folder".into());
        }
        Ok(Self {
            path,
            directory: m.is_dir(),
            bytes: m.len(),
            modified: m.modified().map_err(|e| e.to_string())?,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if Self::read(&self.path)? != *self {
            return Err("Selected source changed; review it again".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Ticket {
    pub source: Stamp,
    pub destination: Option<PathBuf>,
}
impl Ticket {
    pub fn new(p: &Path, name: Option<&str>) -> Result<Self, String> {
        let source = Stamp::read(p)?;
        let parent = source
            .path
            .parent()
            .ok_or("Drive roots cannot be managed")?;
        if fs::metadata(&source.path)
            .map_err(|e| e.to_string())?
            .permissions()
            .readonly()
            && !source.directory
        {
            return Err("Read-only file: clear its attribute before physical management".into());
        }
        let destination = if let Some(name) = name {
            component(name)?;
            let path = parent.join(name);
            if path.exists() && !case_target(&source.path, &path) {
                return Err("Destination exists; no replacement is allowed".into());
            }
            Some(path)
        } else {
            None
        };
        Ok(Self {
            source,
            destination,
        })
    }
    pub fn run(
        &self,
        confirmed: bool,
        c: &AtomicBool,
        mut rename: impl FnMut(&Path, &Path) -> Result<(), String>,
        mut recycle: impl FnMut(&Path, &AtomicBool) -> Result<(), String>,
    ) -> Result<Event, String> {
        if !confirmed {
            return Err("Confirm the exact physical target first".into());
        }
        assets::canceled(c)?;
        self.source.validate()?;
        if let Some(to) = &self.destination {
            if assets::checked(
                to.parent().ok_or("Missing folder")?,
                Path::new(to.file_name().ok_or("Missing name")?),
                false,
            )?
            .exists()
                && !case_target(&self.source.path, to)
            {
                return Err("Destination appeared; source preserved".into());
            }
            rename(&self.source.path, to)?;
            Ok(Event::Renamed {
                old: self.source.path.clone(),
                new: checked(to)?,
            })
        } else {
            reject_tree_links(&self.source.path, c)?;
            recycle(&self.source.path, c)?;
            if self.source.path.exists() {
                return Err(
                    "Recycle did not remove the selected source; no permanent fallback".into(),
                );
            }
            Ok(Event::Recycled(self.source.path.clone()))
        }
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Renamed { old: PathBuf, new: PathBuf },
    Recycled(PathBuf),
}
pub fn new_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    component(name)?;
    let parent = folder(parent)?;
    let path = assets::checked(&parent, Path::new(name), false)?;
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    checked(&path)
}
pub fn reject_tree_links(p: &Path, c: &AtomicBool) -> Result<(), String> {
    let mut pending = vec![p.to_owned()];
    let mut count = 0;
    while let Some(path) = pending.pop() {
        assets::canceled(c)?;
        let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if assets::linked(&m) {
            return Err(
                "Physical operations refuse a folder tree containing links/reparse points".into(),
            );
        }
        count += 1;
        if count > 8192 {
            return Err("Folder operation exceeds8192 entries; manage a smaller folder".into());
        }
        if m.is_dir() {
            for child in fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(child.map_err(|e| e.to_string())?.path());
            }
        }
    }
    Ok(())
}
pub struct Thumbnail {
    pub path: PathBuf,
    pub stamp: Stamp,
    pub size: [usize; 2],
    pub pixels: Vec<[u8; 4]>,
    pub notice: Option<String>,
}
pub fn thumbnail(p: &Path, budget: usize, c: &AtomicBool) -> Result<Thumbnail, String> {
    let stamp = Stamp::read(p)?;
    if stamp.directory || !supported(p) || stamp.bytes > 64 * 1024 * 1024 {
        return Err("No thumbnail for this type/size; source unchanged".into());
    }
    assets::canceled(c)?;
    crate::exchange_guidance::probe(p)?;
    let (d, notices) = crate::files::open_report(p, (budget / 4).min(16 * 1024 * 1024))
        .map_err(|e| e.to_string())?;
    let render = crate::render::Renderer::for_export(&d)?;
    let step = (d.width as f64 / 96.).max(d.height as f64 / 96.).max(1.);
    let size = [
        (d.width as f64 / step).ceil() as usize,
        (d.height as f64 / step).ceil() as usize,
    ];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        assets::canceled(c)?;
        for x in 0..size[0] {
            pixels.push(
                render
                    .display_sample([
                        (x as f64 + 0.5) / size[0] as f64 * d.width as f64,
                        (y as f64 + 0.5) / size[1] as f64 * d.height as f64,
                    ])?
                    .rgba8(),
            )
        }
    }
    stamp.validate()?;
    Ok(Thumbnail {
        path: p.into(),
        stamp,
        size,
        pixels,
        notice: (!notices.is_empty()).then(|| notices.join("; ")),
    })
}
#[path = "file_browser_verify.rs"]
pub mod verify;

/// UTC calendar display, avoiding locale-dependent ordering or raw epoch labels.
pub fn modified_label(time: Option<SystemTime>) -> String {
    let Some(seconds) = time
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
    else {
        return "Unknown date".into();
    };
    let Ok(days) = i64::try_from(seconds / 86400) else {
        return "Date outside range".into();
    };
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    if y > 9999 {
        return "Date outside range".into();
    }
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

// Only the same existing sibling may receive a case-only rename. Distinct hard-link
// names, different folders and unrelated occupied destinations remain collisions.
fn case_target(source: &Path, target: &Path) -> bool {
    source != target
        && source.parent() == target.parent()
        && source
            .file_name()
            .zip(target.file_name())
            .is_some_and(|(a, b)| {
                a.to_string_lossy()
                    .eq_ignore_ascii_case(&b.to_string_lossy())
            })
        && target.canonicalize().ok().as_deref() == Some(source)
}
pub(crate) fn remap_scene(scene: &mut crate::scene::Scene, old: &Path, new: &Path) -> usize {
    let mut count = 0;
    for link in &mut scene.file_objects.entries {
        if let Some(path) = remap(&link.path, old, new) {
            link.path = path;
            count += 1;
        }
    }
    count
}
/// Informational references without a managed path adapter are listed, never
/// silently rewritten as arbitrary text, and never opened during reconciliation.
pub fn unmapped(scene: &crate::scene::Scene, old: &Path) -> Vec<PathBuf> {
    fn visit(v: &serde_json::Value, old: &Path, paths: &mut BTreeSet<PathBuf>) {
        if paths.len() >= 128 {
            return;
        }
        match v {
            serde_json::Value::String(s) => {
                let p = Path::new(s);
                if p.is_absolute() && remap(p, old, old).is_some() {
                    paths.insert(p.to_owned());
                }
            }
            serde_json::Value::Array(a) => {
                for v in a {
                    visit(v, old, paths);
                }
            }
            serde_json::Value::Object(o) => {
                for v in o.values() {
                    visit(v, old, paths);
                }
            }
            _ => {}
        }
    }
    let mut paths = BTreeSet::new();
    if let Ok(v) = serde_json::to_value(scene) {
        visit(&v, old, &mut paths);
    }
    paths.into_iter().collect()
}

#[path = "file_browser_completion.rs"]
mod completion;

#[cfg(all(test, windows))]
mod drive_policy_tests {
    #[test]
    fn mapped_network_and_disconnected_drives_are_refused_before_path_io() {
        use super::drive_kind;
        use winsafe::co::DRIVE;
        assert!(drive_kind(DRIVE::REMOTE).is_err());
        assert!(drive_kind(DRIVE::UNKNOWN).is_err());
        assert!(drive_kind(DRIVE::NO_ROOT_DIR).is_err());
        for kind in [DRIVE::REMOVABLE, DRIVE::FIXED, DRIVE::CDROM, DRIVE::RAMDISK] {
            assert!(drive_kind(kind).is_ok());
        }
    }
}
