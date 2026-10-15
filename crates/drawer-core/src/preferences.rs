//! Local language, confirmation preferences and reversible IME sessions.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    Korean,
    English,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImeMode {
    #[default]
    Keep,
    Open,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confirmation {
    ResetProperties,
    ResetLayout,
}
impl Confirmation {
    pub const ALL: [Self; 2] = [Self::ResetProperties, Self::ResetLayout];
    pub fn label(self) -> &'static str {
        match self {
            Self::ResetProperties => "Reset tool property draft",
            Self::ResetLayout => "Reset dock layout draft",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub language: Language,
    pub text_ime: ImeMode,
    pub number_ime: ImeMode,
    pub suppressed: BTreeSet<Confirmation>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            language: Language::Korean,
            text_ime: ImeMode::Keep,
            number_ime: ImeMode::Keep,
            suppressed: BTreeSet::new(),
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1 && self.number_ime != ImeMode::Open
    }
    pub fn restore(&mut self, warning: Option<Confirmation>) {
        if let Some(w) = warning {
            self.suppressed.remove(&w);
        } else {
            self.suppressed.clear();
        }
    }
}
pub trait ImeBackend {
    /// Acquire only the current application UI thread's input context.
    fn begin(&mut self) -> Result<bool, String>;
    fn set(&mut self, open: bool) -> Result<(), String>;
    fn end(&mut self);
}
#[derive(Default)]
pub struct ImeSession {
    target: Option<(u64, ImeMode)>,
    original: Option<bool>,
    applied: Option<bool>,
    failed: bool,
}
impl ImeSession {
    pub fn finish(&mut self, backend: &mut impl ImeBackend) -> Result<(), String> {
        if let Some(original) = self.original {
            backend.set(original)?;
        }
        self.original = None;
        self.applied = None;
        self.target = None;
        self.failed = false;
        backend.end();
        Ok(())
    }
    pub fn update(
        &mut self,
        backend: &mut impl ImeBackend,
        target: Option<(u64, ImeMode)>,
        composing: bool,
    ) -> Result<(), String> {
        if self.target != target {
            self.finish(backend)?;
            self.target = target;
        }
        let Some((_, mode)) = target else {
            return Ok(());
        };
        if self.failed {
            return Ok(());
        }
        if mode == ImeMode::Keep {
            return Ok(());
        }
        if self.original.is_none() {
            match backend.begin() {
                Ok(original) => self.original = Some(original),
                Err(e) => {
                    self.failed = true;
                    backend.end();
                    return Err(e);
                }
            }
        }
        if composing {
            return Ok(());
        }
        let desired = mode == ImeMode::Open;
        if self.applied.is_none() {
            if self.original != Some(desired)
                && let Err(error) = backend.set(desired)
            {
                self.failed = true;
                if let Err(restore) = backend.set(self.original.unwrap()) {
                    return Err(format!("{error}; restore pending: {restore}"));
                }
                self.original = None;
                backend.end();
                return Err(error);
            }
            self.applied = Some(desired);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        open: bool,
        fail_begin: bool,
        fail_set: bool,
        sets: Vec<bool>,
        ended: usize,
    }
    impl ImeBackend for Fake {
        fn begin(&mut self) -> Result<bool, String> {
            if self.fail_begin {
                Err("query failed".into())
            } else {
                Ok(self.open)
            }
        }
        fn set(&mut self, open: bool) -> Result<(), String> {
            self.sets.push(open);
            if self.fail_set {
                self.fail_set = false;
                Err("control denied".into())
            } else {
                self.open = open;
                Ok(())
            }
        }
        fn end(&mut self) {
            self.ended += 1;
        }
    }
    #[test]
    fn ime_entry_exit_composition_focus_loss_and_failure_restore_previous_state() {
        for original in [false, true] {
            for mode in [ImeMode::Keep, ImeMode::Open, ImeMode::Closed] {
                let mut b = Fake {
                    open: original,
                    fail_begin: false,
                    fail_set: false,
                    sets: vec![],
                    ended: 0,
                };
                let mut s = ImeSession::default();
                s.update(&mut b, Some((1, mode)), true).unwrap();
                assert_eq!(b.open, original);
                s.update(&mut b, Some((1, mode)), false).unwrap();
                assert_eq!(
                    b.open,
                    if mode == ImeMode::Keep {
                        original
                    } else {
                        mode == ImeMode::Open
                    }
                );
                let count = b.sets.len();
                s.update(&mut b, Some((1, mode)), false).unwrap();
                assert_eq!(b.sets.len(), count);
                s.update(&mut b, None, false).unwrap();
                assert_eq!(b.open, original);
            }
        }
        let mut b = Fake {
            open: true,
            fail_begin: false,
            fail_set: true,
            sets: vec![],
            ended: 0,
        };
        let mut s = ImeSession::default();
        assert!(s.update(&mut b, Some((1, ImeMode::Closed)), false).is_err());
        assert!(b.open);
        let count = b.sets.len();
        s.update(&mut b, Some((1, ImeMode::Closed)), false).unwrap();
        assert_eq!(b.sets.len(), count);
        s.update(&mut b, None, false).unwrap();
        b.fail_begin = true;
        assert!(s.update(&mut b, Some((2, ImeMode::Open)), false).is_err());
        assert!(b.open);
        let mut p = Settings::default();
        p.suppressed.extend(Confirmation::ALL);
        p.restore(Some(Confirmation::ResetLayout));
        assert!(!p.suppressed.contains(&Confirmation::ResetLayout));
        assert!(p.suppressed.contains(&Confirmation::ResetProperties));
        p.restore(None);
        assert!(p.suppressed.is_empty());
    }
}
