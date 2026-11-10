//! Iteration scheduling is independent of drawing Undo and wall-clock time.
use crate::{
    action_runner::Session, auto_actions::Action, document::Document, storage::UserStorage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::{OnceLock, atomic::AtomicBool},
    time::Instant,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub count: u16,
    pub wait_ms: u32,
    pub confirm: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            count: 1,
            wait_ms: 0,
            confirm: false,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1 && (1..=1000).contains(&self.count) && self.wait_ms <= 60000
    }
    pub fn load(s: &UserStorage) -> Result<Self, String> {
        let p = s.root().join("settings/action-repeat.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let bytes = crate::asset_catalog::bytes(&p, 16384)?;
        let v: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !v.valid() {
            return Err("Invalid repeat preferences; original file preserved".into());
        }
        Ok(v)
    }
    pub fn save(&self, s: &UserStorage) -> Result<(), String> {
        if !self.valid() {
            return Err("Repeat count1..1000 and wait0..60000ms required".into());
        }
        let p = s.root().join("settings/action-repeat.json");
        let tmp = p.with_extension("json.write.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            fs::rename(&tmp, p).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
}
pub fn now() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
#[derive(Clone, Debug)]
pub struct Schedule {
    pub settings: Settings,
    pub completed: u16,
    pub paused: bool,
    pub confirmation: bool,
    pub deadline: u64,
}
impl Schedule {
    pub fn new(settings: Settings) -> Result<Self, String> {
        if !settings.valid() {
            return Err("Invalid repeat options".into());
        }
        Ok(Self {
            settings,
            completed: 0,
            paused: false,
            confirmation: false,
            deadline: 0,
        })
    }
    pub fn complete(&self) -> bool {
        self.completed == self.settings.count
    }
    pub fn ready(&self, now: u64) -> bool {
        !self.complete() && !self.paused && !self.confirmation && now >= self.deadline
    }
    pub fn finish(&mut self, now: u64) {
        self.completed += 1;
        self.deadline = now.saturating_add(self.settings.wait_ms as u64);
        self.confirmation = !self.complete() && self.settings.confirm;
    }
    pub fn stop(&mut self) {
        self.paused = true;
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.confirmation = false;
    }
}
pub struct Run {
    pub action: Action,
    pub schedule: Schedule,
    source: (u64, u64, u32),
    available: usize,
}
impl Run {
    pub fn new(
        d: &Document,
        action: Action,
        settings: Settings,
        available: usize,
    ) -> Result<Self, String> {
        Session::new(d, action.clone(), available)?;
        Ok(Self {
            action,
            schedule: Schedule::new(settings)?,
            source: (d.id, d.revision, d.current_frame),
            available,
        })
    }
    pub fn source(&self) -> u64 {
        self.source.0
    }
    pub fn prepare(&self, d: &Document, now: u64) -> Result<Session, String> {
        if !self.schedule.ready(now) {
            return Err("Wait, confirm continuation, or resume the next incomplete round".into());
        }
        if self.source != (d.id, d.revision, d.current_frame) {
            return Err("Original document/revision/frame changed; restart rounds".into());
        }
        Session::new(d, self.action.clone(), self.available)
    }
    pub fn commit(
        &mut self,
        s: Session,
        d: &mut Document,
        now: u64,
        ppp: f64,
    ) -> Result<bool, String> {
        if self.source != (d.id, d.revision, d.current_frame) {
            return Err("Round source changed; prepared result discarded".into());
        }
        let changed = s.commit_with_scale(d, ppp)?;
        self.source = (d.id, d.revision, d.current_frame);
        self.schedule.finish(now);
        Ok(changed)
    }
}
#[path = "action_repeat_verify.rs"]
pub mod verify;
