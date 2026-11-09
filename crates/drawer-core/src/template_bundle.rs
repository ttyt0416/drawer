//! Versioned, manifest-last local templates; native snapshots carry render resources.
use crate::{asset_catalog as assets, document::Document, material_library::Library};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
const NATIVE_LIMIT: usize = 64 * 1024 * 1024;
const MASK_LIMIT: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Layers,
    Cut,
    Animation,
}
impl Kind {
    pub const ALL: [Self; 3] = [Self::Layers, Self::Cut, Self::Animation];
    pub fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layer stack",
            Self::Cut => "Comic cut / page",
            Self::Animation => "Animation / timelines",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    bytes: usize,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    native_version: u32,
    name: String,
    kind: Kind,
    frame: u32,
    files: Vec<File>,
}
fn label(s: &str) -> bool {
    !s.is_empty() && s == s.trim() && s.chars().count() <= 64 && !s.chars().any(char::is_control)
}
fn read(root: &Path, name: &str, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let path = assets::checked(root, Path::new(name), true)?;
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() > limit as u64 {
        return Err("Template file exceeds its allocation limit".into());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        assets::canceled(cancel)?;
        let n = f.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > limit {
            return Err("Template file grew beyond its limit".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(bytes)
}
fn payloads(
    d: &Document,
    name: &str,
    kind: Kind,
    masks: &Library,
    cancel: &AtomicBool,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    assets::canceled(cancel)?;
    if !label(name)
        || !masks.valid()
        || d.memory_bytes() > NATIVE_LIMIT
        || kind == Kind::Animation && d.animation.is_none()
    {
        return Err(
            "Choose a valid name, bounded document and an animation for timeline templates".into(),
        );
    }
    // A full page snapshot preserves editable layer/folder/cut/timeline dependencies together.
    let mut copy = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
    copy.light_table = d.light_table.clone();
    let native = crate::files::native_bytes(&copy).map_err(|e| e.to_string())?;
    let masks = serde_json::to_vec(masks).map_err(|e| e.to_string())?;
    if native.len() > NATIVE_LIMIT || masks.len() > MASK_LIMIT {
        return Err("Template bundle exceeds 64MiB native / 8MiB masks".into());
    }
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("template.drawer".into(), native),
        ("masks.json".into(), masks),
    ];
    let m = Manifest {
        version: 1,
        native_version: crate::files::VERSION,
        name: name.into(),
        kind,
        frame: d.current_frame,
        files: values
            .iter()
            .map(|(p, b)| File {
                path: p.clone(),
                bytes: b.len(),
                sha256: assets::hash(b),
            })
            .collect(),
    };
    values.push((
        "manifest.json".into(),
        serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?,
    ));
    assets::canceled(cancel)?;
    Ok(values)
}
/// Publishes only a new directory. Existing destinations and foreign temporary files survive.
pub fn export(
    d: &Document,
    path: &Path,
    name: &str,
    kind: Kind,
    masks: &Library,
    cancel: &AtomicBool,
) -> Result<(), String> {
    crate::file_objects::local_path(path)?;
    let parent = path
        .parent()
        .ok_or("Missing template parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let name_on_disk = path.file_name().ok_or("Missing template folder name")?;
    let destination = assets::checked(&parent, Path::new(name_on_disk), false)?;
    if destination.exists() {
        return Err("Choose a new template folder; existing files are never overwritten".into());
    }
    let values = payloads(d, name, kind, masks, cancel)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = parent.join(format!(
        ".drawer-template-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&temp).map_err(|e| e.to_string())?;
    let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in values {
            assets::canceled(cancel)?;
            let p = assets::checked(&temp, Path::new(&name), false)?;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .map_err(|e| e.to_string())?;
            created.push(p);
            for chunk in bytes.chunks(65536) {
                assets::canceled(cancel)?;
                f.write_all(chunk).map_err(|e| e.to_string())?;
            }
            f.sync_all().map_err(|e| e.to_string())?;
        }
        assets::canceled(cancel)?;
        if destination.exists() {
            return Err("Template destination appeared; publication refused".into());
        }
        fs::rename(&temp, &destination).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        for p in created.iter().rev() {
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_dir(&temp);
    }
    result
}
pub struct Prepared {
    pub name: String,
    pub kind: Kind,
    pub document: Document,
    pub masks: Library,
    pub source: PathBuf,
}
impl Prepared {
    /// New IDs in the user's library; frozen document references stay immutable.
    pub fn copied_masks(&self, current: &Library) -> Result<Library, String> {
        let mut next = current.clone();
        for a in self.masks.assets() {
            next = next
                .register_asset(a.clone(), true)
                .map_err(|e| e.to_string())?;
        }
        Ok(next)
    }
}
/// Entire bundle is validated before returning an independent, unsaved document.
pub fn import(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Prepared, String> {
    crate::file_objects::local_path(path)?;
    let root = path.canonicalize().map_err(|e| e.to_string())?;
    assets::checked(path, Path::new("manifest.json"), true)?;
    let m: Manifest = serde_json::from_slice(&read(&root, "manifest.json", 65536, cancel)?)
        .map_err(|e| e.to_string())?;
    if m.version != 1
        || m.native_version == 0
        || m.native_version > crate::files::VERSION
        || !label(&m.name)
        || m.files.len() != 2
        || m.files[0].path != "template.drawer"
        || m.files[1].path != "masks.json"
    {
        return Err("Unsupported template manifest/version or resource path".into());
    }
    for p in fs::read_dir(&root).map_err(|e| e.to_string())? {
        let p = p.map_err(|e| e.to_string())?;
        if !["manifest.json", "template.drawer", "masks.json"]
            .iter()
            .any(|n| p.file_name() == *n)
        {
            return Err("Unexpected template resource".into());
        }
    }
    if budget < NATIVE_LIMIT.min(m.files[0].bytes).saturating_mul(3) + m.files[1].bytes {
        return Err("Insufficient workspace memory for template validation".into());
    }
    let mut data = Vec::new();
    for (f, limit) in m.files.iter().zip([NATIVE_LIMIT, MASK_LIMIT]) {
        if f.bytes > limit || !assets::valid_hash(&f.sha256) {
            return Err("Invalid template resource size/hash".into());
        }
        let bytes = read(&root, &f.path, limit, cancel)?;
        if bytes.len() != f.bytes || assets::hash(&bytes) != f.sha256 {
            return Err("Missing/changed template resource".into());
        }
        data.push(bytes);
    }
    let b = &data[0];
    if b.get(..8) != Some(b"DRAWER\r\n")
        || b.get(8..12)
            .is_none_or(|s| u32::from_le_bytes(s.try_into().unwrap()) != m.native_version)
    {
        return Err("Template native version mismatch".into());
    }
    let mut d = crate::files::read_native_snapshot(
        &root.join("template.drawer"),
        b,
        budget.saturating_sub(b.len()),
    )
    .map_err(|e| e.to_string())?;
    let masks: Library = serde_json::from_slice(&data[1]).map_err(|e| e.to_string())?;
    if !masks.valid()
        || m.kind == Kind::Animation && d.animation.is_none()
        || m.frame >= d.animation.as_ref().map_or(1, |a| a.frames)
    {
        return Err("Invalid/duplicate template mask IDs or timeline".into());
    }
    assets::canceled(cancel)?;
    d.history = Default::default();
    d.path = None;
    d.source_path = None;
    d.title = m.name.clone();
    d.dirty = true;
    d.current_frame = m.frame;
    Ok(Prepared {
        name: m.name,
        kind: m.kind,
        document: d,
        masks,
        source: root,
    })
}
#[path = "template_bundle_verify.rs"]
pub mod verify;
