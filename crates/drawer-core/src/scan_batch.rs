//! Device-independent continuous-scan state. Device access stays in the app broker.
use crate::{book_io, document::Document, page_resources, references::LocalImage};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
pub struct Image {
    pub image: LocalImage,
    pub original: Arc<Vec<u8>>,
    pub filename: String,
}
#[derive(Clone)]
pub struct Accepted {
    pub slot: usize,
    pub document: Document,
    pub original: Arc<Vec<u8>>,
    pub filename: String,
}
#[derive(Clone)]
pub struct Session {
    pub count: usize,
    pub slot: usize,
    pub error: Option<String>,
    pub accepted: Vec<Accepted>,
    pub skipped: Vec<usize>,
    pub cancelled: bool,
    dpi: f64,
    budget: usize,
}
impl Session {
    pub fn new(count: usize, dpi: f64, budget: usize) -> Result<Self, String> {
        if !(1..=512).contains(&count) || !(1. ..=4800.).contains(&dpi) || budget < 4096 {
            return Err("Scan needs1..512 pages, valid DPI and available memory".into());
        }
        Ok(Self {
            count,
            slot: 0,
            error: None,
            accepted: vec![],
            skipped: vec![],
            cancelled: false,
            dpi,
            budget,
        })
    }
    pub fn done(&self) -> bool {
        !self.cancelled && self.slot == self.count && self.error.is_none()
    }
    pub fn fail(&mut self, message: String) {
        if !self.cancelled && self.slot < self.count {
            self.error = Some(message.chars().take(8000).collect());
        }
    }
    pub fn retry(&mut self) -> Result<(), String> {
        if self.error.take().is_none() || self.cancelled {
            return Err("No failed scan to retry".into());
        }
        Ok(())
    }
    pub fn skip(&mut self) -> Result<(), String> {
        self.retry()?;
        self.skipped.push(self.slot);
        self.slot += 1;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.error = None;
        self.accepted.clear();
    }
    pub fn used(&self) -> usize {
        self.accepted
            .iter()
            .map(|p| p.document.resident_bytes().saturating_add(p.original.len()))
            .sum()
    }
    pub fn remaining(&self) -> usize {
        self.budget.saturating_sub(self.used())
    }
    pub fn accept(&mut self, image: Image) -> Result<(), String> {
        if self.cancelled || self.done() || self.error.is_some() {
            return Err("Scan batch is not waiting for an image".into());
        }
        if !image.image.valid()
            || image.original.is_empty()
            || image.original.len() > 64 * 1024 * 1024
            || image.filename.len() > 1024
            || image.filename.contains('\0')
        {
            return Err("Invalid scan image or original".into());
        }
        let mut document = image
            .image
            .document(self.remaining().saturating_sub(image.original.len()))
            .map_err(|e| e.to_string())?;
        document.dpi = self.dpi;
        document.title = format!("Scan {:04}", self.slot + 1);
        if document
            .resident_bytes()
            .saturating_add(image.original.len())
            > self.remaining()
        {
            return Err("Scan batch memory budget exceeded".into());
        }
        self.accepted.push(Accepted {
            slot: self.slot,
            document,
            original: image.original,
            filename: image.filename,
        });
        self.slot += 1;
        Ok(())
    }
    pub fn bind(&mut self, work: &crate::story_text::Work) -> Result<(), String> {
        if !work.valid() || work.pages.len() + self.accepted.len() > 512 {
            return Err("Invalid project / scan page count".into());
        }
        let total = (work.pages.len() + self.accepted.len()) as u32;
        for (n, a) in self.accepted.iter_mut().enumerate() {
            let p = &mut a.document.scene.comics.page;
            p.right_binding = work.right_binding;
            p.first_right = work.first_right;
            p.number = (work.pages.len() + n + 1) as u32;
            p.total = total;
        }
        Ok(())
    }
    pub fn prepare(
        &self,
        directory: &Path,
        cancel: &AtomicBool,
    ) -> Result<page_resources::Prepared, String> {
        if !self.done() || self.accepted.is_empty() {
            return Err("Finish/review the scan batch before adding pages".into());
        }
        let pages = self
            .accepted
            .iter()
            .map(|a| a.document.snapshot())
            .map(|s| crate::layer_merge::preview(&s, 0))
            .collect::<Result<Vec<_>, _>>()?;
        let mut owned = page_resources::prepare(&pages, directory, cancel)?;
        let mut records = vec![];
        for image in &self.accepted {
            book_io::check_cancel(cancel)?;
            let ext = Path::new(&image.filename)
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| s.len() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()))
                .unwrap_or("bin");
            let path = owned.blob(
                &format!("scan-{:04}.{ext}", image.slot + 1),
                &image.original,
            )?;
            records.push(serde_json::json!({"slot":image.slot+1,"original_name":image.filename,"original":path.file_name(),"dpi":self.dpi}));
        }
        owned.blob("scans.json",&serde_json::to_vec_pretty(&serde_json::json!({"version":1,"requested":self.count,"skipped":self.skipped.iter().map(|s|s+1).collect::<Vec<_>>(),"images":records})).map_err(|e|e.to_string())?)?;
        book_io::check_cancel(cancel)?;
        Ok(owned)
    }
}
