//! Runtime values stay live; persistence and tool changes restore locked baselines.
use crate::{assets::BrushTip, brush_presets::Preset, workspaces::Snapshot};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub builtin: Box<[Option<Box<Preset>>; 7]>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            builtin: Box::new(std::array::from_fn(|_| None)),
        }
    }
}
fn slot(tip: BrushTip) -> usize {
    BrushTip::ALL.iter().position(|t| *t == tip).unwrap()
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.builtin.iter().enumerate().all(|(n, p)| {
            p.as_ref()
                .is_none_or(|p| p.locked && p.tip == BrushTip::ALL[n] && p.validate().is_ok())
        })
    }
    pub fn empty(&self) -> bool {
        self.builtin.iter().all(Option::is_none)
    }
}
pub fn baseline(s: &Snapshot, tip: BrushTip) -> Option<&Preset> {
    if let Some(t) = s.user_tools.active_entry().filter(|t| t.preset.tip == tip) {
        return t.preset.locked.then_some(&t.preset);
    }
    s.locks.builtin[slot(tip)].as_deref()
}
pub fn set(current: &Snapshot, tip: BrushTip, locked: bool) -> Result<Snapshot, String> {
    if !current.valid() || current.tool != tip.name() {
        return Err("Choose the current drawing tool before locking".into());
    }
    let mut next = current.clone();
    if let Some(t) = current.user_tools.active_entry() {
        let id = t.id;
        let p = Preset::capture(current, tip, &t.preset.name)?;
        next.user_tools.lock_tool(id, locked, Some(p))?;
    } else if locked {
        if next.locks.builtin[slot(tip)].is_none() {
            let mut p = Preset::capture(current, tip, &format!("Locked {}", tip.label()))?;
            p.locked = true;
            next.locks.builtin[slot(tip)] = Some(Box::new(p));
        }
    } else {
        if let Some(p) = next.locks.builtin[slot(tip)].take() {
            for stored in &mut next.brushes.presets {
                if stored.name.eq_ignore_ascii_case(&p.name) {
                    stored.locked = false;
                }
            }
        }
    }
    Ok(next)
}
pub fn restore_current(current: &Snapshot, tip: BrushTip) -> Result<Snapshot, String> {
    match baseline(current, tip) {
        Some(p) => p.apply(current),
        None => Ok(current.clone()),
    }
}
pub fn apply_preset(current: &Snapshot, p: &Preset) -> Result<Snapshot, String> {
    let old_tip = BrushTip::ALL
        .into_iter()
        .find(|t| t.name() == current.tool)
        .ok_or("Invalid tool")?;
    let base = restore_current(current, old_tip)?;
    let mut next = p.apply(&base)?;
    next.locks.builtin[slot(p.tip)] = p.locked.then(|| Box::new(p.clone()));
    Ok(next)
}
/// Only the serialized copy is normalized. The caller retains its temporary values.
pub fn for_storage(current: &Snapshot) -> Result<Snapshot, String> {
    let tip = BrushTip::ALL
        .into_iter()
        .find(|t| t.name() == current.tool)
        .ok_or("Invalid active tool")?;
    if current.locks.empty() && baseline(current, tip).is_none() {
        return Ok(current.clone());
    }
    let active =
        baseline(current, tip)
            .cloned()
            .unwrap_or(Preset::capture(current, tip, "Live brush")?);
    let mut next = current.clone();
    for p in current.locks.builtin.iter().flatten() {
        next = p.apply(&next)?;
    }
    next = active.apply(&next)?;
    Ok(next)
}
