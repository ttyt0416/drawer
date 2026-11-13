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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    embedded_links: Vec<EmbeddedLink>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddedLink {
    target: crate::layer_search::Key,
    file: String,
}
fn cached_png(link: &crate::file_objects::Link) -> Result<Vec<u8>, String> {
    let crate::material_layer::Source::Image(image) = &link.cache.source else {
        return Err("Linked cache is not an image".into());
    };
    crate::references::LocalImage {
        width: image.width,
        height: image.height,
        pixels: image.pixels.as_ref().clone(),
    }
    .png()
    .map_err(|e| e.to_string())
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
    embedded_root: Option<&Path>,
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
    crate::text_appearance::freeze_all(&mut copy.scene)?;
    let mut embedded_links = Vec::new();
    let mut images = Vec::new();
    if let Some(root) = embedded_root {
        for (n, link) in copy.scene.file_objects.entries.iter_mut().enumerate() {
            assets::canceled(cancel)?;
            let file = format!("linked-{n:03}.png");
            let png = cached_png(link)?;
            if png.len() > 16 * 1024 * 1024
                || images
                    .iter()
                    .map(|(_, b): &(String, Vec<u8>)| b.len())
                    .sum::<usize>()
                    .saturating_add(png.len())
                    > NATIVE_LIMIT
            {
                return Err("Portable linked caches exceed16MiB each /64MiB total".into());
            }
            embedded_links.push(EmbeddedLink {
                target: link.target,
                file: file.clone(),
            });
            images.push((file.clone(), png));
            link.path = root.join(file);
            link.selection = Default::default();
        }
    }
    let native = crate::files::native_bytes(&copy).map_err(|e| e.to_string())?;
    let masks = serde_json::to_vec(masks).map_err(|e| e.to_string())?;
    if native.len() > NATIVE_LIMIT || masks.len() > MASK_LIMIT {
        return Err("Template bundle exceeds 64MiB native / 8MiB masks".into());
    }
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("template.drawer".into(), native),
        ("masks.json".into(), masks),
    ];
    values.extend(images);
    let m = Manifest {
        version: if embedded_root.is_some() { 2 } else { 1 },
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
        embedded_links,
    };
    let manifest = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
    if manifest.len() > 65536 {
        return Err("Portable bundle manifest exceeds64KiB".into());
    }
    values.push(("manifest.json".into(), manifest));
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
    export_resources(d, path, name, kind, masks, cancel, false)
}
pub fn export_resources(
    d: &Document,
    path: &Path,
    name: &str,
    kind: Kind,
    masks: &Library,
    cancel: &AtomicBool,
    embed_linked_caches: bool,
) -> Result<(), String> {
    let path = crate::file_browser::new_path(path)?;
    let parent = crate::file_browser::folder(path.parent().ok_or("Missing template parent")?)?;
    let name_on_disk = path.file_name().ok_or("Missing template folder name")?;
    let destination = assets::checked(&parent, Path::new(name_on_disk), false)?;
    if destination.exists() {
        return Err("Choose a new template folder; existing files are never overwritten".into());
    }
    let values = payloads(
        d,
        name,
        kind,
        masks,
        cancel,
        embed_linked_caches.then_some(destination.as_path()),
    )?;
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
    pub notices: Vec<String>,
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
    let root = crate::file_browser::folder(path)?;
    assets::checked(path, Path::new("manifest.json"), true)?;
    let m: Manifest = serde_json::from_slice(&read(&root, "manifest.json", 65536, cancel)?)
        .map_err(|e| e.to_string())?;
    if ![1, 2].contains(&m.version)
        || m.native_version == 0
        || m.native_version > crate::files::VERSION
        || !label(&m.name)
        || m.embedded_links.len() > 256
        || m.version == 1 && !m.embedded_links.is_empty()
        || m.files.len() != 2 + m.embedded_links.len()
        || m.files[0].path != "template.drawer"
        || m.files[1].path != "masks.json"
    {
        return Err("Unsupported template manifest/version or resource path".into());
    }
    let mut targets = std::collections::BTreeSet::new();
    for (n, link) in m.embedded_links.iter().enumerate() {
        if link.file != format!("linked-{n:03}.png")
            || m.files[n + 2].path != link.file
            || !targets.insert(link.target)
        {
            return Err("Invalid portable resource map".into());
        }
    }
    for p in fs::read_dir(&root).map_err(|e| e.to_string())? {
        let p = p.map_err(|e| e.to_string())?;
        if p.file_name() != "manifest.json"
            && !m.files.iter().any(|f| p.file_name() == f.path.as_str())
        {
            return Err("Unexpected template resource".into());
        }
    }
    let total = m
        .files
        .iter()
        .try_fold(0usize, |n, f| n.checked_add(f.bytes))
        .ok_or("Bundle size overflow")?;
    if total > 136 * 1024 * 1024
        || budget
            < NATIVE_LIMIT
                .min(m.files[0].bytes)
                .saturating_mul(3)
                .saturating_add(total)
    {
        return Err("Insufficient workspace memory for template validation".into());
    }
    let mut data = Vec::new();
    for (n, f) in m.files.iter().enumerate() {
        let limit = match n {
            0 => NATIVE_LIMIT,
            1 => MASK_LIMIT,
            _ => 16 * 1024 * 1024,
        };
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
        budget.saturating_sub(total),
    )
    .map_err(|e| e.to_string())?;
    for (n, link) in m.embedded_links.iter().enumerate() {
        let cached = d
            .scene
            .file_objects
            .entries
            .iter_mut()
            .find(|l| l.target == link.target)
            .ok_or("Portable resource target missing")?;
        if cached_png(cached)? != data[n + 2] {
            return Err("Portable PNG differs from native cache".into());
        }
        cached.path = root.join(&link.file);
        cached.selection = Default::default();
    }
    let mut notices = vec![format!(
        "{} linked image caches copied with relative bundle names. Model geometry/textures, audio, image materials and text appearance caches are embedded in the native document.",
        m.embedded_links.len()
    )];
    if d.scene.file_objects.entries.len() > m.embedded_links.len() {
        notices.push("Original source links retained; their files are not probed automatically. Use cached appearances or explicitly relink to a local file.".into());
    }
    let mut fonts = Vec::new();
    for object in &d.scene.objects {
        if let crate::scene::PageObject::Text {
            layout: Some(style),
            ..
        } = object
        {
            for font in style.fonts() {
                if let crate::text_layout::Font::Local(reference) = font
                    && !fonts.contains(&font)
                    && crate::font_library::data(reference).is_none()
                {
                    fonts.push(font);
                    notices.push(format!("Missing font: {}. Unicode, formatting, layout version and cached glyph appearance are preserved. Select and review a local replacement before editing; no font file is redistributed.",font.label()));
                }
            }
        }
    }
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
        notices,
    })
}
#[path = "template_bundle_verify.rs"]
pub mod verify;
