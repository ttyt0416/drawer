//! Local work manifests and atomic, linked history for a text replacement across pages.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::PageObject,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    version: u32,
    pub title: String,
    pub pages: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_ids: Vec<u64>,
    #[serde(default)]
    pub next_page_id: u64,
    #[serde(default)]
    pub right_binding: bool,
    #[serde(default = "first_right")]
    pub first_right: bool,
}
fn first_right() -> bool {
    true
}
fn local(p: &Path) -> bool {
    crate::font_library::local(p)
        && p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("drawer"))
}
impl Work {
    pub fn new(title: String, pages: Vec<PathBuf>) -> Result<Self, String> {
        let work = Self {
            version: 1,
            title,
            page_ids: (1..=pages.len() as u64).collect(),
            next_page_id: pages.len() as u64 + 1,
            right_binding: false,
            first_right: true,
            pages,
        };
        if !work.valid() {
            return Err("A local work needs a name and 1–512 distinct saved .drawer pages".into());
        }
        Ok(work)
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && !self.title.trim().is_empty()
            && self.title.chars().count() <= 64
            && !self.title.chars().any(char::is_control)
            && !self.pages.is_empty()
            && self.pages.len() <= 512
            && (self.page_ids.is_empty() && self.next_page_id == 0
                || self.page_ids.len() == self.pages.len()
                    && self
                        .page_ids
                        .iter()
                        .all(|id| *id > 0 && *id < self.next_page_id)
                    && self
                        .page_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == self.pages.len())
            && self.pages.iter().enumerate().all(|(i, p)| {
                local(p)
                    && p.as_os_str().len() <= 4096
                    && !self.pages[..i].iter().any(|q| {
                        q.to_string_lossy().to_lowercase() == p.to_string_lossy().to_lowercase()
                    })
            })
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if !crate::font_library::local(&path)
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("drawer-work"))
        {
            return Err("Choose a local .drawer-work manifest".into());
        }
        let mut bytes = vec![];
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Work manifest limit4MiB".into());
        }
        let work: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !work.valid() {
            return Err("Invalid local work manifest".into());
        }
        Ok(work)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid work".into());
        }
        let parent = path
            .parent()
            .ok_or("Choose a local work destination")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent)
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("drawer-work"))
        {
            return Err("Save as a local .drawer-work manifest".into());
        }
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Work manifest limit4MiB".into());
        }
        let temp = path.with_extension("drawer-work-write.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| -> io::Result<()> {
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result.map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub find: String,
    pub replace: String,
    pub case_sensitive: bool,
}
#[derive(Clone, Debug)]
pub struct Match {
    pub page: String,
    pub object: u64,
    pub first: usize,
    pub last: usize,
    pub before: String,
    pub after: String,
}
pub struct Page {
    pub id: u64,
    pub revision: u64,
    pub frame: u32,
    pub token: u64,
    pub after: Document,
}
pub struct Prepared {
    pub query: Query,
    pub pages: Vec<Page>,
    pub matches: Vec<Match>,
    pub transaction: Transaction,
}
#[derive(Clone)]
pub struct Transaction {
    pub id: u64,
    members: Vec<(u64, u64, u64)>,
}
static NEXT: AtomicU64 = AtomicU64::new(1);
pub fn prepare(documents: &[&Document], query: &Query) -> Result<Prepared, String> {
    if query.find.is_empty()
        || query.find.chars().count() > 256
        || query.find.len() > 2048
        || query.replace.chars().count() > 4096
        || query.replace.len() > 65536
    {
        return Err(
            "Find must contain 1–256 characters; replacement limit4096 characters/64KiB".into(),
        );
    }
    if documents
        .iter()
        .enumerate()
        .any(|(i, d)| documents[..i].iter().any(|p| p.id == d.id))
    {
        return Err("Duplicate work source page".into());
    }
    if documents.is_empty()
        || documents.len() > 512
        || documents.iter().map(|d| d.resident_bytes()).sum::<usize>() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Work page/snapshot budget exceeded".into());
    }
    let regex = regex::RegexBuilder::new(&regex::escape(&query.find))
        .case_insensitive(!query.case_sensitive)
        .build()
        .map_err(|e| e.to_string())?;
    let group = next_group();
    let mut pages = vec![];
    let mut matches = vec![];
    let mut preview_bytes = 0usize;
    let mut members = vec![];
    for d in documents {
        let mut state = d.snapshot();
        let mut changed = false;
        for (n, object) in state.scene.objects.iter_mut().enumerate() {
            let PageObject::Text { text, layout, .. } = object else {
                continue;
            };
            let found: Vec<_> = regex
                .find_iter(text)
                .map(|m| (m.start(), m.end()))
                .collect();
            if found.is_empty() {
                continue;
            }
            if found.len().saturating_add(matches.len()) > 10000 {
                return Err("Work match limit10000".into());
            }
            let original = text.clone();
            let mut style = layout
                .as_deref()
                .cloned()
                .unwrap_or_else(crate::text_layout::Style::legacy);
            for (a, b) in found.iter().copied() {
                let first = original[..a].chars().count();
                let last = original[..b].chars().count();
                if !crate::text_format::boundary(&original, first)
                    || !crate::text_format::boundary(&original, last)
                {
                    return Err(
                        "A match splits a grapheme; include its complete combining/emoji sequence"
                            .into(),
                    );
                }
                let before: String = original[a..b].chars().take(160).collect();
                let after: String = query.replace.chars().take(160).collect();
                preview_bytes = preview_bytes.saturating_add(before.len() + after.len());
                if preview_bytes > 4 * 1024 * 1024 {
                    return Err("Work match preview limit4MiB".into());
                }
                matches.push(Match {
                    page: d.title.clone(),
                    object: d.scene.catalog.objects.id(n),
                    first,
                    last,
                    before,
                    after,
                });
            }
            for (a, b) in found.into_iter().rev() {
                let old = text.clone();
                text.replace_range(a..b, &query.replace);
                if text.chars().count() > 4096 || text.len() > 65536 {
                    return Err("Replacement exceeds a text object's limit".into());
                }
                style.remap_text(&old, text);
            }
            if *text != original {
                *layout = Some(Box::new(style));
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        crate::text_layout::validate_scene(&state.scene)?;
        let mut after = (*d).clone();
        after.commit_state_group(state, Some(group))?;
        members.push((d.id, d.history_token(), after.history_token()));
        pages.push(Page {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            token: d.history_token(),
            after,
        });
    }
    Ok(Prepared {
        query: query.clone(),
        pages,
        matches,
        transaction: Transaction { id: group, members },
    })
}
impl Prepared {
    pub fn validate(&self, documents: &[Document]) -> Result<(), String> {
        for p in &self.pages {
            let d = documents
                .iter()
                .find(|d| d.id == p.id)
                .ok_or("A source page is closed")?;
            if d.revision != p.revision
                || d.current_frame != p.frame
                || d.history_token() != p.token
            {
                return Err("A source page changed; prepare the work preview again".into());
            }
        }
        Ok(())
    }
    pub fn apply(self, documents: &mut [Document]) -> Result<Transaction, String> {
        self.validate(documents)?;
        let mut staged = vec![];
        for p in self.pages {
            let d = documents.iter().find(|d| d.id == p.id).unwrap();
            let mut copy = d.clone();
            copy.commit_state_group(p.after.snapshot(), Some(self.transaction.id))?;
            staged.push(copy);
        }
        for copy in staged {
            let d = documents.iter_mut().find(|d| d.id == copy.id).unwrap();
            *d = copy;
        }
        Ok(self.transaction)
    }
}
impl Transaction {
    pub(crate) fn new(id: u64, members: Vec<(u64, u64, u64)>) -> Self {
        Self { id, members }
    }
    pub fn contains(&self, id: u64) -> bool {
        self.members.iter().any(|m| m.0 == id)
    }
    pub fn step(&self, documents: &mut [Document], redo: bool) -> Result<(), String> {
        for (id, before, after) in &self.members {
            let d = documents
                .iter()
                .find(|d| d.id == *id)
                .ok_or("A work page is closed; linked history cannot be partly applied")?;
            let expected = if redo { *before } else { *after };
            if d.history_token() != expected || d.history_group(redo) != Some(self.id) {
                return Err(
                    "Undo later changes on every affected page first; work history stays atomic"
                        .into(),
                );
            }
        }
        for (id, _, _) in &self.members {
            let d = documents.iter_mut().find(|d| d.id == *id).unwrap();
            if redo {
                d.redo();
            } else {
                d.undo();
            }
        }
        Ok(())
    }
}

pub(crate) fn next_group() -> u64 {
    0xf000_0000_0000_0000 | NEXT.fetch_add(1, Ordering::Relaxed)
}
