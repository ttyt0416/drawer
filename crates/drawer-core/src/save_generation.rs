//! One immutable page generation. The project manifest is the sole commit point.
use crate::{document::Document, story_text::Work};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: u64,
    pub bytes: u64,
    pub sha256: String,
    pub native_schema: u32,
    pub revision: u64,
    pub frame: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub generation: String,
    pub pages: Vec<Page>,
}
impl Manifest {
    pub fn valid(&self, ids: &[u64]) -> bool {
        self.schema == 1
            && !self.generation.is_empty()
            && self.generation.len() <= 64
            && self
                .generation
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && self.pages.len() == ids.len()
            && self.pages.iter().zip(ids).all(|(p, id)| {
                p.id == *id
                    && p.bytes > 0
                    && p.bytes <= 2 * 1024 * 1024 * 1024
                    && crate::asset_catalog::valid_hash(&p.sha256)
                    && (1..=crate::files::VERSION).contains(&p.native_schema)
            })
    }
    pub fn verify(&self, work: &Work) -> Result<(), String> {
        if !self.valid(&work.page_ids) {
            return Err("Invalid project generation schema/page identities".into());
        }
        let parent = work.pages[0].parent().ok_or("Generation folder missing")?;
        if work.pages.iter().any(|p| p.parent() != Some(parent)) {
            return Err("Mixed project generation folders are refused".into());
        }
        for (record, path) in self.pages.iter().zip(&work.pages) {
            self.verify_page(record.id, path)?;
        }
        Ok(())
    }
    pub fn verify_page(&self, id: u64, path: &Path) -> Result<(), String> {
        let expected = self
            .pages
            .iter()
            .find(|p| p.id == id)
            .ok_or("Page is not in the saved generation")?;
        let actual = inspect(id, path)?;
        if *expected != actual {
            return Err(format!(
                "Page {id}: saved generation hash/size/schema/revision mismatch"
            ));
        }
        Ok(())
    }
}
pub fn inspect(id: u64, path: &Path) -> Result<Page, String> {
    let path = crate::file_browser::checked(path)?;
    let m = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() > 2 * 1024 * 1024 * 1024 {
        return Err("Native generation block size/type limit".into());
    }
    let mut f = fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut head = [0; 12];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    if &head[..8] != b"DRAWER\r\n" {
        return Err("Native generation signature mismatch".into());
    }
    let native_schema = u32::from_le_bytes(head[8..].try_into().unwrap());
    if !(1..=crate::files::VERSION).contains(&native_schema) {
        return Err("Unknown native generation schema".into());
    }
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(head);
    let mut tail = Vec::new();
    let mut buf = [0; 65536];
    let mut count = 12u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or("Generation size overflow")?;
        if count > 2 * 1024 * 1024 * 1024 {
            return Err("Generation size limit".into());
        }
        hash.update(&buf[..n]);
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > 64 {
            tail.drain(..tail.len() - 64);
        }
    }
    if count != m.len() {
        return Err("Generation block changed while hashing".into());
    }
    let revision = if native_schema >= 124 {
        if tail.len() != 64 || &tail[..8] != b"DWEND1\r\n" {
            return Err("Missing generation revision footer".into());
        }
        u64::from_le_bytes(tail[16..24].try_into().unwrap())
    } else {
        0
    };
    let frame = if native_schema >= 124 {
        u32::from_le_bytes(tail[24..28].try_into().unwrap())
    } else {
        0
    };
    Ok(Page {
        id,
        bytes: count,
        sha256: format!("{:x}", hash.finalize()),
        native_schema,
        revision,
        frame,
    })
}
/// Freeze every open page at the same call boundary; a writer never re-reads a live document.
pub fn capture(opened: &[(u64, &Document)]) -> Result<Vec<(u64, Document)>, String> {
    let sum = opened
        .iter()
        .try_fold(0usize, |n, (_, d)| n.checked_add(d.resident_bytes()))
        .ok_or("Snapshot memory overflow")?;
    if sum > crate::document::DEFAULT_DOCUMENT_BUDGET / 4 {
        return Err("Project snapshot capture memory limit".into());
    }
    Ok(opened
        .iter()
        .map(|(id, d)| (*id, d.current_copy()))
        .collect())
}
#[path = "save_generation_verify.rs"]
pub mod verify;
