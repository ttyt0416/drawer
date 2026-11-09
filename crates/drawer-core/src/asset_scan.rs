use super::*;
use std::sync::atomic::AtomicUsize;
fn note(notes: &mut Vec<String>, s: impl Into<String>) {
    if notes.len() < 128 {
        notes.push(s.into().chars().take(1000).collect());
    }
}
fn clean(s: &str, max: usize) -> String {
    let s = s
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>();
    let s = s.trim();
    if s.is_empty() {
        "Unnamed".into()
    } else {
        s.into()
    }
}
fn inspect(
    path: &Path,
    b: &[u8],
    cancel: &AtomicBool,
) -> Result<(Kind, [u32; 2], Vec<u8>), String> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if ["png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp"].contains(&ext.as_str()) {
        let source = crate::image_import::ImageSource::from_encoded(
            path.to_owned(),
            b.to_vec(),
            128 * 1024 * 1024,
            cancel,
        )
        .map_err(|e| e.to_string())?;
        let page = source
            .decode(0, true, 128 * 1024 * 1024, cancel)
            .map_err(|e| e.to_string())?;
        let size = [page.image.width, page.image.height];
        let pixels = &page.image.pixels;
        let mut thumb = vec![0; 16 * 16 * 4];
        let scale = (16. / size[0] as f64).min(16. / size[1] as f64);
        let w = (size[0] as f64 * scale).round().clamp(1., 16.) as usize;
        let h = (size[1] as f64 * scale).round().clamp(1., 16.) as usize;
        for y in 0..h {
            for x in 0..w {
                let sx = (x as u64 * size[0] as u64 / w as u64) as usize;
                let sy = (y as u64 * size[1] as u64 / h as u64) as usize;
                let at = ((y + (16 - h) / 2) * 16 + x + (16 - w) / 2) * 4;
                thumb[at..at + 4].copy_from_slice(&pixels[sy * size[0] as usize + sx]);
            }
        }
        return Ok((Kind::Image, size, thumb));
    }
    if ["gpl", "csv"].contains(&ext.as_str()) {
        crate::palette_text::decode(path, b)?;
        return Ok((Kind::Palette, [0; 2], vec![]));
    }
    if ext == "json" {
        if path
            .file_name()
            .is_some_and(|s| s.eq_ignore_ascii_case("manifest.json"))
        {
            crate::tool_bundle::import(
                path.parent().ok_or("Missing tool folder")?,
                &Default::default(),
                &Default::default(),
            )
            .map_err(|e| e.to_string())?;
            return Ok((Kind::BrushBundle, [0; 2], vec![]));
        }
        if crate::palette_exchange::Bundle::decode(b).is_ok() {
            return Ok((Kind::Palette, [0; 2], vec![]));
        }
        if crate::brush_presets::Preset::decode(b).is_ok() {
            return Ok((Kind::Brush, [0; 2], vec![]));
        }
        if let Ok(l) = serde_json::from_slice::<crate::gradient_ramp::Library>(
            b.strip_prefix(&[239, 187, 191]).unwrap_or(b),
        ) && l.valid()
        {
            return Ok((Kind::Gradient, [0; 2], vec![]));
        }
        return Err(
            "JSON is not a verified own brush, palette, gradient or tool-folder manifest".into(),
        );
    }
    Err("Unsupported local asset type".into())
}
struct Walker<'a> {
    index: Index,
    previous: Option<&'a Index>,
    seen: BTreeSet<PathBuf>,
    cancel: &'a AtomicBool,
    progress: &'a AtomicUsize,
    visited: usize,
    total: usize,
}
impl Walker<'_> {
    fn walk(&mut self, dir: &Path, depth: usize) -> Result<(), String> {
        canceled(self.cancel)?;
        if depth > 32 {
            note(&mut self.index.notes, "Subfolder depth exceeds32");
            return Ok(());
        }
        let list = match fs::read_dir(dir) {
            Ok(v) => v,
            Err(e) => {
                note(
                    &mut self.index.notes,
                    format!("Unavailable folder {}: {e}", dir.display()),
                );
                return Ok(());
            }
        };
        for result in list {
            canceled(self.cancel)?;
            self.visited += 1;
            self.progress.store(self.visited, Ordering::Relaxed);
            if self.visited > 32768 {
                return Err(
                    "Asset traversal exceeds32768 entries; choose a narrower folder".into(),
                );
            }
            let entry = match result {
                Ok(v) => v,
                Err(e) => {
                    note(
                        &mut self.index.notes,
                        format!("Unreadable folder entry: {e}"),
                    );
                    continue;
                }
            };
            let path = entry.path();
            let relative = path
                .strip_prefix(&self.index.root)
                .map_err(|_| "Folder traversal escaped root")?
                .to_owned();
            if path.file_name().is_some_and(|n| n == ".drawer-trash") {
                continue;
            }
            if relative.to_str().is_none() {
                note(&mut self.index.notes, "Non-Unicode asset path skipped");
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(e) => {
                    note(
                        &mut self.index.notes,
                        format!("{}: {e}", relative.display()),
                    );
                    continue;
                }
            };
            if linked(&metadata) {
                note(
                    &mut self.index.notes,
                    format!("Link/reparse point skipped: {}", relative.display()),
                );
                continue;
            }
            if metadata.is_dir() {
                if self.index.recursive {
                    self.walk(&path, depth + 1)?;
                }
                continue;
            }
            if !metadata.is_file() || self.index.ignored.contains(&relative) {
                continue;
            }
            let ext = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if ![
                "png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp", "json", "gpl", "csv",
            ]
            .contains(&ext.as_str())
            {
                continue;
            }
            if self.index.entries.len() == MAX_ENTRIES {
                return Err("Catalog exceeds4096 assets; choose a narrower folder".into());
            }
            self.seen.insert(relative.clone());
            let previous = self
                .previous
                .and_then(|p| p.entries.iter().find(|e| e.path == relative));
            let default = Metadata {
                name: clean(&path.file_stem().unwrap_or_default().to_string_lossy(), 64),
                folder: relative
                    .parent()
                    .map_or_else(String::new, |p| p.to_string_lossy().replace('\\', "/")),
                ..Default::default()
            };
            let mut asset = Entry {
                path: relative.clone(),
                sha256: String::new(),
                kind: if ext == "json" {
                    Kind::Unknown
                } else if ["gpl", "csv"].contains(&ext.as_str()) {
                    Kind::Palette
                } else {
                    Kind::Image
                },
                metadata: previous.map_or(default, |e| e.metadata.clone()),
                dimensions: [0; 2],
                thumbnail: vec![],
                reason: None,
                duplicate_of: None,
            };
            let loaded = if metadata.len() > FILE_LIMIT as u64 {
                Err("Encoded asset exceeds32MiB".into())
            } else if self.total.saturating_add(metadata.len() as usize) > 512 * 1024 * 1024 {
                Err("Scan byte budget exceeds512MiB; choose a narrower folder".into())
            } else {
                self.total += metadata.len() as usize;
                bytes(&path, FILE_LIMIT)
            };
            match loaded {
                Ok(b) => {
                    asset.sha256 = hash(&b);
                    if let Some(old) = previous
                        && old.sha256 == asset.sha256
                        && old.available()
                        && old.kind != Kind::BrushBundle
                    {
                        asset.kind = old.kind;
                        asset.dimensions = old.dimensions;
                        asset.thumbnail = old.thumbnail.clone();
                    } else {
                        match inspect(&path, &b, self.cancel) {
                            Ok((kind, size, thumb)) => {
                                asset.kind = kind;
                                asset.dimensions = size;
                                asset.thumbnail = thumb;
                            }
                            Err(e) => asset.reason = Some(e.chars().take(1000).collect()),
                        }
                    }
                }
                Err(e) => asset.reason = Some(e.chars().take(1000).collect()),
            }
            canceled(self.cancel)?;
            self.index.entries.push(asset);
        }
        Ok(())
    }
}
pub fn scan(
    root: &Path,
    recursive: bool,
    previous: Option<&Index>,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
) -> Result<Index, String> {
    canceled(cancel)?;
    let absolute = if root.is_absolute() {
        root.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(root)
    };
    if !crate::font_library::local(&absolute) {
        return Err("Choose an offline folder on a local disk".into());
    }
    let root = match absolute.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            if let Some(old) = previous
                && old
                    .root
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&absolute.to_string_lossy())
            {
                let mut unavailable = old.clone();
                for entry in &mut unavailable.entries {
                    entry.reason = Some(format!("Root folder unavailable: {e}"));
                }
                unavailable.notes = vec![format!("Root folder unavailable: {e}")];
                return Ok(unavailable);
            }
            return Err(e.to_string());
        }
    };
    if !crate::font_library::local(&root)
        || !root.is_dir()
        || linked(&fs::symlink_metadata(&absolute).map_err(|e| e.to_string())?)
    {
        return Err("Choose a local folder without links/reparse points".into());
    }
    let previous = previous.filter(|p| p.root == root);
    let index = Index {
        version: 1,
        root: root.clone(),
        recursive,
        entries: vec![],
        ignored: previous.map_or_else(BTreeSet::new, |p| p.ignored.clone()),
        notes: vec![],
    };
    let mut walker = Walker {
        index,
        previous,
        seen: BTreeSet::new(),
        cancel,
        progress,
        visited: 0,
        total: 0,
    };
    walker.walk(&root, 0)?;
    if let Some(old) = previous {
        for entry in &old.entries {
            if walker.seen.contains(&entry.path) || walker.index.ignored.contains(&entry.path) {
                continue;
            }
            if walker.index.entries.len() == MAX_ENTRIES {
                return Err("Catalog including missing entries exceeds4096".into());
            }
            let mut entry = entry.clone();
            entry.reason=Some("Not seen: file moved/deleted, subfolder excluded, or folder permission unavailable. Refresh after repairing the source.".into());
            entry.duplicate_of = None;
            walker.index.entries.push(entry);
        }
    }
    walker.index.entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hashes = std::collections::BTreeMap::<String, PathBuf>::new();
    for entry in &mut walker.index.entries {
        if entry.available() {
            if let Some(first) = hashes.get(&entry.sha256) {
                entry.duplicate_of = Some(first.clone());
            } else {
                hashes.insert(entry.sha256.clone(), entry.path.clone());
            }
        }
    }
    canceled(cancel)?;
    if !walker.index.valid() {
        return Err(
            "Asset names/paths/metadata or index memory limit exceeded; prior catalog unchanged"
                .into(),
        );
    }
    Ok(walker.index)
}

