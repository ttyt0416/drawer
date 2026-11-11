//! Session-only state identities. Tokens and outstanding save fences survive clearing.
use super::*;
#[derive(Clone, Debug)]
pub struct Entry {
    pub token: u64,
    pub label: String,
    pub current: bool,
    pub redo: bool,
}
impl Document {
    pub fn history_entries(&self) -> Vec<Entry> {
        let h = &self.history;
        let first = h.undo.first().map_or(h.current, |e| e.from);
        let mut rows = vec![Entry {
            token: first,
            label: "Retained starting state".into(),
            current: first == h.current,
            redo: false,
        }];
        for (e, redo) in h
            .undo
            .iter()
            .map(|e| (e, false))
            .chain(h.redo.iter().rev().map(|e| (e, true)))
        {
            rows.push(Entry {
                token: e.to,
                label: e.label.clone().unwrap_or_else(|| match e.kind {
                    EditKind::Pixel { .. } | EditKind::Pixels { .. } | EditKind::Multi { .. } => {
                        "Drawing".into()
                    }
                    _ => "Document change".into(),
                }),
                current: e.to == h.current,
                redo,
            });
        }
        rows
    }
    pub fn jump_history(&mut self, token: u64) -> Result<bool, String> {
        let rows = self.history_entries();
        let target = rows
            .iter()
            .position(|e| e.token == token)
            .ok_or("This history state was removed; refresh the list")?;
        let current = rows
            .iter()
            .position(|e| e.current)
            .ok_or("Current history state missing")?;
        if current == target {
            return Ok(false);
        }
        // Stage the whole move: an intermediate allocation failure cannot publish a partial jump.
        if self
            .resident_bytes()
            .checked_mul(2)
            .is_none_or(|n| n > self.working_budget)
        {
            return Err("History jump needs a second state within the working memory budget; use single-step Undo/Redo".into());
        }
        let mut candidate = self.clone();
        for _ in 0..current.abs_diff(target) {
            let changed = if target < current {
                candidate.undo()
            } else {
                candidate.redo()
            };
            if !changed {
                return Err(candidate
                    .take_resource_error()
                    .unwrap_or("History move could not be completed; document preserved".into()));
            }
        }
        *self = candidate;
        Ok(true)
    }
    /// Confirmation belongs to the UI. This releases records without changing the drawing identity.
    pub fn clear_session_history(&mut self) {
        self.history.undo.clear();
        self.history.undo.shrink_to_fit();
        self.history.redo.clear();
        self.history.redo.shrink_to_fit();
    }
}
#[path = "history_navigation_verify.rs"]
pub mod verify;
