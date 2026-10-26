//! Drawer storyboard v1: local native sources plus editable frame/dialogue JSON.
use crate::{
    book_io, book_production::Setup, document::Document, page_batch::Stamp, scene::PageObject,
    story_text::Work,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub object: PageObject,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub path: PathBuf,
    pub sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: u64,
    pub source: Reference,
    pub image: Reference,
    pub frames: Vec<Entry>,
    pub dialogues: Vec<Entry>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub format: String,
    pub version: u32,
    pub title: String,
    pub right_binding: bool,
    pub first_right: bool,
    pub setup: Setup,
    pub pages: Vec<Page>,
}
pub struct Prepared {
    pub file: File,
    pub pages: Vec<Document>,
    pub sources: Vec<Stamp>,
}
impl Prepared {
    pub fn fresh(&self) -> Result<(), String> {
        for s in &self.sources {
            if hash(&book_io::read_local(&s.path, 64 * 1024 * 1024)?) != s.hash {
                return Err(format!(
                    "Storyboard reference changed: {}",
                    s.path.display()
                ));
            }
        }
        Ok(())
    }
    pub fn work(&self, paths: Vec<PathBuf>) -> Result<Work, String> {
        if paths.len() != self.file.pages.len() {
            return Err("Storyboard page count mismatch".into());
        }
        let mut w = Work::new(self.file.title.clone(), paths)?;
        w.page_ids = self.file.pages.iter().map(|p| p.id).collect();
        w.next_page_id = w
            .page_ids
            .iter()
            .max()
            .unwrap()
            .checked_add(1)
            .ok_or("Page ID exhausted")?;
        w.right_binding = self.file.right_binding;
        w.first_right = self.file.first_right;
        if !w.valid() {
            return Err("Invalid storyboard project".into());
        }
        Ok(w)
    }
}
fn hash(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}
fn exchange_object(o: &PageObject, frame: bool) -> bool {
    if frame {
        matches!(o,PageObject::Comic(c) if matches!(c.kind,crate::comics::Kind::Frame(_)))
    } else {
        matches!(o, PageObject::Text { .. })
    }
}
fn entries(d: &Document, frame: bool) -> Vec<Entry> {
    d.scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| exchange_object(o, frame))
        .map(|(n, o)| Entry {
            id: d.scene.catalog.objects.id(n),
            object: o.clone(),
        })
        .collect()
}
fn checked_path(base: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().len() > 4096 || path.to_string_lossy().contains('\0') {
        return Err("Invalid storyboard path".into());
    }
    let p = if path.is_absolute() {
        path.to_path_buf()
    } else {
        if path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            return Err("Storyboard relative path escapes its folder".into());
        }
        base.join(path)
    };
    if !crate::font_library::local(&p) {
        return Err("Storyboard references must be local drive files".into());
    }
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&p) {
        return Err("Storyboard reference resolves outside local drive".into());
    }
    Ok(p)
}
pub fn decode(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Prepared, String> {
    if !path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("drawer-storyboard"))
    {
        return Err("Only Drawer storyboard v1 (.drawer-storyboard) is supported; external proprietary storyboards have no verified adapter".into());
    }
    if path.is_absolute() && !crate::font_library::local(path) {
        return Err("Choose a local drive storyboard".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let path = path.as_path();
    let bytes = book_io::read_local(path, LIMIT)?;
    let file: File = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if file.format != "Drawer storyboard"
        || file.version != 1
        || file.pages.is_empty()
        || file.pages.len() > 512
        || !file.setup.valid()
    {
        return Err("Unsupported/invalid storyboard version, setup or page count".into());
    }
    let base = path.parent().ok_or("Missing storyboard folder")?;
    let mut result = Prepared {
        file,
        pages: vec![],
        sources: vec![],
    };
    let mut used = bytes.len();
    let mut ids = std::collections::BTreeSet::new();
    for p in &result.file.pages {
        book_io::check_cancel(cancel)?;
        if p.id == 0 || !ids.insert(p.id) || p.frames.len() + p.dialogues.len() > 4096 {
            return Err("Duplicate/invalid storyboard page IDs or object limit".into());
        }
        let source = checked_path(base, &p.source.path)?;
        if !source
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("drawer"))
        {
            return Err("Storyboard source must be a native page".into());
        }
        if result.sources.iter().any(|s| s.path == source) {
            return Err("Duplicate storyboard native reference".into());
        }
        let data = book_io::read_local(
            &source,
            (64 * 1024 * 1024).min(budget.saturating_sub(used) / 2),
        )?;
        if hash(&data) != p.source.sha256 {
            return Err("Storyboard native source hash mismatch".into());
        }
        let mut d = crate::files::read_native_snapshot(
            &source,
            &data,
            budget.saturating_sub(used + data.len()),
        )
        .map_err(|e| e.to_string())?;
        let image = checked_path(base, &p.image.path)?;
        let image_bytes =
            book_io::read_local(&image, budget.saturating_sub(used + d.resident_bytes()) / 2)?;
        if hash(&image_bytes) != p.image.sha256 {
            return Err("Storyboard image reference hash mismatch".into());
        }
        // The preview is a real bounded local image, never a browser/remote URL.
        let preview = crate::image_import::ImageSource::from_encoded(
            image.clone(),
            image_bytes,
            budget.saturating_sub(used + d.resident_bytes()),
            cancel,
        )
        .map_err(|e| e.to_string())?;
        preview
            .decode(
                0,
                true,
                budget.saturating_sub(used + d.resident_bytes()) / 2,
                cancel,
            )
            .map_err(|e| e.to_string())?;
        let mut seen = std::collections::BTreeSet::new();
        for (list, frame) in [(&p.frames, true), (&p.dialogues, false)] {
            for row in list {
                if !seen.insert(row.id) || !exchange_object(&row.object, frame) {
                    return Err("Duplicate or incorrect storyboard object kind".into());
                }
                let n = d
                    .scene
                    .catalog
                    .objects
                    .index(row.id, d.scene.objects.len())
                    .ok_or("Storyboard refers to a missing object ID")?;
                if !exchange_object(&d.scene.objects[n], frame) {
                    return Err("Storyboard object reference changed type".into());
                }
                d.scene.objects[n] = row.object.clone();
                if let PageObject::Comic(c) = &mut d.scene.objects[n]
                    && let crate::comics::Kind::Frame(f) = &mut c.kind
                {
                    crate::frame_ops::refresh(f)?;
                }
            }
            if d.scene
                .objects
                .iter()
                .filter(|o| exchange_object(o, frame))
                .count()
                != list.len()
            {
                return Err("Storyboard omitted an existing frame/dialogue reference".into());
            }
        }
        if !d.scene.valid() {
            return Err("Storyboard edits produce an invalid scene or reference".into());
        }
        crate::text_layout::validate_scene(&d.scene)?;
        used = used.saturating_add(d.resident_bytes());
        if used > budget {
            return Err("Storyboard pages exceed memory budget".into());
        }
        result.sources.push(Stamp {
            path: source,
            hash: p.source.sha256,
        });
        result.sources.push(Stamp {
            path: image,
            hash: p.image.sha256,
        });
        d.path = None;
        d.source_path = None;
        result.pages.push(d);
    }
    let w = result.work(
        (0..result.pages.len())
            .map(|n| base.join(format!("page-{n}.drawer")))
            .collect(),
    )?;
    result.file.setup.validate(&w)?;
    book_io::check_cancel(cancel)?;
    Ok(result)
}
#[allow(clippy::too_many_arguments)]
pub fn export(
    path: &Path,
    work: &Work,
    pages: &[Document],
    setup: &Setup,
    budget: usize,
    cancel: &AtomicBool,
    writer: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    setup.validate(work)?;
    if pages.len() != work.pages.len()
        || pages.iter().map(Document::resident_bytes).sum::<usize>() > budget
        || !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("drawer-storyboard"))
    {
        return Err("Choose a new .drawer-storyboard for all prepared pages within budget".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose a local storyboard folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut owned = crate::page_resources::prepare(pages, &parent, cancel)?;
    let mut file = File {
        format: "Drawer storyboard".into(),
        version: 1,
        title: work.title.clone(),
        right_binding: work.right_binding,
        first_right: work.first_right,
        setup: setup.clone(),
        pages: vec![],
    };
    for (n, d) in pages.iter().enumerate() {
        book_io::check_cancel(cancel)?;
        let native = book_io::read_local(&owned.paths[n], 64 * 1024 * 1024)?;
        let r = crate::render::Renderer::new(d);
        if let Some(e) = r.error() {
            return Err(e.into());
        }
        let scale = (d.width.max(d.height) as f64 / 256.).max(1.);
        let w = (d.width as f64 / scale).ceil() as u32;
        let h = (d.height as f64 / scale).ceil() as u32;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            book_io::check_cancel(cancel)?;
            for x in 0..w {
                pixels.extend(r.pixel([
                    (x as f64 + 0.5) * d.width as f64 / w as f64,
                    (y as f64 + 0.5) * d.height as f64 / h as f64,
                ]));
            }
        }
        use image::ImageEncoder;
        let mut png = vec![];
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        let image = owned.blob(&format!("page-{:04}.png", n + 1), &png)?;
        let relative = |p: &Path| {
            p.strip_prefix(&parent)
                .map(Path::to_path_buf)
                .map_err(|e| e.to_string())
        };
        file.pages.push(Page {
            id: work.page_ids[n],
            source: Reference {
                path: relative(&owned.paths[n])?,
                sha256: hash(&native),
            },
            image: Reference {
                path: relative(&image)?,
                sha256: hash(&png),
            },
            frames: entries(d, true),
            dialogues: entries(d, false),
        });
    }
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("Storyboard JSON limit4MiB".into());
    }
    book_io::publish(path, &bytes, cancel, writer)?;
    owned.keep();
    Ok(())
}