/// Re-index one explicit registration/restoration without walking the whole folder.
pub fn refresh_one(index: &Index, path: &Path, cancel: &AtomicBool) -> Result<Index, String> {
    canceled(cancel)?;
    let absolute = checked(&index.root, path, true)?;
    let b = bytes(&absolute, FILE_LIMIT)?;
    let (kind, dimensions, thumbnail) = inspect(&absolute, &b, cancel)?;
    let metadata = index
        .entries
        .iter()
        .find(|e| e.path == path)
        .map(|e| e.metadata.clone())
        .unwrap_or_else(|| Metadata {
            name: clean(&path.file_stem().unwrap_or_default().to_string_lossy(), 64),
            folder: path
                .parent()
                .map_or_else(String::new, |p| p.to_string_lossy().replace('\\', "/")),
            ..Default::default()
        });
    let sha256 = hash(&b);
    let duplicate_of = index
        .entries
        .iter()
        .find(|e| e.path != path && e.available() && e.sha256 == sha256)
        .map(|e| e.path.clone());
    let mut next = index.clone();
    next.ignored.remove(path);
    next.entries.retain(|e| e.path != path);
    next.entries.push(Entry {
        path: path.into(),
        sha256,
        kind,
        metadata,
        dimensions,
        thumbnail,
        reason: None,
        duplicate_of,
    });
    next.entries.sort_by(|a, b| a.path.cmp(&b.path));
    if !next.valid() {
        return Err("Registered/restored asset exceeds index limits".into());
    }
    canceled(cancel)?;
    Ok(next)
}
