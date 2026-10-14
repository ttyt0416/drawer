use crate::{
    document::{Background, CanvasSpec, Document, MAX_DIMENSION},
    storage::UserStorage,
    view::View,
};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const REFERENCE_BUDGET: usize = 64 * 1024 * 1024;
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}
impl LocalImage {
    pub fn from_rgba(width: usize, height: usize, bytes: &[u8], budget: usize) -> io::Result<Self> {
        let count = width
            .checked_mul(height)
            .ok_or_else(|| invalid("Image size overflow"))?;
        let len = count
            .checked_mul(4)
            .ok_or_else(|| invalid("Image size overflow"))?;
        if !(1..=MAX_DIMENSION as usize).contains(&width)
            || !(1..=MAX_DIMENSION as usize).contains(&height)
            || len > budget
            || bytes.len() != len
        {
            return Err(invalid(
                "Invalid image dimensions, pixel data or memory budget",
            ));
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| invalid("Not enough reference memory"))?;
        pixels.extend(bytes.as_chunks::<4>().0.iter().copied());
        Ok(Self {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
    pub fn read(path: &Path, budget: usize) -> io::Result<Self> {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            return Err(invalid("Supported reference file format: PNG"));
        }
        let mut reader = image::ImageReader::open(path)?;
        reader.set_format(image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(budget as u64);
        reader.limits(limits);
        let rgba = reader
            .decode()
            .map_err(|e| invalid(e.to_string()))?
            .into_rgba8();
        Self::from_rgba(
            rgba.width() as usize,
            rgba.height() as usize,
            rgba.as_raw(),
            budget,
        )
    }
    pub fn memory_bytes(&self) -> usize {
        self.pixels.len() * 4
    }
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.width <= MAX_DIMENSION
            && self.height > 0
            && self.height <= MAX_DIMENSION
            && self.pixels.len() == self.width as usize * self.height as usize
    }
    pub fn sample(&self, point: [f64; 2]) -> Option<[u8; 4]> {
        if point.iter().any(|v| !v.is_finite())
            || point[0] < 0.0
            || point[1] < 0.0
            || point[0] >= self.width as f64
            || point[1] >= self.height as f64
        {
            return None;
        }
        Some(self.pixels[(point[1] as u32 * self.width + point[0] as u32) as usize])
    }
    pub fn document(&self, budget: usize) -> io::Result<Document> {
        if self.pixels.len() != self.width as usize * self.height as usize {
            return Err(invalid("Image pixel count does not match its dimensions"));
        }
        let mut doc = Document::create(
            CanvasSpec {
                width: self.width,
                height: self.height,
                dpi: 300.0,
                background: Background::Transparent,
            },
            budget,
        )
        .map_err(|e| invalid(e.to_string()))?;
        doc.layers[0].pixels.copy_from_slice(&self.pixels);
        Ok(doc)
    }
    fn png(&self) -> io::Result<Vec<u8>> {
        if !self.valid() {
            return Err(invalid("Invalid reference pixels"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.memory_bytes())
            .map_err(|_| invalid("Not enough image memory"))?;
        for pixel in &self.pixels {
            bytes.extend_from_slice(pixel);
        }
        let mut result = Vec::new();
        image::codecs::png::PngEncoder::new(&mut result)
            .write_image(
                &bytes,
                self.width,
                self.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| invalid(e.to_string()))?;
        Ok(result)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRecord {
    pub id: u64,
    pub name: String,
    pub source: Option<PathBuf>,
    pub copy: String,
    pub view: View,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceWorkspace {
    pub version: u32,
    pub next_id: u64,
    pub entries: Vec<ReferenceRecord>,
    pub active: Option<u64>,
    pub color: [u8; 4],
    pub auto_pick: bool,
    pub show_ring: bool,
    pub visible: bool,
    pub position: [f32; 2],
    pub size: [f32; 2],
}
impl Default for ReferenceWorkspace {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            entries: Vec::new(),
            active: None,
            color: [0, 0, 0, 255],
            auto_pick: false,
            show_ring: true,
            visible: false,
            position: [220.0, 100.0],
            size: [560.0, 480.0],
        }
    }
}
impl ReferenceWorkspace {
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        self.version == 1
            && self.entries.len() <= 64
            && self.next_id > 0
            && self.entries.iter().all(|r| {
                r.id > 0
                    && r.id < self.next_id
                    && ids.insert(r.id)
                    && !r.name.is_empty()
                    && r.name.len() <= 1024
                    && r.copy == format!("reference-{}.png", r.id)
                    && View::decode(&r.view.encode()).is_some()
            })
            && self.active.is_none_or(|id| ids.contains(&id))
            && self
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e6)
            && self
                .size
                .iter()
                .all(|v| v.is_finite() && (240.0..=4000.0).contains(v))
    }
    pub fn active_record(&self) -> Option<&ReferenceRecord> {
        let id = self.active?;
        self.entries.iter().find(|r| r.id == id)
    }
    pub fn active_record_mut(&mut self) -> Option<&mut ReferenceRecord> {
        let id = self.active?;
        self.entries.iter_mut().find(|r| r.id == id)
    }
    pub fn step(&mut self, delta: i32) {
        if self.entries.is_empty() {
            self.active = None;
            return;
        }
        let index = self
            .entries
            .iter()
            .position(|r| Some(r.id) == self.active)
            .unwrap_or(0);
        self.active = Some(
            self.entries[(index as i32 + delta).rem_euclid(self.entries.len() as i32) as usize].id,
        );
    }
}
fn assets(storage: &UserStorage) -> io::Result<PathBuf> {
    let directory = storage.root().join("materials/references");
    fs::create_dir_all(&directory)?;
    if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
        return Err(invalid(
            "Reference copies must use a regular local directory",
        ));
    }
    Ok(directory)
}
fn settings(storage: &UserStorage) -> PathBuf {
    storage.root().join("settings/references.json")
}
fn read_workspace(path: &Path) -> io::Result<ReferenceWorkspace> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_SETTINGS_BYTES {
        return Err(invalid("Reference settings are too large"));
    }
    let workspace: ReferenceWorkspace =
        serde_json::from_reader(file).map_err(|e| invalid(e.to_string()))?;
    if !workspace.valid() {
        return Err(invalid("Unsupported or damaged reference settings"));
    }
    Ok(workspace)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = path
        .file_name()
        .ok_or_else(|| invalid("Missing filename"))?
        .to_owned();
    name.push(".write.tmp");
    let temporary = path.with_file_name(name);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
fn save_workspace(storage: &UserStorage, workspace: &ReferenceWorkspace) -> io::Result<()> {
    if !workspace.valid() {
        return Err(invalid("Invalid reference workspace"));
    }
    let path = settings(storage);
    if path.exists() {
        let backup = match read_workspace(&path) {
            Ok(_) => "references.previous.json",
            Err(e) if e.kind() == io::ErrorKind::InvalidData => "references.damaged.json",
            Err(e) => return Err(e),
        };
        if fs::metadata(&path)?.len() > MAX_SETTINGS_BYTES * 8 {
            return Err(invalid(
                "Reference settings exceed the recovery budget; the original is preserved",
            ));
        }
        atomic_write(
            &storage.root().join("settings").join(backup),
            &fs::read(&path)?,
        )?;
    }
    let bytes = serde_json::to_vec_pretty(workspace).map_err(|e| invalid(e.to_string()))?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(invalid("Reference settings are too large"));
    }
    atomic_write(&path, &bytes)
}

pub struct ReferenceSession {
    pub workspace: ReferenceWorkspace,
    pub images: HashMap<u64, Arc<LocalImage>>,
    pub missing: HashMap<u64, String>,
    saved: ReferenceWorkspace,
    undo: Vec<ReferenceWorkspace>,
    redo: Vec<ReferenceWorkspace>,
}
impl Default for ReferenceSession {
    fn default() -> Self {
        let workspace = ReferenceWorkspace::default();
        Self {
            saved: workspace.clone(),
            workspace,
            images: HashMap::new(),
            missing: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}
impl ReferenceSession {
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        let workspace = match read_workspace(&settings(storage)) {
            Ok(w) => w,
            Err(e) if e.kind() == io::ErrorKind::NotFound => ReferenceWorkspace::default(),
            Err(e) => return Err(e),
        };
        let mut session = Self {
            saved: workspace.clone(),
            workspace,
            ..Self::default()
        };
        session.load_images(storage)?;
        Ok(session)
    }
    fn load_images(&mut self, storage: &UserStorage) -> io::Result<()> {
        let directory = assets(storage)?;
        let mut used = 0;
        self.images.clear();
        self.missing.clear();
        for record in &self.workspace.entries {
            let copy = directory.join(&record.copy);
            let result = LocalImage::read(&copy, REFERENCE_BUDGET - used).or_else(|copy_error| {
                if let Some(source) = &record.source {
                    LocalImage::read(source, REFERENCE_BUDGET - used)
                } else {
                    Err(copy_error)
                }
            });
            match result {
                Ok(image) => {
                    used += image.memory_bytes();
                    self.images.insert(record.id, Arc::new(image));
                }
                Err(error) => {
                    self.missing.insert(
                        record.id,
                        format!("Reference missing or unreadable: {error}"),
                    );
                }
            }
        }
        Ok(())
    }
    pub fn memory_bytes(&self) -> usize {
        self.images.values().map(|image| image.memory_bytes()).sum()
    }
    pub fn active_image(&self) -> Option<&LocalImage> {
        self.images.get(&self.workspace.active?).map(AsRef::as_ref)
    }
    pub fn persist(&mut self, storage: &UserStorage) -> io::Result<()> {
        if self.workspace == self.saved {
            return Ok(());
        }
        if let Err(error) = save_workspace(storage, &self.workspace) {
            self.workspace = self.saved.clone();
            return Err(error);
        }
        self.undo.push(self.saved.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.saved = self.workspace.clone();
        Ok(())
    }
    pub fn add(
        &mut self,
        storage: &UserStorage,
        image: LocalImage,
        name: String,
        source: Option<PathBuf>,
    ) -> io::Result<u64> {
        if !image.valid()
            || image.memory_bytes() > REFERENCE_BUDGET.saturating_sub(self.memory_bytes())
            || self.workspace.entries.len() >= 64
            || name.is_empty()
            || name.len() > 1024
        {
            return Err(invalid("Reference exceeds the available image/list budget"));
        }
        let directory = assets(storage)?;
        let mut id = self.workspace.next_id;
        for _ in 0..1024 {
            if !directory.join(format!("reference-{id}.png")).exists() {
                break;
            }
            id = id
                .checked_add(1)
                .ok_or_else(|| invalid("Reference IDs exhausted"))?;
        }
        let copy = format!("reference-{id}.png");
        let path = directory.join(&copy);
        let next_id = id
            .checked_add(1)
            .ok_or_else(|| invalid("Reference IDs exhausted"))?;
        let bytes = image.png()?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        let original = self.workspace.clone();
        self.workspace.next_id = next_id;
        self.workspace.entries.push(ReferenceRecord {
            id,
            name,
            source,
            copy,
            view: View::default(),
        });
        self.workspace.active = Some(id);
        if let Err(error) = self.persist(storage) {
            self.workspace = original;
            let _ = fs::remove_file(path);
            return Err(error);
        }
        self.images.insert(id, Arc::new(image));
        Ok(id)
    }
    pub fn remove_active(&mut self, storage: &UserStorage) -> io::Result<()> {
        let Some(id) = self.workspace.active else {
            return Ok(());
        };
        let index = self
            .workspace
            .entries
            .iter()
            .position(|r| r.id == id)
            .unwrap();
        self.workspace.entries.remove(index);
        self.workspace.active = self
            .workspace
            .entries
            .get(index.min(self.workspace.entries.len().saturating_sub(1)))
            .map(|r| r.id);
        self.persist(storage)?;
        self.images.remove(&id);
        self.missing.remove(&id);
        Ok(())
    }
    pub fn undo(&mut self, storage: &UserStorage) -> io::Result<()> {
        let Some(previous) = self.undo.last().cloned() else {
            return Ok(());
        };
        assets(storage)?;
        save_workspace(storage, &previous)?;
        let current = self.saved.clone();
        self.undo.pop();
        self.redo.push(current);
        self.workspace = previous.clone();
        self.saved = previous;
        self.load_images(storage)
    }
    pub fn redo(&mut self, storage: &UserStorage) -> io::Result<()> {
        let Some(next) = self.redo.last().cloned() else {
            return Ok(());
        };
        assets(storage)?;
        save_workspace(storage, &next)?;
        self.redo.pop();
        self.undo.push(self.saved.clone());
        self.workspace = next.clone();
        self.saved = next;
        self.load_images(storage)
    }
    pub fn restore_previous(&mut self, storage: &UserStorage) -> io::Result<()> {
        assets(storage)?;
        let previous = read_workspace(&storage.root().join("settings/references.previous.json"))?;
        self.workspace = previous;
        self.persist(storage)?;
        self.load_images(storage)
    }
    pub fn reset(&mut self, storage: &UserStorage) -> io::Result<()> {
        self.workspace = ReferenceWorkspace::default();
        self.workspace.next_id = self.saved.next_id;
        self.workspace.visible = true;
        self.persist(storage)?;
        self.images.clear();
        self.missing.clear();
        Ok(())
    }
    pub fn export(&self, path: &Path) -> io::Result<()> {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("drawrefs"))
        {
            return Err(invalid("Choose a .drawrefs destination"));
        }
        let mut workspace = self.workspace.clone();
        for record in &mut workspace.entries {
            record.source = None;
        }
        let manifest = serde_json::to_vec(&workspace).map_err(|e| invalid(e.to_string()))?;
        let mut bytes = b"DRAWREF1".to_vec();
        bytes.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&manifest);
        for record in &workspace.entries {
            let image = self
                .images
                .get(&record.id)
                .ok_or_else(|| invalid("Resolve missing references before exporting"))?;
            // Encode the loaded pixels; this also includes a readable fallback source.
            let png = image.png()?;
            bytes.extend_from_slice(&(png.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&png);
            if bytes.len() > REFERENCE_BUDGET * 2 {
                return Err(invalid("Reference bundle exceeds export budget"));
            }
        }
        atomic_write(path, &bytes)
    }
    pub fn import(&mut self, storage: &UserStorage, path: &Path) -> io::Result<()> {
        let mut file = fs::File::open(path)?;
        if file.metadata()?.len() > REFERENCE_BUDGET as u64 * 2 {
            return Err(invalid("Reference bundle is too large"));
        }
        let mut magic = [0; 8];
        file.read_exact(&mut magic)?;
        if &magic != b"DRAWREF1" {
            return Err(invalid("Unsupported reference bundle"));
        }
        let mut length = [0; 4];
        file.read_exact(&mut length)?;
        let count = u32::from_le_bytes(length) as usize;
        if count as u64 > MAX_SETTINGS_BYTES {
            return Err(invalid("Reference manifest is too large"));
        }
        let mut json = vec![0; count];
        file.read_exact(&mut json)?;
        let mut workspace: ReferenceWorkspace =
            serde_json::from_slice(&json).map_err(|e| invalid(e.to_string()))?;
        if !workspace.valid() {
            return Err(invalid("Invalid reference bundle workspace"));
        }
        let directory = assets(storage)?;
        let mut staged = Vec::new();
        let mut images = HashMap::new();
        let mut used = 0;
        let old_active = workspace.active;
        let mut next_id = self.saved.next_id.max(workspace.next_id);
        let result = (|| {
            for record in &mut workspace.entries {
                let mut length = [0; 8];
                file.read_exact(&mut length)?;
                let length = u64::from_le_bytes(length);
                if length > REFERENCE_BUDGET as u64 {
                    return Err(invalid("Reference image is too large"));
                }
                let mut bytes = vec![0; length as usize];
                file.read_exact(&mut bytes)?;
                let mut reader = image::ImageReader::with_format(
                    io::Cursor::new(bytes),
                    image::ImageFormat::Png,
                );
                let mut limits = image::Limits::default();
                limits.max_alloc = Some((REFERENCE_BUDGET - used) as u64);
                limits.max_image_width = Some(MAX_DIMENSION);
                limits.max_image_height = Some(MAX_DIMENSION);
                reader.limits(limits);
                let rgba = reader
                    .decode()
                    .map_err(|e| invalid(e.to_string()))?
                    .into_rgba8();
                let image = LocalImage::from_rgba(
                    rgba.width() as usize,
                    rgba.height() as usize,
                    rgba.as_raw(),
                    REFERENCE_BUDGET - used,
                )?;
                used += image.memory_bytes();
                while directory.join(format!("reference-{next_id}.png")).exists() {
                    next_id = next_id
                        .checked_add(1)
                        .ok_or_else(|| invalid("Reference IDs exhausted"))?;
                }
                let id = next_id;
                next_id = next_id
                    .checked_add(1)
                    .ok_or_else(|| invalid("Reference IDs exhausted"))?;
                let path = directory.join(format!("reference-{id}.png"));
                let mut copy = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)?;
                staged.push(path.clone());
                copy.write_all(&image.png()?)?;
                copy.sync_all()?;
                if Some(record.id) == old_active {
                    workspace.active = Some(id);
                }
                record.id = id;
                record.copy = format!("reference-{id}.png");
                record.source = None;
                images.insert(id, Arc::new(image));
            }
            let mut trailing = [0];
            if file.read(&mut trailing)? != 0 {
                return Err(invalid("Unexpected reference bundle data"));
            }
            workspace.next_id = next_id;
            save_workspace(storage, &workspace)
        })();
        if let Err(error) = result {
            for path in staged {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        self.undo.push(self.saved.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.saved = workspace.clone();
        self.workspace = workspace;
        self.images = images;
        self.missing.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    fn sample() -> LocalImage {
        LocalImage::from_rgba(2, 1, &[240, 30, 80, 128, 20, 40, 60, 0], 8).unwrap()
    }
    fn source(root: &Path) -> PathBuf {
        let path = root.join("source.png");
        fs::write(&path, sample().png().unwrap()).unwrap();
        path
    }
    #[test]
    fn local_copy_views_alpha_color_and_layout_survive_restart_and_missing_sources() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("data")).unwrap();
        let path = source(&root.0);
        let original = fs::read(&path).unwrap();
        let mut session = ReferenceSession::load(&storage).unwrap();
        let id = session
            .add(
                &storage,
                LocalImage::read(&path, 1024).unwrap(),
                "Example".into(),
                Some(path.clone()),
            )
            .unwrap();
        session.workspace.active_record_mut().unwrap().view = View {
            zoom: 2.0,
            angle: 37.0,
            pan: [12.0, -7.0],
            flipped: false,
        };
        session.workspace.color = [240, 30, 80, 128];
        session.workspace.auto_pick = true;
        session.workspace.position = [80.0, 110.0];
        session.persist(&storage).unwrap();
        let loaded = ReferenceSession::load(&storage).unwrap();
        assert_eq!(loaded.workspace, session.workspace);
        assert_eq!(loaded.active_image().unwrap(), &sample());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(&path).unwrap();
        let loaded = ReferenceSession::load(&storage).unwrap();
        assert_eq!(loaded.active_image().unwrap(), &sample());
        assert!(loaded.missing.is_empty());
        fs::remove_file(
            assets(&storage)
                .unwrap()
                .join(format!("reference-{id}.png")),
        )
        .unwrap();
        let loaded = ReferenceSession::load(&storage).unwrap();
        assert!(loaded.active_image().is_none());
        assert!(loaded.missing[&id].contains("missing"));
        assert_eq!(loaded.workspace, session.workspace);
    }
    #[test]
    fn remove_and_undo_keep_original_and_portable_bundle_reopens_without_external_paths() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("data")).unwrap();
        let other = UserStorage::at(root.0.join("moved")).unwrap();
        let path = source(&root.0);
        let original = fs::read(&path).unwrap();
        let mut session = ReferenceSession::default();
        session
            .add(&storage, sample(), "Source".into(), Some(path.clone()))
            .unwrap();
        session
            .add(&storage, sample(), "Clipboard".into(), None)
            .unwrap();
        let active = session.workspace.active;
        session.workspace.step(-1);
        session.workspace.active_record_mut().unwrap().view.zoom = 3.0;
        session.persist(&storage).unwrap();
        session.workspace.step(1);
        assert_eq!(session.workspace.active, active);
        session.remove_active(&storage).unwrap();
        assert_eq!(session.workspace.entries.len(), 1);
        assert_eq!(fs::read(&path).unwrap(), original);
        session.undo(&storage).unwrap();
        assert_eq!(session.workspace.entries.len(), 2);
        session.redo(&storage).unwrap();
        assert_eq!(session.workspace.entries.len(), 1);
        let bundle = root.0.join("offline.drawrefs");
        session.export(&bundle).unwrap();
        fs::remove_file(path).unwrap();
        let mut moved = ReferenceSession::default();
        moved.import(&other, &bundle).unwrap();
        assert_eq!(moved.workspace.active_record().unwrap().view.zoom, 3.0);
        assert!(moved.workspace.entries.iter().all(|r| r.source.is_none()));
        assert_eq!(moved.active_image().unwrap(), &sample());
        assert_eq!(
            ReferenceSession::load(&other)
                .unwrap()
                .active_image()
                .unwrap(),
            &sample()
        );
    }
    #[test]
    fn failed_save_import_and_bad_metadata_preserve_workspace_files_and_foreign_temporary() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("data")).unwrap();
        let mut session = ReferenceSession::default();
        session.add(&storage, sample(), "One".into(), None).unwrap();
        let original = session.workspace.clone();
        let before = fs::read(settings(&storage)).unwrap();
        let temp = storage.root().join("settings/references.json.write.tmp");
        fs::write(&temp, "other writer").unwrap();
        assert!(session.add(&storage, sample(), "Two".into(), None).is_err());
        assert_eq!(session.workspace, original);
        assert_eq!(session.images.len(), 1);
        assert_eq!(fs::read(settings(&storage)).unwrap(), before);
        assert_eq!(fs::read_to_string(&temp).unwrap(), "other writer");
        fs::remove_file(temp).unwrap();
        assert_eq!(fs::read_dir(assets(&storage).unwrap()).unwrap().count(), 1);
        let bundle = root.0.join("bad.drawrefs");
        session.export(&bundle).unwrap();
        let mut bytes = fs::read(&bundle).unwrap();
        bytes.pop();
        fs::write(&bundle, bytes).unwrap();
        assert!(session.import(&storage, &bundle).is_err());
        assert_eq!(session.workspace, original);
        assert_eq!(fs::read_dir(assets(&storage).unwrap()).unwrap().count(), 1);
        assert_eq!(fs::read(settings(&storage)).unwrap(), before);
        let mut bad = original.clone();
        bad.entries[0].copy = "../escape.png".into();
        session.workspace = bad;
        assert!(session.persist(&storage).is_err());
        assert_eq!(session.workspace, original);
        fs::write(settings(&storage), b"broken").unwrap();
        assert!(ReferenceSession::load(&storage).is_err());
        session.workspace.show_ring = false;
        session.persist(&storage).unwrap();
        assert_eq!(
            fs::read(storage.root().join("settings/references.damaged.json")).unwrap(),
            b"broken"
        );
    }
    #[test]
    fn alpha_zero_rotated_sampling_and_new_drawing_are_independent_and_bad_images_are_rejected() {
        let image = sample();
        let view = View {
            zoom: 3.0,
            angle: 37.0,
            pan: [14.0, -17.0],
            flipped: true,
        };
        let size = [2.0, 1.0];
        let center = [300.0, 240.0];
        for (point, color) in [
            ([0.5, 0.5], [240, 30, 80, 128]),
            ([1.5, 0.5], [20, 40, 60, 0]),
        ] {
            let screen = view.document_to_screen(point, size, center, 1.5);
            assert_eq!(
                image.sample(view.screen_to_document(screen, size, center, 1.5)),
                Some(color)
            );
        }
        assert_eq!(image.sample([f64::NAN, 0.0]), None);
        assert_eq!(image.sample([-0.1, 0.5]), None);
        let mut doc = image.document(8).unwrap();
        assert!(doc.path.is_none() && doc.source_path.is_none() && doc.dirty);
        assert_eq!(doc.history_counts(), (0, 0));
        doc.edit_pixel(0, 0, 0, [0, 0, 0, 255]);
        assert_eq!(image, sample());
        for (w, h, bytes, budget) in [
            (0, 1, vec![], 8),
            (2, 1, vec![0; 7], 8),
            (2, 1, vec![0; 8], 7),
            (usize::MAX, 2, vec![], 8),
        ] {
            assert!(LocalImage::from_rgba(w, h, &bytes, budget).is_err());
        }
    }
}
