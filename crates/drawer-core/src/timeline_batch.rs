use crate::{
    document::Document,
    sizing::DocumentState,
    timeline_time_edit::{dense, encode},
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::BTreeMap};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Selector {
    Ids { first: u64, last: u64 },
    Names { first: String, last: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub track: u64,
    pub selector: Selector,
    pub start: u32,
    pub end: u32,
    pub hold: u32,
    pub gap: u32,
    pub step: u32,
    pub repeats: u32,
}
impl Default for Pattern {
    fn default() -> Self {
        Self {
            track: 0,
            selector: Selector::Ids { first: 1, last: 1 },
            start: 0,
            end: 1,
            hold: 1,
            gap: 0,
            step: 1,
            repeats: 1,
        }
    }
}
impl Pattern {
    pub fn valid(&self) -> bool {
        self.start < self.end
            && self.end <= 10000
            && (1..=10000).contains(&self.hold)
            && self.gap <= 10000
            && (1..=10000).contains(&self.step)
            && self.repeats <= 10000
            && match &self.selector {
                Selector::Ids { first, last } => *first > 0 && first <= last,
                Selector::Names { first, last } => {
                    !first.trim().is_empty()
                        && !last.trim().is_empty()
                        && first.len() <= 256
                        && last.len() <= 256
                }
            }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presets {
    pub entries: BTreeMap<String, Pattern>,
}
impl Presets {
    pub fn valid(&self) -> bool {
        self.entries.len() <= 64
            && self.entries.iter().all(|(n, p)| {
                !n.trim().is_empty()
                    && n.len() <= 128
                    && !n.chars().any(char::is_control)
                    && p.valid()
            })
    }
    pub fn set(&mut self, name: String, pattern: Pattern) -> Result<(), String> {
        let mut next = self.clone();
        next.entries.insert(name, pattern);
        if !next.valid() {
            return Err("Preset name/rule/count limit".into());
        }
        *self = next;
        Ok(())
    }
}
fn natural(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (mut x, mut y) = (i, j);
            while x < a.len() && a[x].is_ascii_digit() {
                x += 1;
            }
            while y < b.len() && b[y].is_ascii_digit() {
                y += 1;
            }
            let (mut p, mut q) = (i, j);
            while p + 1 < x && a[p] == b'0' {
                p += 1;
            }
            while q + 1 < y && b[q] == b'0' {
                q += 1;
            }
            let o = (x - p).cmp(&(y - q)).then_with(|| a[p..x].cmp(&b[q..y]));
            if o != Ordering::Equal {
                return o;
            }
            (i, j) = (x, y);
        } else {
            let o = a[i].cmp(&b[j]);
            if o != Ordering::Equal {
                return o;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}
pub fn sources(d: &Document, p: &Pattern) -> Result<Vec<u64>, String> {
    if !p.valid() {
        return Err("Invalid batch selection/rule".into());
    }
    let defs = &d.scene.cels.definitions;
    let list = match &p.selector {
        Selector::Ids { first, last } => {
            let mut ids = defs
                .iter()
                .filter(|s| {
                    s.object >= *first
                        && s.object <= *last
                        && (s.object - first).is_multiple_of(p.step as u64)
                })
                .map(|s| s.object)
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }
        Selector::Names { first, last } => {
            if defs.iter().filter(|s| s.name == *first).count() != 1
                || defs.iter().filter(|s| s.name == *last).count() != 1
            {
                return Err(
                    "Name endpoints must be unique; use cel IDs for duplicate names".into(),
                );
            }
            let mut names = defs.iter().collect::<Vec<_>>();
            names.sort_by(|a, b| natural(&a.name, &b.name).then_with(|| a.object.cmp(&b.object)));
            let from = names.iter().position(|s| s.name == *first).unwrap();
            let to = names.iter().position(|s| s.name == *last).unwrap();
            if from > to {
                return Err("Name range is reversed in natural order".into());
            }
            names[from..=to]
                .iter()
                .step_by(p.step as usize)
                .map(|s| s.object)
                .collect()
        }
    };
    if list.is_empty() {
        Err("No existing cel IDs/names match this rule".into())
    } else {
        Ok(list)
    }
}
pub fn apply(
    s: &mut DocumentState,
    p: &Pattern,
    rows: &mut Vec<String>,
    budget: usize,
) -> Result<u32, String> {
    if !p.valid() || p.end > s.animation.as_ref().ok_or("Choose an animation")?.frames {
        return Err("Invalid batch interval, hold, gap, skip or repetition".into());
    }
    let d = crate::layer_merge::preview(s, 0)?;
    let ids = sources(&d, p)?;
    let n = s
        .scene
        .catalog
        .tracks
        .index(p.track, s.animation.as_ref().unwrap().tracks.len())
        .ok_or("Select a live target track")?;
    let old = dense(s, n);
    let end = s.animation.as_ref().unwrap().tracks[n].end.max(p.end);
    let mut values = old.clone();
    values.resize(end as usize, None);
    let mut at = p.start;
    let mut cycles = 0;
    let mut replaced = 0;
    while at < p.end && (p.repeats == 0 || cycles < p.repeats) {
        for id in &ids {
            if at >= p.end {
                break;
            }
            crate::timeline_editor::attach(s, p.track, *id, at, budget)?;
            let link = s
                .scene
                .cels
                .links
                .iter()
                .find(|l| l.track == p.track && l.source == *id)
                .unwrap();
            let cell = s.scene.catalog.cells[&p.track]
                .index(
                    link.cell,
                    s.animation.as_ref().unwrap().tracks[n].cells.len(),
                )
                .unwrap();
            let stop = at.saturating_add(p.hold).min(p.end);
            for f in at..stop {
                if old.get(f as usize).is_some_and(Option::is_some) {
                    replaced += 1;
                }
                values[f as usize] = Some(cell);
            }
            rows.push(format!(
                "Track{} frames{}–{} ← cel{} ({})",
                p.track,
                at + 1,
                stop,
                id,
                s.scene.cels.get(*id).unwrap().name
            ));
            at = stop;
            let stop = at.saturating_add(p.gap).min(p.end);
            for f in at..stop {
                if old.get(f as usize).is_some_and(Option::is_some) {
                    replaced += 1;
                }
                values[f as usize] = None;
            }
            if stop > at {
                rows.push(format!("Frames{}–{} ← empty", at + 1, stop));
            }
            at = stop;
        }
        cycles += 1;
    }
    encode(s, n, &values)?;
    rows.insert(
        0,
        format!(
            "Batch: {} existing cels, {} cycles; range{}–{} (outside range preserved)",
            ids.len(),
            cycles,
            p.start + 1,
            p.end
        ),
    );
    Ok(replaced)
}
