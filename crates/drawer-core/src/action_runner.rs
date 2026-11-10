//! Resumable private action execution and new-file-only local batches.
use crate::{
    auto_actions::{Action, prepare_operation},
    document::Document,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Errors {
    #[default]
    Stop,
    Skip,
}
pub struct Session {
    pub action: Action,
    pub cursor: usize,
    pub issues: Vec<String>,
    source: (u64, u64, u32),
    work: Document,
    available: usize,
}
impl Session {
    pub fn new(doc: &Document, action: Action, available: usize) -> Result<Self, String> {
        let mut ids = BTreeSet::new();
        if action.name.trim().is_empty()
            || action.name.chars().count() > 64
            || action.name.chars().any(char::is_control)
            || action.steps.is_empty()
            || action.steps.len() > 256
            || !action.steps.iter().any(|s| s.enabled)
            || action
                .steps
                .iter()
                .any(|s| s.id == 0 || !ids.insert(s.id) || !s.operation.valid())
        {
            return Err("Invalid action or no enabled steps".into());
        }
        if doc
            .memory_bytes()
            .saturating_mul(10)
            .saturating_add(doc.resident_bytes())
            > available
        {
            return Err("Resumable action exceeds workspace memory".into());
        }
        let mut work = crate::layer_merge::preview(&doc.snapshot(), doc.current_frame)?;
        work.work_time = doc.work_time.clone();
        work.light_table = doc.light_table.clone();
        work.view = doc.view;
        work.tool = doc.tool;
        work.camera_guides = doc.camera_guides;
        work.onion = doc.onion;
        work.keyframe_content = doc.keyframe_content;
        work.title = doc.title.clone();
        work.path = doc.path.clone();
        work.source_path = doc.source_path.clone();
        Ok(Self {
            source: (doc.id, doc.revision, doc.current_frame),
            action,
            cursor: 0,
            issues: vec![],
            work,
            available,
        })
    }
    pub fn complete(&self) -> bool {
        self.cursor == self.action.steps.len()
    }
    pub fn document(&self) -> &Document {
        &self.work
    }
    pub fn skip(&mut self) -> Result<(), String> {
        if self.complete() {
            return Err("Action already complete".into());
        }
        self.issues
            .push(format!("Step {} skipped explicitly", self.cursor + 1));
        self.cursor += 1;
        Ok(())
    }
    /// Cursor advances only after an entire step succeeds; stop/retry cannot replay prior steps.
    pub fn advance(&mut self, errors: Errors, cancel: &AtomicBool) -> Result<(), String> {
        if self.complete() || cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let step = &self.action.steps[self.cursor];
        if !step.enabled {
            self.cursor += 1;
            return Ok(());
        }
        let result = prepare_operation(&self.work, &step.operation, cancel).and_then(|state| {
            if state
                .memory_bytes()
                .saturating_mul(10)
                .saturating_add(self.work.resident_bytes())
                > self.available
            {
                Err("Action result exceeds workspace memory".into())
            } else {
                Ok(state)
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match result {
            Ok(state) => {
                state.restore(&mut self.work);
                self.cursor += 1;
                Ok(())
            }
            Err(e) => {
                let e = format!("Step {}: {e}", self.cursor + 1);
                if errors == Errors::Skip {
                    self.issues.push(e);
                    self.cursor += 1;
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }
    pub fn run(&mut self, errors: Errors, cancel: &AtomicBool) -> Result<(), String> {
        while !self.complete() && !cancel.load(Ordering::Relaxed) {
            self.advance(errors, cancel)?;
        }
        Ok(())
    }
    pub fn commit(self, doc: &mut Document) -> Result<bool, String> {
        if !self.complete() {
            return Err("Finish or explicitly skip remaining steps first".into());
        }
        if self.source != (doc.id, doc.revision, doc.current_frame) {
            return Err("Original document/revision/frame changed; restart action".into());
        }
        let mut label = format!("Auto action: {}", self.action.name);
        while label.len() > 256 {
            label.pop();
        }
        doc.commit_state_named(self.work.snapshot(), &label)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ResultFile {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub status: String,
    pub issues: Vec<String>,
}
pub struct Batch {
    pub inputs: Vec<PathBuf>,
    pub outputs: Vec<PathBuf>,
    pub cursor: usize,
    pub results: Vec<ResultFile>,
    pub log: PathBuf,
    pub current: Option<Session>,
    pub action: Action,
    stamp: Option<crate::file_sources::Stamp>,
    notices: Vec<String>,
    log_hash: Option<String>,
    available: usize,
}
static NEXT: AtomicU64 = AtomicU64::new(1);
impl Batch {
    pub fn new(
        inputs: Vec<PathBuf>,
        folder: &Path,
        pattern: &str,
        action: Action,
        available: usize,
    ) -> Result<Self, String> {
        if inputs.is_empty() || inputs.len() > 128 {
            return Err("Select 1..128 local files".into());
        }
        let mut names = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for p in &inputs {
            crate::file_objects::local_path(p)?;
            let key = p.to_string_lossy().to_lowercase();
            if !sources.insert(key) {
                return Err("Duplicate batch input".into());
            }
        }
        let mut outputs = vec![];
        for (n, p) in inputs.iter().enumerate() {
            let name = crate::output_batch::filename(
                pattern,
                p.file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("Invalid source name")?,
                n + 1,
                "drawer",
            )?;
            let path = crate::asset_catalog::checked(folder, Path::new(&name), false)?;
            if !names.insert(path.to_string_lossy().to_lowercase())
                || path.exists()
                || inputs.iter().any(|p| {
                    p.to_string_lossy()
                        .eq_ignore_ascii_case(&path.to_string_lossy())
                })
            {
                return Err("Output collision/protected source; choose a new output name".into());
            }
            outputs.push(path);
        }
        let log = crate::asset_catalog::checked(
            folder,
            Path::new(&format!(
                "action-results-{}-{}.json",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            false,
        )?;
        if log.exists() {
            return Err("Batch log already exists".into());
        }
        let mut batch = Self {
            inputs,
            outputs,
            cursor: 0,
            results: vec![],
            log,
            current: None,
            action,
            stamp: None,
            notices: vec![],
            log_hash: None,
            available,
        };
        batch.checkpoint()?;
        Ok(batch)
    }
    pub fn complete(&self) -> bool {
        self.cursor == self.inputs.len()
    }
    pub fn checkpoint(&mut self) -> Result<(), String> {
        #[derive(Serialize)]
        struct Log<'a> {
            version: u32,
            action: &'a Action,
            next_file: usize,
            total: usize,
            step: Option<usize>,
            results: &'a [ResultFile],
        }
        let bytes = serde_json::to_vec_pretty(&Log {
            version: 1,
            action: &self.action,
            next_file: self.cursor,
            total: self.inputs.len(),
            step: self.current.as_ref().map(|s| s.cursor),
            results: &self.results,
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Batch log limit".into());
        }
        let parent = self.log.parent().ok_or("Missing log folder")?;
        crate::asset_catalog::checked(
            parent,
            Path::new(self.log.file_name().ok_or("Missing log name")?),
            false,
        )?;
        match &self.log_hash {
            Some(hash) => {
                if crate::asset_catalog::hash(&crate::asset_catalog::bytes(
                    &self.log,
                    8 * 1024 * 1024,
                )?) != *hash
                {
                    return Err(
                        "Batch log changed externally; preserve it and start a new batch".into(),
                    );
                }
            }
            None => {
                if self.log.exists() {
                    return Err("Foreign batch log is protected".into());
                }
            }
        }
        let temp = self.log.with_extension("json.action-log.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            if self.log_hash.is_none() {
                fs::hard_link(&temp, &self.log).map_err(|e| e.to_string())?;
                Ok(())
            } else {
                fs::rename(&temp, &self.log).map_err(|e| e.to_string())
            }
        })();
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        self.log_hash = Some(crate::asset_catalog::hash(&bytes));
        Ok(())
    }
    fn finish_file(&mut self, status: String, output: Option<PathBuf>) {
        let mut issues = std::mem::take(&mut self.notices);
        if let Some(s) = self.current.take() {
            issues.extend(s.issues)
        }
        if issues.len() > 16 {
            let omitted = issues.len() - 16;
            issues.truncate(16);
            issues.push(format!("{omitted} further step notices omitted"));
        }
        for text in &mut issues {
            *text = text.chars().take(512).collect();
        }
        self.results.push(ResultFile {
            input: self.inputs[self.cursor].clone(),
            output,
            status: status.chars().take(1024).collect(),
            issues,
        });
        self.cursor += 1;
        self.stamp = None;
    }
    pub fn skip_file(&mut self) -> Result<(), String> {
        if self.complete() {
            return Err("Batch already complete".into());
        }
        self.finish_file("Skipped explicitly".into(), None);
        self.checkpoint()
    }
    /// Completed outputs and cursor are retained even if writing the local log then fails.
    pub fn run(&mut self, steps: Errors, files: Errors, cancel: &AtomicBool) -> Result<(), String> {
        self.checkpoint()?;
        while !self.complete() && !cancel.load(Ordering::Relaxed) {
            let result: Result<bool, String> = (|| {
                if self.current.is_none() {
                    let stamp = crate::file_sources::Stamp::read(&self.inputs[self.cursor])?;
                    let (doc, notices) =
                        crate::files::open_report(&stamp.path, self.available / 12)
                            .map_err(|e| e.to_string())?;
                    stamp.validate()?;
                    let current = Session::new(&doc, self.action.clone(), self.available)?;
                    self.stamp = Some(stamp);
                    self.notices = notices;
                    self.current = Some(current);
                }
                let s = self.current.as_mut().unwrap();
                s.run(steps, cancel)?;
                if cancel.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                self.stamp
                    .as_ref()
                    .ok_or("Missing source fingerprint")?
                    .validate()?;
                let bytes = crate::files::native_bytes(s.document()).map_err(|e| e.to_string())?;
                if bytes.len() > 64 * 1024 * 1024 {
                    return Err("Batch output exceeds 64 MiB".into());
                }
                let warning = publish_new(&self.outputs[self.cursor], &bytes, cancel)?;
                if let Some(warning) = warning {
                    self.notices.push(warning)
                }
                Ok(true)
            })();
            if cancel.load(Ordering::Relaxed) && !matches!(result, Ok(true)) {
                self.checkpoint()?;
                return Ok(());
            }
            match result {
                Ok(true) => {
                    self.finish_file("Written".into(), Some(self.outputs[self.cursor].clone()))
                }
                Ok(false) => return Ok(()),
                Err(e) => {
                    if files == Errors::Stop {
                        self.checkpoint()?;
                        return Err(format!("File {}: {e}", self.cursor + 1));
                    }
                    self.finish_file(format!("Failed: {e}"), None);
                }
            }
            self.checkpoint()?;
        }
        self.checkpoint()
    }
}
fn publish_new(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<Option<String>, String> {
    let parent = path.parent().ok_or("Missing output folder")?;
    let path = crate::asset_catalog::checked(
        parent,
        Path::new(path.file_name().ok_or("Missing output name")?),
        false,
    )?;
    if path.exists() {
        return Err("Output exists; no file replaced".into());
    }
    let temp = path.with_extension("drawer.action-write.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        for b in bytes.chunks(65536) {
            crate::asset_catalog::canceled(cancel)?;
            file.write_all(b).map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        crate::asset_catalog::canceled(cancel)?;
        fs::hard_link(&temp, &path).map_err(|e| e.to_string())
    })();
    let cleanup = fs::remove_file(&temp);
    result?;
    Ok(cleanup.err().map(|e| {
        format!(
            "Output completed; owned temporary file requires cleanup: {} ({e})",
            temp.display()
        )
    }))
}
#[path = "action_runner_verify.rs"]
pub mod verify;
