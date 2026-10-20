//! Explicit source selection and reviewed refresh batches; normal drawing reads only caches.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    file_objects::{self as links, Link},
    layer_search::Key,
    material_layer::{Image, Source},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::SystemTime,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    #[default]
    Page,
    Layer(Key),
    Timeline,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub content: Content,
    pub comp: Option<u64>,
    pub frame: u32,
}
impl Selection {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.frame < crate::animation::MAX_FRAMES
            && self.comp.is_none_or(|n| n > 0)
            && match self.content {
                Content::Layer(Key::Selection(_)) => false,
                Content::Layer(Key::Layer(crate::layer_properties::Target::Folder(n))) => n < 256,
                Content::Layer(Key::Layer(
                    crate::layer_properties::Target::Raster(n)
                    | crate::layer_properties::Target::Vector(n),
                ))
                | Content::Layer(
                    Key::Object(n) | Key::Gradient(n) | Key::World(n) | Key::Track(n),
                ) => n > 0,
                Content::Layer(Key::Cell { track, cell }) => track > 0 && cell > 0,
                Content::Layer(_) => false,
                _ => true,
            }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub path: PathBuf,
    pub length: u64,
    pub modified: SystemTime,
    pub fingerprint: u64,
}
impl Stamp {
    pub fn read(path: &Path) -> Result<Self, String> {
        links::local_path(path)?;
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        links::local_path(&path)?;
        let mut f = fs::File::open(&path).map_err(|e| e.to_string())?;
        let before = f.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > 64 * 1024 * 1024 {
            return Err("Source snapshot exceeds 64 MiB".into());
        }
        let mut hash = 0xcbf29ce484222325u64;
        let mut length = 0u64;
        let mut buffer = [0; 65536];
        loop {
            let n = f.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            length += n as u64;
            if length > 64 * 1024 * 1024 {
                return Err("Source grew beyond 64 MiB".into());
            }
            for byte in &buffer[..n] {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        }
        let after = f.metadata().map_err(|e| e.to_string())?;
        if length != before.len()
            || after.len() != before.len()
            || after.modified().ok() != before.modified().ok()
        {
            return Err("Source changed while reading; retry the preview".into());
        }
        Ok(Self {
            path,
            length,
            modified: after.modified().map_err(|e| e.to_string())?,
            fingerprint: hash,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if Self::read(&self.path)? != *self {
            Err(format!(
                "Source changed after preview: {}",
                self.path.display()
            ))
        } else {
            Ok(())
        }
    }
}
fn encoded_snapshot(path: &Path) -> Result<(Stamp, Vec<u8>), String> {
    let stamp = Stamp::read(path)?;
    let file = fs::File::open(&stamp.path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(stamp.length as usize)
        .map_err(|e| e.to_string())?;
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |value, byte| {
        (value ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    if bytes.len() as u64 != stamp.length || hash != stamp.fingerprint {
        return Err("Source changed while capturing its encoded snapshot".into());
    }
    stamp.validate()?;
    Ok((stamp, bytes))
}
pub struct Metadata {
    pub native: bool,
    pub frames: u32,
    pub timeline: bool,
    pub layers: Vec<(Key, String)>,
    pub comps: Vec<(u64, String)>,
}
pub fn inspect(path: &Path) -> Result<Metadata, String> {
    let (stamp, bytes) = encoded_snapshot(path)?;
    let info = if native(&stamp.path) {
        let d =
            crate::files::read_native_snapshot(&stamp.path, &bytes, DEFAULT_DOCUMENT_BUDGET / 8)
                .map_err(|e| e.to_string())?;
        Metadata {
            native: true,
            frames: d.animation.as_ref().map_or(1, |a| a.frames),
            timeline: d.animation.is_some(),
            layers: crate::layer_catalog::rows(&d)?
                .into_iter()
                .filter(|r| !matches!(r.key, Key::Selection(_)))
                .map(|r| (r.key, r.name))
                .collect(),
            comps: d
                .scene
                .layer_comps
                .comps
                .iter()
                .map(|c| (c.id, c.name.clone()))
                .collect(),
        }
    } else {
        let s = crate::image_import::ImageSource::from_encoded(
            stamp.path.clone(),
            bytes,
            DEFAULT_DOCUMENT_BUDGET / 4,
            &AtomicBool::new(false),
        )
        .map_err(|e| e.to_string())?;
        Metadata {
            native: false,
            frames: s.pages.len() as u32,
            timeline: false,
            layers: vec![],
            comps: vec![],
        }
    };
    stamp.validate()?;
    Ok(info)
}
fn native(path: &Path) -> bool {
    path.extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("drawer"))
}
fn dependencies(
    path: &Path,
    owner: Option<&Path>,
    stack: &mut Vec<PathBuf>,
    stamps: &mut Vec<Stamp>,
) -> Result<(), String> {
    let (stamp, bytes) = encoded_snapshot(path)?;
    if owner == Some(stamp.path.as_path()) || stack.contains(&stamp.path) {
        return Err("Circular local file reference".into());
    }
    if stack.len() >= 8 || stamps.len() >= 64 {
        return Err("Nested file references exceed depth 8 or 64 files".into());
    }
    let children = if native(&stamp.path) {
        let d =
            crate::files::read_native_snapshot(&stamp.path, &bytes, DEFAULT_DOCUMENT_BUDGET / 8)
                .map_err(|e| e.to_string())?;
        d.scene
            .file_objects
            .entries
            .iter()
            .map(|l| l.path.clone())
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    drop(bytes);
    stack.push(stamp.path.clone());
    stamps.push(stamp);
    for child in children {
        dependencies(&child, owner, stack, stamps)?;
    }
    stack.pop();
    Ok(())
}
pub struct Snapshot {
    pub path: PathBuf,
    pub image: Image,
    pub stamps: Vec<Stamp>,
}
pub fn read(path: &Path, owner: Option<&Path>, selection: &Selection) -> Result<Snapshot, String> {
    read_checked(path, owner, selection, || Ok(()))
}
fn read_checked(
    path: &Path,
    owner: Option<&Path>,
    selection: &Selection,
    after_decode: impl FnOnce() -> Result<(), String>,
) -> Result<Snapshot, String> {
    if !selection.valid() {
        return Err("Invalid source layer/comp/frame selection".into());
    }
    let owner = owner.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_owned()));
    let mut stamps = vec![];
    dependencies(path, owner.as_deref(), &mut vec![], &mut stamps)?;
    let path = stamps
        .first()
        .ok_or("Missing source snapshot")?
        .path
        .clone();
    let (captured, bytes) = encoded_snapshot(&path)?;
    if stamps.first() != Some(&captured) {
        return Err("Root source changed during dependency inspection".into());
    }
    let image = if native(&path) {
        let mut d = crate::files::read_native_snapshot(&path, &bytes, DEFAULT_DOCUMENT_BUDGET / 8)
            .map_err(|e| e.to_string())?;
        if selection.frame >= d.animation.as_ref().map_or(1, |a| a.frames) {
            return Err("Selected source frame no longer exists".into());
        }
        d.current_frame = selection.frame;
        if let Some(id) = selection.comp {
            let prepared = crate::layer_comps::prepare(&d, crate::layer_comps::Command::Apply(id))?;
            if !prepared.missing.is_empty() {
                return Err(
                    "Selected source comp has missing layers; repair it in the source document"
                        .into(),
                );
            }
            prepared.state.restore(&mut d);
        }
        if let Content::Layer(key) = selection.content
            && crate::layer_catalog::thumbnail(&d, key).is_none()
        {
            return Err("Selected source layer no longer exists".into());
        }
        if selection.content == Content::Timeline && d.animation.is_none() {
            return Err("Selected source timeline no longer exists".into());
        }
        capture(&d, selection.content)?
    } else {
        if selection.content != Content::Page || selection.comp.is_some() {
            return Err("Image exchange sources support page/frame selection; layers and comps require a native document".into());
        }
        let cancel = AtomicBool::new(false);
        let source = crate::image_import::ImageSource::from_encoded(
            path.clone(),
            bytes,
            DEFAULT_DOCUMENT_BUDGET / 4,
            &cancel,
        )
        .map_err(|e| e.to_string())?;
        let decoded = source
            .decode(
                selection.frame as usize,
                true,
                DEFAULT_DOCUMENT_BUDGET / 4,
                &cancel,
            )
            .map_err(|e| e.to_string())?
            .image;
        if decoded.pixels.len() > 4 * 1024 * 1024 {
            return Err("Source image exceeds 4 million pixels".into());
        }
        Image {
            width: decoded.width,
            height: decoded.height,
            pixels: Arc::new(decoded.pixels),
        }
    };
    after_decode()?;
    for stamp in &stamps {
        stamp.validate()?;
    }
    Ok(Snapshot {
        path,
        image,
        stamps,
    })
}
fn capture(d: &Document, content: Content) -> Result<Image, String> {
    let count = d.width as usize * d.height as usize;
    if count > 4 * 1024 * 1024 {
        return Err("Selected source exceeds 4 million pixels".into());
    }
    let references = crate::eyedropper::References {
        enabled: true,
        all_rasters: false,
        objects: false,
        vectors: false,
        animation: true,
        ..Default::default()
    };
    let renderer = if content == Content::Timeline {
        crate::render::Renderer::sampling(d, &references)
    } else {
        crate::render::Renderer::new(d)
    };
    if let Some(error) = renderer.error() {
        return Err(error.into());
    }
    let floating = (d.color != crate::color_storage::Config::legacy())
        .then(|| crate::float_render::Renderer::new(d))
        .transpose()?;
    let space = crate::color_profile::Transform::new(
        &d.color.profile,
        &crate::color_profile::RgbProfile::Srgb,
    )?;
    let mut pixels = Vec::with_capacity(count);
    for y in 0..d.height {
        for x in 0..d.width {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let sample = match content {
                Content::Layer(key) => {
                    if let Some(r) = &floating {
                        r.source_pixel(key, point)?
                    } else {
                        crate::color_precision::Sample::from8(
                            renderer.source_pixel_unbounded(key, point),
                        )
                    }
                }
                _ => renderer.sample(point)?,
            };
            pixels.push(space.map(sample)?.rgba8());
        }
    }
    Ok(Image {
        width: d.width,
        height: d.height,
        pixels: Arc::new(pixels),
    })
}
pub struct Prepared {
    pub state: DocumentState,
    pub succeeded: Vec<Key>,
    pub failed: Vec<(Key, String)>,
    pub stamps: Vec<Stamp>,
}
impl Prepared {
    pub fn validate(&self, d: &Document) -> Result<(), String> {
        for stamp in &self.stamps {
            stamp.validate()?;
        }
        links::validate(d, &self.state)
    }
}
pub fn refresh(
    d: &Document,
    requests: &[(Key, Option<PathBuf>, Selection)],
) -> Result<Prepared, String> {
    if requests.is_empty()
        || requests.len() > 256
        || requests
            .iter()
            .map(|v| v.0)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != requests.len()
    {
        return Err("Choose 1–256 unique linked source requests".into());
    }
    let mut result = Prepared {
        state: d.snapshot(),
        succeeded: vec![],
        failed: vec![],
        stamps: vec![],
    };
    for (key, path, selection) in requests {
        let attempt = (|| {
            let current = d
                .scene
                .file_objects
                .get(*key)
                .ok_or("Missing linked layer")?;
            let target =
                crate::layer_catalog::target(d, *key).ok_or("Missing linked layer target")?;
            let flags = links::stored_flags(d, target);
            if flags.locked || flags.pixels_locked {
                return Err("Unlock this source layer before refreshing".into());
            }
            let snapshot = read(
                path.as_deref().unwrap_or(&current.path),
                d.path.as_deref(),
                selection,
            )?;
            let mut next = current.clone();
            next.path = snapshot.path;
            next.selection = selection.clone();
            let old_size = match &next.cache.source {
                Source::Image(i) => [i.width as f64, i.height as f64],
                _ => return Err("Invalid linked image cache".into()),
            };
            if next.cache.area.min == [0.; 2] && next.cache.area.max == old_size {
                next.cache.area.max = [snapshot.image.width as f64, snapshot.image.height as f64];
            }
            next.cache.source = Source::Image(snapshot.image);
            let i = result
                .state
                .scene
                .file_objects
                .entries
                .iter()
                .position(|l| l.target == *key)
                .ok_or("Missing prepared link")?;
            let original = std::mem::replace(&mut result.state.scene.file_objects.entries[i], next);
            if let Err(error) = links::validate(d, &result.state) {
                result.state.scene.file_objects.entries[i] = original;
                return Err(error);
            }
            result.stamps.extend(snapshot.stamps);
            Ok(())
        })();
        match attempt {
            Ok(()) => result.succeeded.push(*key),
            Err(error) => {
                let path = d
                    .scene
                    .file_objects
                    .get(*key)
                    .map(|l| l.path.display().to_string())
                    .unwrap_or_else(|| "Missing source layer".into());
                result.failed.push((*key, format!("{path} — {error}")));
            }
        }
    }
    result.stamps.sort_by(|a, b| a.path.cmp(&b.path));
    result.stamps.dedup();
    // If two requests saw different versions of the same dependency, neither
    // version may silently pass as one coherent reviewed batch.
    for pair in result.stamps.windows(2) {
        if pair[0].path == pair[1].path && pair[0] != pair[1] {
            return Err("A shared source changed during batch preparation; retry".into());
        }
    }
    result.validate(d)?;
    Ok(result)
}
pub fn refresh_all(d: &Document) -> Result<Prepared, String> {
    refresh(
        d,
        &d.scene
            .file_objects
            .entries
            .iter()
            .map(|l| (l.target, None, l.selection.clone()))
            .collect::<Vec<_>>(),
    )
}
pub fn create(
    d: &Document,
    path: &Path,
    name: &str,
    selection: Selection,
) -> Result<(DocumentState, u64, Vec<Stamp>), String> {
    let snapshot = read(path, d.path.as_deref(), &selection)?;
    let mut layer = crate::material_layer::Layer {
        source: Source::Image(snapshot.image),
        area: crate::scene::Rect {
            min: [0.; 2],
            max: [0.; 2],
        },
        transform: d.scene.transform.inverse(),
        repeat: [false; 2],
        mapping: Default::default(),
    };
    if let Source::Image(i) = &layer.source {
        layer.area.max = [i.width as f64, i.height as f64];
    }
    let mut placeholder = layer.clone();
    placeholder.source = Source::Solid([0; 4]);
    let (mut state, id) = crate::material_layer::prepare(d, None, name, placeholder)?;
    state.scene.file_objects.entries.push(Link {
        target: Key::Object(id),
        path: snapshot.path,
        cache: layer,
        selection,
    });
    links::validate(d, &state)?;
    Ok((state, id, snapshot.stamps))
}
pub fn requires_v63(s: &crate::scene::Scene) -> bool {
    s.file_objects
        .entries
        .iter()
        .any(|l| !l.selection.is_default())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reject_change_during_decode_and_after_preview() {
        let root = crate::test_support::TempDir::new();
        let path =
            crate::file_objects_verify::source(&root.0, "changing.drawer", [1, 2, 3, 255]).unwrap();
        let result = super::read_checked(&path, None, &Default::default(), || {
            crate::file_objects_verify::source(&root.0, "changing.drawer", [4, 5, 6, 255])?;
            Ok(())
        });
        assert!(result.is_err());
        let stamp = super::Stamp::read(&path).unwrap();
        crate::file_objects_verify::source(&root.0, "changing.drawer", [7, 8, 9, 255]).unwrap();
        assert!(stamp.validate().is_err());
    }
}
