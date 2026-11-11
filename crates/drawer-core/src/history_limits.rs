use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub steps: u32,
    pub memory_mib: u32,
    pub group_ms: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            steps: 100,
            memory_mib: 64,
            group_ms: 1000,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (1..=1000).contains(&self.steps)
            && (1..=64).contains(&self.memory_mib)
            && self.group_ms <= 5000
    }
    pub fn bytes(self) -> usize {
        self.memory_mib as usize * 1024 * 1024
    }
}
impl Document {
    pub fn can_configure_history(&self, s: Settings) -> Result<(), String> {
        if !s.valid() {
            return Err("History ranges:1..1000 steps;1..64MiB; grouping0..5000ms".into());
        }
        if self.history.redo.len() > s.steps as usize
            || self.history.redo.iter().map(Edit::bytes).sum::<usize>() > s.bytes()
        {
            return Err("Retained Redo alone exceeds the proposed limit. Redo, make a new edit or explicitly clear history before reducing it; current records preserved".into());
        }
        Ok(())
    }
    pub fn configure_history(&mut self, s: Settings) -> Result<usize, String> {
        self.can_configure_history(s)?;
        let before = self.history.undo.len();
        if self.history.limits != s {
            self.history.last_group = None;
            self.history.limits = s;
        }
        self.trim_history_memory();
        Ok(before - self.history.undo.len())
    }
    pub fn history_limits(&self) -> Settings {
        self.history.limits
    }
    pub fn history_pruned(&self) -> u64 {
        self.history.pruned
    }
    pub fn history_barrier(&mut self) {
        self.history.last_group = None;
    }
    pub fn history_bytes(&self) -> usize {
        self.history
            .undo
            .iter()
            .chain(&self.history.redo)
            .map(Edit::bytes)
            .sum()
    }
}
#[path = "history_limits_verify.rs"]
pub mod verify;
