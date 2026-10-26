//! Stable page order, lazy native-page loading and manifest-last project snapshots.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    story_text::Work,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug)]
pub struct Model {
    pub work: Work,
    pub selected: u64,
    past: Vec<Work>,
    future: Vec<Work>,
    high_water: u64,
    pub loads: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    First,
    Previous,
    Next,
    Last,
    Number(usize),
}
#[derive(Clone, Debug)]
pub enum Edit {
    Replace {
        id: u64,
        path: PathBuf,
    },
    Batch {
        paths: Vec<PathBuf>,
        remove: Vec<u64>,
        index: usize,
    },
    Add {
        path: PathBuf,
        index: usize,
    },
    Delete(u64),
    Move {
        id: u64,
        index: usize,
    },
    Metadata {
        title: String,
        right_binding: bool,
        first_right: bool,
    },
}
fn canonical(path: &Path) -> Result<PathBuf, String> {
    let p = path.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&p)
        || !p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("drawer"))
        || !p.is_file()
    {
        return Err("Choose a local .drawer page".into());
    }
    Ok(p)
}
impl Model {
    pub fn new(mut work: Work) -> Result<Self, String> {
        if !work.valid() {
            return Err("Invalid page work".into());
        }
        if work.page_ids.is_empty() {
            work.page_ids = (1..=work.pages.len() as u64).collect();
            work.next_page_id = work.pages.len() as u64 + 1;
        }
        Ok(Self {
            selected: work.page_ids[0],
            high_water: work.next_page_id,
            work,
            past: vec![],
            future: vec![],
            loads: 0,
        })
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::new(Work::load(path)?)
    }
    pub fn index(&self, id: u64) -> Result<usize, String> {
        self.work
            .page_ids
            .iter()
            .position(|v| *v == id)
            .ok_or_else(|| "Page was removed".into())
    }
    pub fn current(&self) -> usize {
        self.index(self.selected).unwrap_or(0)
    }
    pub fn navigate(&mut self, n: Navigation) -> Result<u64, String> {
        let i = match n {
            Navigation::First => 0,
            Navigation::Previous => self.current().saturating_sub(1),
            Navigation::Next => (self.current() + 1).min(self.work.pages.len() - 1),
            Navigation::Last => self.work.pages.len() - 1,
            Navigation::Number(n) => n
                .checked_sub(1)
                .filter(|v| *v < self.work.pages.len())
                .ok_or("Page number outside the work")?,
        };
        self.selected = self.work.page_ids[i];
        Ok(self.selected)
    }
    pub fn facing(&self, index: usize) -> Option<usize> {
        if index >= self.work.pages.len() {
            return None;
        }
        let right = self.work.first_right == (index.is_multiple_of(2));
        let other = if right != self.work.right_binding {
            index.checked_sub(1)?
        } else {
            index.checked_add(1)?
        };
        (other < self.work.pages.len()).then_some(other)
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.past.len(), self.future.len())
    }
    pub fn edit(&mut self, e: Edit) -> Result<(), String> {
        let mut work = self.work.clone();
        match e {
            Edit::Replace { id, path } => {
                let index = self.index(id)?;
                let path = canonical(&path)?;
                if path == work.pages[index] {
                    return Ok(());
                }
                if work.pages.iter().any(|p| {
                    p.to_string_lossy()
                        .eq_ignore_ascii_case(&path.to_string_lossy())
                }) {
                    return Err("This page is already in the work".into());
                }
                let next = work.next_page_id.max(self.high_water);
                work.next_page_id = next.checked_add(1).ok_or("Page ID exhausted")?;
                work.pages[index] = path;
                work.page_ids[index] = next;
            }
            Edit::Batch {
                paths,
                remove,
                index,
            } => {
                if paths.is_empty()
                    || paths.len() > 512
                    || remove.len() > 512
                    || remove
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != remove.len()
                    || remove.iter().any(|id| !work.page_ids.contains(id))
                {
                    return Err("Choose a nonempty page batch and existing removal IDs".into());
                }
                for n in (0..work.pages.len()).rev() {
                    if remove.contains(&work.page_ids[n]) {
                        work.pages.remove(n);
                        work.page_ids.remove(n);
                    }
                }
                if index > work.pages.len() || work.pages.len().saturating_add(paths.len()) > 512 {
                    return Err("Batch insertion index/count limit512".into());
                }
                let mut prepared = vec![];
                for path in paths {
                    let path = canonical(&path)?;
                    if work.pages.iter().chain(prepared.iter()).any(|p| {
                        p.to_string_lossy()
                            .eq_ignore_ascii_case(&path.to_string_lossy())
                    }) {
                        return Err("A batch page is duplicated or already in the work".into());
                    }
                    prepared.push(path);
                }
                for (offset, path) in prepared.into_iter().enumerate() {
                    let next = work.next_page_id.max(self.high_water);
                    work.next_page_id = next.checked_add(1).ok_or("Page ID exhausted")?;
                    work.pages.insert(index + offset, path);
                    work.page_ids.insert(index + offset, next);
                }
            }
            Edit::Add { path, index } => {
                if index > work.pages.len() || work.pages.len() >= 512 {
                    return Err("Page insertion index/count limit512".into());
                }
                let path = canonical(&path)?;
                if work.pages.iter().any(|p| {
                    p.to_string_lossy()
                        .eq_ignore_ascii_case(&path.to_string_lossy())
                }) {
                    return Err("This page is already in the work".into());
                }
                let id = work.next_page_id.max(self.high_water);
                work.next_page_id = id.checked_add(1).ok_or("Page ID exhausted")?;
                work.pages.insert(index, path);
                work.page_ids.insert(index, id);
            }
            Edit::Delete(id) => {
                if work.pages.len() == 1 {
                    return Err("Keep at least one page; an empty drawing page is allowed".into());
                }
                let i = self.index(id)?;
                work.pages.remove(i);
                work.page_ids.remove(i);
            }
            Edit::Move { id, index } => {
                if index >= work.pages.len() {
                    return Err("Page order index outside the work".into());
                }
                let i = self.index(id)?;
                let path = work.pages.remove(i);
                let id = work.page_ids.remove(i);
                work.pages.insert(index, path);
                work.page_ids.insert(index, id);
            }
            Edit::Metadata {
                title,
                right_binding,
                first_right,
            } => {
                work.title = title;
                work.right_binding = right_binding;
                work.first_right = first_right;
            }
        }
        if !work.valid() {
            return Err("Invalid page operation".into());
        }
        if work == self.work {
            return Ok(());
        }
        self.high_water = self.high_water.max(work.next_page_id);
        work.next_page_id = self.high_water;
        let old_index = self.current();
        self.past.push(std::mem::replace(&mut self.work, work));
        self.work.next_page_id = self.work.next_page_id.max(self.high_water);
        while self.past.len() > 128
            || self
                .past
                .iter()
                .map(|w| {
                    w.pages
                        .iter()
                        .map(|p| p.as_os_str().len() * 2 + 128)
                        .sum::<usize>()
                })
                .sum::<usize>()
                > 8 * 1024 * 1024
        {
            self.past.remove(0);
        }
        self.future.clear();
        if !self.work.page_ids.contains(&self.selected) {
            self.selected = self.work.page_ids[old_index.min(self.work.pages.len() - 1)];
        }
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        let Some(work) = self.past.pop() else {
            return false;
        };
        self.future.push(std::mem::replace(&mut self.work, work));
        self.work.next_page_id = self.work.next_page_id.max(self.high_water);
        if !self.work.page_ids.contains(&self.selected) {
            self.selected = self.work.page_ids[0];
        }
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(work) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(&mut self.work, work));
        self.work.next_page_id = self.work.next_page_id.max(self.high_water);
        if !self.work.page_ids.contains(&self.selected) {
            self.selected = self.work.page_ids[0];
        }
        true
    }
    pub fn open_page(&mut self, id: u64, budget: usize) -> Result<Document, String> {
        let i = self.index(id)?;
        let path = canonical(&self.work.pages[i])?;
        let doc = crate::files::open(&path, budget).map_err(|e| e.to_string())?;
        self.loads += 1;
        Ok(doc)
    }
}
pub fn duplicate(source: &Document, destination: &Path) -> Result<(), String> {
    if destination.try_exists().map_err(|e| e.to_string())? {
        return Err("Choose a new filename for the duplicated page".into());
    }
    if source.resident_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Page copy memory limit".into());
    }
    let mut copy = source.clone();
    crate::files::save(&mut copy, destination).map_err(|e| e.to_string())
}
fn digest(path: &Path) -> Result<[u8; 32], String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    if !f.metadata().map_err(|e| e.to_string())?.is_file()
        || f.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 * 1024
    {
        return Err("Project page file limit1GiB".into());
    }
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(h.finalize().into())
}
struct OwnedDirectory {
    path: PathBuf,
    parent: PathBuf,
    keep: bool,
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        if !self.keep
            && self.path.is_absolute()
            && self.path.parent() == Some(self.parent.as_path())
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
/// The original manifest and all source pages stay intact until every page is ready.
/// Open documents are matched by page ID so that current unsaved edits enter the snapshot.
pub fn snapshot(
    model: &Model,
    destination: &Path,
    opened: &[(u64, &Document)],
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Work, String> {
    if !model.work.valid()
        || opened.iter().enumerate().any(|(n, (id, _))| {
            model.index(*id).is_err() || opened[..n].iter().any(|(v, _)| v == id)
        })
    {
        return Err("Invalid project/open page IDs".into());
    }
    let parent = destination
        .parent()
        .ok_or("Choose a project directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !crate::font_library::local(&parent)
        || !destination
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("drawer-work"))
    {
        return Err("Save a local .drawer-work project".into());
    }
    if opened
        .iter()
        .map(|(_, d)| d.resident_bytes())
        .sum::<usize>()
        > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Project snapshot open-document budget".into());
    }
    let mut stamps = vec![];
    let mut bytes = 0u64;
    for (id, p) in model.work.page_ids.iter().zip(&model.work.pages) {
        if let Some((_, d)) = opened.iter().find(|(n, _)| n == id) {
            bytes = bytes.saturating_add(d.resident_bytes() as u64 * 4);
        } else {
            let p = canonical(p)?;
            bytes = bytes.saturating_add(fs::metadata(&p).map_err(|e| e.to_string())?.len());
            stamps.push((*id, p.clone(), digest(&p)?));
        }
    }
    if bytes > 2 * 1024 * 1024 * 1024 {
        return Err("Project snapshot resource limit2GiB".into());
    }
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let staging = parent.join(format!(".drawer-project-{nonce}.tmp"));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let mut owned = OwnedDirectory {
        path: staging,
        parent: parent.clone(),
        keep: false,
    };
    let total = model.work.pages.len() + stamps.len() + 1;
    for (n, (id, source)) in model
        .work
        .page_ids
        .iter()
        .zip(&model.work.pages)
        .enumerate()
    {
        tick(n, total)?;
        let out = owned.path.join(format!("page-{id}.drawer"));
        if let Some((_, d)) = opened.iter().find(|(i, _)| i == id) {
            let mut copy = (**d).clone();
            crate::files::save(&mut copy, &out).map_err(|e| e.to_string())?;
        } else {
            let (_, p, hash) = stamps
                .iter()
                .find(|(i, _, _)| i == id)
                .ok_or("Page source missing")?;
            let mut input = fs::File::open(p).map_err(|e| e.to_string())?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            drop(output);
            crate::files::open(&out, DEFAULT_DOCUMENT_BUDGET / 8).map_err(|e| e.to_string())?;
            if digest(&out)? != *hash {
                return Err(format!("Page {} changed while copying", source.display()));
            }
        }
    }
    for (n, (_, path, hash)) in stamps.iter().enumerate() {
        tick(model.work.pages.len() + n, total)?;
        if digest(path)? != *hash {
            return Err("A closed project page changed; no snapshot published".into());
        }
    }
    tick(total - 1, total)?;
    let final_dir = parent.join(format!("Drawer-project-{nonce}"));
    fs::rename(&owned.path, &final_dir).map_err(|e| e.to_string())?;
    owned.path = final_dir;
    let mut work = model.work.clone();
    work.next_page_id = work.next_page_id.max(model.high_water);
    work.pages = work
        .page_ids
        .iter()
        .map(|id| owned.path.join(format!("page-{id}.drawer")))
        .collect();
    work.save(destination)?;
    owned.keep = true;
    Ok(work)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub dock: crate::workspaces::Dock,
    pub width: f32,
    pub size: [f32; 2],
    pub position: [f32; 2],
    pub auto_scroll: bool,
    pub facing: bool,
    pub new_view: bool,
    pub scroll: f32,
    pub project: Option<PathBuf>,
    pub selected: Option<u64>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            dock: crate::workspaces::Dock::Left,
            width: 300.,
            size: [600., 720.],
            position: [25., 40.],
            auto_scroll: true,
            facing: true,
            new_view: false,
            scroll: 0.,
            project: None,
            selected: None,
        }
    }
}
impl Preferences {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        (0. ..=30000.).contains(&self.scroll)
            && self.selected.is_none_or(|id| id > 0)
            && self.project.as_ref().is_none_or(|p| {
                crate::font_library::local(p)
                    && p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("drawer-work"))
            })
            && (180. ..=800.).contains(&self.width)
            && self.size.iter().all(|v| (180. ..=2000.).contains(v))
            && self
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e6)
    }
}
