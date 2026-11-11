//! A completed write marks exactly the captured logical state as saved.
use crate::document::Document;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub struct Checkpoint {
    document: u64,
    pub revision: u64,
    token: u64,
    meta: u64,
    serial: u64,
    light_instance: u64,
    light_token: u64,
    previous_path: Option<PathBuf>,
}
#[path = "saved_revision_verify.rs"]
pub mod verify;
impl Document {
    pub fn begin_save_checkpoint(&mut self) -> Result<Checkpoint, String> {
        if self.history.inflight.len() >= 32 {
            return Err("Too many outstanding document saves".into());
        }
        let serial = self
            .history
            .save_serial
            .checked_add(1)
            .ok_or("Save request counter exhausted")?;
        self.history.save_serial = serial;
        self.history.inflight.insert(serial, self.history.current);
        Ok(Checkpoint {
            document: self.id,
            revision: self.revision,
            token: self.history.current,
            meta: self.history.meta,
            serial,
            light_instance: self.light_table.instance(),
            light_token: self.light_table.stamp(),
            previous_path: self.path.clone(),
        })
    }
    pub fn cancel_save_checkpoint(&mut self, checkpoint: Checkpoint) -> bool {
        checkpoint.document == self.id
            && self.history.inflight.remove(&checkpoint.serial) == Some(checkpoint.token)
    }
    /// Call only after the writer verified and published this checkpoint's native file.
    pub fn complete_save_checkpoint(
        &mut self,
        checkpoint: Checkpoint,
        path: &Path,
    ) -> Result<bool, String> {
        if checkpoint.document != self.id {
            return Err("Saved result belongs to another document".into());
        }
        if self.history.inflight.remove(&checkpoint.serial) != Some(checkpoint.token) {
            return Err("Save request was canceled or already consumed".into());
        }
        if checkpoint.serial < self.history.saved_serial {
            return Err("A newer save already completed; stale result ignored".into());
        }
        if self.path != checkpoint.previous_path && self.path.as_deref() != Some(path) {
            return Err(
                "Document destination changed while saving; live destination preserved".into(),
            );
        }
        self.history.saved = Some(checkpoint.token);
        self.history.saved_meta = checkpoint.meta;
        self.history.saved_serial = checkpoint.serial;
        if !self
            .light_table
            .mark_saved_checkpoint(checkpoint.light_instance, checkpoint.light_token)
        {
            self.history.saved = None;
        }
        self.path = Some(path.to_path_buf());
        self.source_path = None;
        self.title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.refresh_dirty();
        Ok(!self.dirty)
    }
}
