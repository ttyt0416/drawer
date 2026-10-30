//! Local animation finishing controls; guide pixels never enter final compositing.
use crate::{
    animation_render::Area,
    document::Document,
    scene::{Point, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub safe: Option<[u8; 2]>,
    pub area: Option<Area>,
}
impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        self.safe
            .is_none_or(|p| p.iter().all(|p| (1..=100).contains(p)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub output: bool,
    pub safe: bool,
    pub overflow: bool,
    pub range: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            output: true,
            safe: true,
            overflow: false,
            range: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub pattern: String,
    pub start: u32,
    pub step: u32,
    pub width: u8,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            pattern: "Cel_{n}".into(),
            start: 1,
            step: 1,
            width: 3,
        }
    }
}
fn filename(n: &str) -> bool {
    let head = n
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    !n.trim().is_empty()
        && n.len() <= 180
        && n.chars().count() <= 48
        && n.trim() == n
        && !n.ends_with([' ', '.'])
        && !n
            .chars()
            .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
        && !matches!(head.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(head.len() == 4
            && (head.starts_with("COM") || head.starts_with("LPT"))
            && matches!(head.as_bytes()[3], b'1'..=b'9'))
}
impl Rule {
    pub fn valid(&self) -> bool {
        self.pattern.len() <= 256
            && !self.pattern.is_empty()
            && self.step > 0
            && self.step <= 1000
            && self.width <= 9
    }
    pub fn name(&self, ordinal: usize, id: u64, old: &str) -> Result<String, String> {
        if !self.valid() {
            return Err("Invalid naming pattern, step or width".into());
        }
        let n = (ordinal as u64)
            .checked_mul(self.step as u64)
            .and_then(|n| n.checked_add(self.start as u64))
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or("Name counter exceeds32-bit range")?;
        let mut out = String::new();
        let mut rest = self.pattern.as_str();
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix("{{") {
                out.push('{');
                rest = r;
            } else if let Some(r) = rest.strip_prefix("}}") {
                out.push('}');
                rest = r;
            } else if let Some(r) = rest.strip_prefix('{') {
                let end = r.find('}').ok_or("Close each naming token")?;
                match &r[..end] {
                    "n" => out.push_str(&format!("{:0width$}", n, width = self.width as usize)),
                    "id" => out.push_str(&id.to_string()),
                    "old" => out.push_str(old),
                    _ => return Err("Use {n}, {id}, {old}, {{ or }} in naming rules".into()),
                }
                rest = &r[end + 1..];
            } else {
                let c = rest.chars().next().unwrap();
                if c == '}' {
                    return Err("Unmatched name token brace".into());
                }
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
            if out.len() > 255 {
                return Err("Generated name exceeds255 UTF-8 bytes".into());
            }
        }
        use unicode_normalization::UnicodeNormalization;
        let out = out.nfc().collect::<String>();
        if !filename(&out) {
            return Err("Generated name must be a safe local filename (at most48 characters /180 UTF-8 bytes)".into());
        }
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub view: View,
    pub cel: Rule,
    pub shot: Rule,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            view: Default::default(),
            cel: Default::default(),
            shot: Rule {
                pattern: "Shot_{n}".into(),
                ..Default::default()
            },
        }
    }
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.version == 1 && self.cel.valid() && self.shot.valid()
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local finishing preferences".into());
        }
        let p = root.join("settings/animation-finish.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let b = crate::book_io::read_local(&p, 8192)?;
        let p: Self = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid finishing preferences; file preserved".into());
        }
        Ok(p)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid local finishing preferences".into());
        }
        let parent = root.join("settings");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        let mut stage = crate::animation_export::Stage::new(&parent)?;
        let mut f = stage.create("finish.json")?;
        f.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(
            stage.directory.join("finish.json"),
            parent.join("animation-finish.json"),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guides {
    pub output: Option<[Point; 4]>,
    pub safe: Option<[Point; 4]>,
    pub overflow: Option<[Point; 4]>,
    pub outside: bool,
}
pub fn guides(d: &Document, view: View, budget: usize) -> Result<Guides, String> {
    if d.animation.is_none() {
        return Ok(Guides::default());
    }
    let f =
        d.scene
            .animation_output
            .at(d.current_frame)
            .unwrap_or(crate::animation_output::Frame {
                frame: d.current_frame,
                bounds: Rect {
                    min: [0., 0.],
                    max: [d.width as f64, d.height as f64],
                },
                creation: d.scene.transform,
            });
    let output = view.output.then(|| f.corners(d.scene.transform));
    let safe = if view.safe {
        d.scene.animation_output.settings.safe.map(|p| {
            let mut key = f;
            for (i, percent) in p.into_iter().enumerate() {
                let inset = (f.bounds.max[i] - f.bounds.min[i]) * (100 - percent) as f64 / 200.;
                key.bounds.min[i] += inset;
                key.bounds.max[i] -= inset;
            }
            key.corners(d.scene.transform)
        })
    } else {
        None
    };
    let overflow = if view.overflow {
        let layout = crate::animation_render::layout(
            d,
            crate::animation_export::Options {
                first: d.current_frame,
                last: d.current_frame,
                view: crate::animation_render::View {
                    area: Area::Overflow,
                    ..Default::default()
                },
                ..Default::default()
            },
            budget,
        )?;
        let [x, y] = layout.origin.map(|v| v as f64);
        Some([
            [x, y],
            [x + layout.width as f64, y],
            [x + layout.width as f64, y + layout.height as f64],
            [x, y + layout.height as f64],
        ])
    } else {
        None
    };
    let outside = view.range
        && d.scene
            .timing
            .playback
            .is_some_and(|p| d.current_frame < p.first || d.current_frame >= p.end);
    Ok(Guides {
        output,
        safe,
        overflow,
        outside,
    })
}
pub enum Edit {
    ClearFrames,
    Output {
        settings: Settings,
        first: u32,
        end: u32,
        bounds: Option<Rect>,
    },
    Audio {
        id: u64,
        start: u32,
        gain: u16,
        mute: bool,
    },
    Names {
        cels: bool,
        shots: bool,
        cel: Rule,
        shot: Rule,
    },
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    pub rows: Vec<String>,
    pub summary: String,
}
impl Prepared {
    pub fn matches(&self, d: &Document) -> bool {
        self.id == d.id && self.revision == d.revision
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("Finishing draft is stale; prepare again".into());
        }
        d.commit_state(self.state)
    }
}
fn rename_shot_cells(
    shots: &mut crate::shot_exchange::Data,
    links: &crate::cels::Data,
    names: &BTreeMap<u64, String>,
) {
    for shot in &mut shots.entries {
        for (cell, image) in shot.cells.iter().zip(&mut shot.shot.cells) {
            if let Some(name) = links
                .source(shot.track, *cell)
                .and_then(|id| names.get(&id))
            {
                image.name = name.clone();
            }
        }
    }
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Finishing preparation exceeds available memory".into());
    }
    let a = d.animation.as_ref().ok_or("Choose an animation")?;
    let mut rows = vec![];
    let (state, summary) = match edit {
        Edit::ClearFrames => {
            let mut s = d.snapshot();
            rows.push(format!(
                "Remove {} output frame keys in this timeline",
                s.scene.animation_output.keys.len()
            ));
            s.scene.animation_output.keys.clear();
            (s, "Clear output frame keys; one Undo restores them".into())
        }
        Edit::Output {
            settings,
            first,
            end,
            bounds,
        } => {
            if !settings.valid() || first >= end || end > a.frames {
                return Err(
                    "Safe percentages must be1..100 and output range inside the timeline".into(),
                );
            }
            let repeat = d.scene.timing.playback.is_some_and(|p| p.repeat);
            let mut s = d.snapshot();
            s.scene.timing.playback = Some(crate::timeline_timing::Playback { first, end, repeat });
            if let Some(bounds) = bounds {
                rows.push(format!(
                    "Output frame key{}: {:?}",
                    d.current_frame + 1,
                    bounds
                ));
                s.scene.animation_output.keys = crate::animation_output::prepare(d, bounds)?
                    .animation_output
                    .keys;
            }
            s.scene.animation_output.settings = settings;
            rows.push(format!(
                "Output/playback frames{}–{}; area {:?}; safe {:?}",
                first + 1,
                end,
                settings.area,
                settings.safe
            ));
            (
                s,
                "Set output frame, title-safe geometry and default output/playback range".into(),
            )
        }
        Edit::Audio {
            id,
            start,
            gain,
            mute,
        } => {
            let data = crate::timeline_timing::inventory(d)?;
            let clip = data
                .audio
                .iter()
                .find(|c| c.id == id)
                .ok_or("Choose an audio clip")?;
            let source = &a.audio[clip.index];
            rows.push(format!(
                "{}: start{} → {}; volume{} → {}%; mute{} → {}",
                source.name,
                source.start + 1,
                start + 1,
                clip.sound.gain,
                gain,
                clip.sound.mute,
                mute
            ));
            (
                crate::audio::edit(d, id, source.name.clone(), start, gain, mute, budget)?,
                "Edit audio start/volume without changing original PCM".into(),
            )
        }
        Edit::Names {
            cels,
            shots,
            cel,
            shot,
        } => {
            if !cels && !shots {
                return Err("Select cels or shots to rename".into());
            }
            let mut s = if cels {
                crate::cels::enable(d, budget)?
            } else {
                d.snapshot()
            };
            let mut names = BTreeMap::new();
            if cels {
                if s.scene.cels.definitions.is_empty() {
                    return Err("No editable cel definitions to rename".into());
                }
                let mut ids = s
                    .scene
                    .cels
                    .definitions
                    .iter()
                    .map(|p| p.object)
                    .collect::<Vec<_>>();
                ids.sort_unstable();
                let mut seen = BTreeSet::new();
                for (i, id) in ids.into_iter().enumerate() {
                    crate::book_io::check_cancel(cancel)?;
                    let old = s.scene.cels.get(id).unwrap();
                    let name = cel.name(i, id, &old.name)?;
                    if !seen.insert(name.to_lowercase()) {
                        return Err("Cel naming rule creates duplicate filenames".into());
                    }
                    rows.push(format!("Cel{id}: {} → {name}", old.name));
                    names.insert(id, name);
                }
                let working = crate::layer_merge::preview(&s, d.current_frame)?;
                for link in &s.scene.cels.links {
                    let (t, c, _) = crate::cels::slot(&working, link.track, link.cell)
                        .ok_or("Cel slot removed")?;
                    crate::cels::editable(
                        &working,
                        crate::layer_properties::Target::Cell {
                            track: t as u16,
                            cell: c as u16,
                        },
                    )?;
                }
                for p in &mut s.scene.cels.definitions {
                    p.name = names[&p.object].clone();
                }
                for link in &s.scene.cels.links {
                    let n = s
                        .scene
                        .catalog
                        .tracks
                        .index(link.track, s.animation.as_ref().unwrap().tracks.len())
                        .ok_or("Track removed")?;
                    let c = s
                        .scene
                        .catalog
                        .cells
                        .get(&link.track)
                        .cloned()
                        .unwrap_or_default()
                        .index(
                            link.cell,
                            s.animation.as_ref().unwrap().tracks[n].cells.len(),
                        )
                        .ok_or("Cel removed")?;
                    s.animation.as_mut().unwrap().tracks[n].cells[c].name =
                        names[&link.source].clone();
                }
                rename_shot_cells(&mut s.scene.shots, &s.scene.cels, &names);
                for entry in &mut s.scene.timelines.entries {
                    if let Some(a) = &mut entry.saved {
                        rename_shot_cells(&mut a.shots, &s.scene.cels, &names);
                    }
                }
            }
            if shots {
                if s.scene.shots.entries.is_empty() {
                    return Err("No shots in this timeline to rename".into());
                }
                let mut order = s
                    .scene
                    .shots
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (b.shot.id, i))
                    .collect::<Vec<_>>();
                order.sort_unstable();
                let mut seen = BTreeSet::new();
                for (i, (id, index)) in order.into_iter().enumerate() {
                    crate::book_io::check_cancel(cancel)?;
                    let old = &s.scene.shots.entries[index].shot;
                    let name = shot.name(i, id, &old.name)?;
                    if !seen.insert(name.to_lowercase()) {
                        return Err("Shot naming rule creates duplicate filenames".into());
                    }
                    rows.push(format!("Shot{id}: {} → {name}", old.name));
                    s.scene.shots.entries[index].shot.name = name;
                }
            }
            (
                s,
                "Rename common cel sources and/or active-timeline shots as reviewed".into(),
            )
        }
    };
    crate::book_io::check_cancel(cancel)?;
    crate::layer_groups::finish(d, &state)?;
    Ok(Prepared {
        state,
        id: d.id,
        revision: d.revision,
        rows,
        summary,
    })
}

#[cfg(test)]
#[path = "animation_finish_tests.rs"]
mod tests;
