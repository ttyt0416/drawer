//! Timeline arrangements share the document's physical tracks, cel slots and editable sources.
//! Only time placement, keys, labels, audio and shot annotations are banked.
use crate::{
    animation::{Audio, Exposure, Fps, Timeline},
    document::Document,
    layer_search::Key,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub track: u64,
    pub end: u32,
    pub exposures: Vec<(u32, u64)>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrangement {
    pub fps: Fps,
    pub frames: u32,
    pub tracks: Vec<Pattern>,
    pub labels: Vec<crate::cels::Label>,
    pub output: crate::animation_output::Data,
    pub keys: crate::keyframes::Data,
    pub timing: crate::timeline_timing::Data,
    pub shots: crate::shot_exchange::Data,
    pub audio: Vec<Audio>,
    pub audio_hashes: Vec<[u8; 32]>,
}
impl Arrangement {
    fn capture(s: &DocumentState) -> Result<Self, String> {
        let a = s.animation.as_ref().ok_or("Create a timeline first")?;
        Ok(Self {
            fps: a.fps,
            frames: a.frames,
            tracks: a
                .tracks
                .iter()
                .enumerate()
                .map(|(n, t)| {
                    let track = s.scene.catalog.tracks.id(n);
                    Pattern {
                        track,
                        end: t.end,
                        exposures: t
                            .exposures
                            .iter()
                            .map(|e| (e.frame, s.scene.catalog.cell_id(track, e.cell)))
                            .collect(),
                    }
                })
                .collect(),
            labels: s.scene.cels.labels.clone(),
            output: s.scene.animation_output.clone(),
            keys: *s.scene.keyframes.clone(),
            timing: *s.scene.timing.clone(),
            shots: s.scene.shots.clone(),
            audio: a.audio.clone(),
            audio_hashes: a
                .audio
                .iter()
                .map(|a| crate::timeline_timing::pcm_hash(&a.samples))
                .collect(),
        })
    }
    fn valid(&self, missing: bool) -> bool {
        let audio = Timeline {
            fps: self.fps,
            frames: self.frames,
            tracks: vec![],
            audio: self.audio.clone(),
        };
        (if missing {
            audio.metadata_valid()
        } else {
            audio.valid()
        }) && self.tracks.len() <= 32
            && self
                .tracks
                .iter()
                .map(|t| t.track)
                .collect::<BTreeSet<_>>()
                .len()
                == self.tracks.len()
            && self.tracks.iter().all(|t| {
                t.track > 0
                    && t.end <= self.frames
                    && t.exposures.len() <= 10000
                    && t.exposures.windows(2).all(|p| p[0].0 < p[1].0)
                    && t.exposures.iter().all(|(f, c)| *f < t.end && *c > 0)
            })
            && self.tracks.iter().map(|t| t.exposures.len()).sum::<usize>() <= 320000
            && self.output.valid()
            && self.keys.valid()
            && self
                .keys
                .tracks
                .iter()
                .flat_map(|t| &t.keys)
                .all(|k| k.frame < self.frames)
            && self.timing.valid(missing)
            && self.shots.valid()
            && self.audio_hashes.len() == self.audio.len()
            && self.labels.len() <= 1024
            && self
                .labels
                .iter()
                .map(|l| l.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.labels.len()
            && self.labels.iter().all(|l| {
                l.id > 0
                    && !l.name.trim().is_empty()
                    && l.name.len() <= 256
                    && !l.name.chars().any(char::is_control)
                    && l.frame < 10000
                    && (!self.timing.labels_linked || l.frame < self.frames)
            })
            && self.timing.playback.is_none_or(|p| p.end <= self.frames)
            && self.timing.clips.iter().all(|c| {
                self.tracks
                    .iter()
                    .find(|t| t.track == c.track)
                    .is_some_and(|t| c.end <= t.end)
            })
            && (missing
                || self.timing.audio.iter().all(|c| {
                    self.audio.get(c.index).is_some_and(|a| {
                        c.end > a.start && crate::timeline_timing::matches(&self.timing, c, a)
                    })
                }))
    }
    fn payload_bytes(&self) -> usize {
        self.audio
            .iter()
            .map(|a| a.samples.len() * 2)
            .sum::<usize>()
            + self.timing.payload_bytes()
    }
    fn bytes(&self) -> usize {
        256 + self.payload_bytes()
            + self.output.bytes()
            + self.keys.bytes()
            + self
                .timing
                .bytes()
                .saturating_sub(self.timing.payload_bytes())
            + self.shots.bytes()
            + self
                .tracks
                .iter()
                .map(|t| 32 + t.exposures.len() * 16)
                .sum::<usize>()
            + self.labels.iter().map(|l| 32 + l.name.len()).sum::<usize>()
            + self.audio.iter().map(|a| 96 + a.name.len()).sum::<usize>()
    }
    fn apply(&self, s: &mut DocumentState) -> Result<(), String> {
        let a = s.animation.as_mut().ok_or("Timeline removed")?;
        a.fps = self.fps;
        a.frames = self.frames;
        a.audio = self.audio.clone();
        for (n, t) in a.tracks.iter_mut().enumerate() {
            let id = s.scene.catalog.tracks.id(n);
            t.exposures.clear();
            t.end = self.frames;
            if let Some(p) = self.tracks.iter().find(|p| p.track == id) {
                t.end = p.end;
                for &(frame, cell) in &p.exposures {
                    let index = s
                        .scene
                        .catalog
                        .cells
                        .get(&id)
                        .cloned()
                        .unwrap_or_default()
                        .index(cell, t.cells.len())
                        .ok_or("Saved timeline refers to a removed cel slot")?;
                    t.exposures.push(Exposure { frame, cell: index });
                }
            }
        }
        s.scene.cels.labels = self.labels.clone();
        s.scene.animation_output = self.output.clone();
        *s.scene.keyframes = self.keys.clone();
        *s.scene.timing = self.timing.clone();
        s.scene.shots = self.shots.clone();
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub drop_frame: bool,
    pub saved: Option<Arrangement>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub active: u64,
    pub entries: Vec<Entry>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            active: 0,
            entries: vec![],
        }
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self, missing: bool) -> bool {
        if self.is_default() {
            return true;
        }
        self.next > 1
            && self.active > 0
            && !self.entries.is_empty()
            && self.entries.len() <= 32
            && self
                .entries
                .iter()
                .map(|p| p.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
            && self.entries.iter().filter(|p| p.id == self.active).count() == 1
            && self.entries.iter().all(|p| {
                p.id > 0
                    && p.id < self.next
                    && name(&p.name)
                    && p.saved.is_none() == (p.id == self.active)
                    && p.saved.as_ref().is_none_or(|a| {
                        a.valid(missing) && (!p.drop_frame || drop_rate(a.fps).is_some())
                    })
            })
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            64 + self
                .entries
                .iter()
                .map(|p| 64 + p.name.len() + p.saved.as_ref().map_or(0, Arrangement::bytes))
                .sum::<usize>()
        }
    }
    pub fn payload_bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|p| p.saved.as_ref())
            .map(Arrangement::payload_bytes)
            .sum()
    }
    pub fn active_entry(&self) -> Option<&Entry> {
        self.entries.iter().find(|p| p.id == self.active)
    }
}
fn name(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn slot(s: &DocumentState, track: u64, cell: u64) -> Option<(usize, usize)> {
    let a = s.animation.as_ref()?;
    let t = s.scene.catalog.tracks.index(track, a.tracks.len())?;
    Some((
        t,
        s.scene
            .catalog
            .cells
            .get(&track)
            .cloned()
            .unwrap_or_default()
            .index(cell, a.tracks[t].cells.len())?,
    ))
}
fn keys(s: &DocumentState) -> BTreeSet<Key> {
    use crate::layer_properties::Target;
    let mut out = BTreeSet::new();
    for n in 0..s.layers.len() {
        out.insert(Key::Layer(Target::Raster(s.scene.layer_ids.id(n))));
    }
    for n in 0..s.scene.paper.folders.len() {
        out.insert(Key::Layer(Target::Folder(n as u16)));
    }
    for l in &s.scene.lines.layers {
        out.insert(Key::Layer(Target::Vector(l.id)));
    }
    for n in 0..s.scene.objects.len() {
        out.insert(Key::Object(s.scene.catalog.objects.id(n)));
    }
    for n in 0..s.scene.worlds.len() {
        out.insert(Key::World(s.scene.catalog.worlds.id(n)));
    }
    for l in &s.scene.gradients.layers {
        out.insert(Key::Gradient(l.id));
    }
    for l in &s.scene.selections.saved {
        out.insert(Key::Selection(l.id));
    }
    if let Some(a) = &s.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let id = s.scene.catalog.tracks.id(n);
            out.insert(Key::Track(id));
            for c in 0..t.cells.len() {
                out.insert(Key::Cell {
                    track: id,
                    cell: s.scene.catalog.cell_id(id, c),
                });
            }
        }
    }
    out
}
fn references(s: &DocumentState, a: &Arrangement, live: &BTreeSet<Key>) -> bool {
    a.tracks.iter().all(|p| {
        live.contains(&Key::Track(p.track))
            && p.exposures
                .iter()
                .all(|(_, c)| slot(s, p.track, *c).is_some())
    }) && a.keys.tracks.iter().all(|t| match t.owner {
        crate::keyframes::Owner::Camera => true,
        crate::keyframes::Owner::Layer(k) => live.contains(&k),
    }) && a.labels.iter().all(|l| l.id < s.scene.cels.next_label)
        && a.shots.entries.iter().all(|b| {
            live.contains(&Key::Track(b.track))
                && b.cells.iter().all(|c| slot(s, b.track, *c).is_some())
        })
}
pub fn valid_state(s: &DocumentState) -> bool {
    let data = &s.scene.timelines;
    if !data.valid(false) {
        return false;
    }
    if data.is_default() {
        return true;
    }
    let Some(a) = &s.animation else {
        return false;
    };
    let live = keys(s);
    data.active_entry()
        .is_some_and(|p| !p.drop_frame || drop_rate(a.fps).is_some())
        && data
            .entries
            .iter()
            .filter_map(|p| p.saved.as_ref())
            .all(|a| references(s, a, &live))
}
pub fn valid_document(d: &Document) -> bool {
    if d.scene.timelines.is_default() {
        return true;
    }
    // Only metadata/topology is inspected; source pixels and native cel payloads are not recopied.
    let data = &d.scene.timelines;
    if !data.valid(false) {
        return false;
    }
    let Some(a) = &d.animation else {
        return false;
    };
    let live = crate::layer_groups::live(d).map(|m| m.into_keys().collect::<BTreeSet<_>>());
    data.active_entry()
        .is_some_and(|p| !p.drop_frame || drop_rate(a.fps).is_some())
        && live.is_ok_and(|live| {
            data.entries
                .iter()
                .filter_map(|p| p.saved.as_ref())
                .all(|p| {
                    p.tracks.iter().all(|t| {
                        live.contains(&Key::Track(t.track))
                            && t.exposures
                                .iter()
                                .all(|(_, c)| crate::cels::slot(d, t.track, *c).is_some())
                    }) && p.keys.tracks.iter().all(|t| match t.owner {
                        crate::keyframes::Owner::Camera => true,
                        crate::keyframes::Owner::Layer(k) => live.contains(&k),
                    }) && p.labels.iter().all(|l| l.id < d.scene.cels.next_label)
                        && p.shots.entries.iter().all(|b| {
                            b.cells
                                .iter()
                                .all(|c| crate::cels::slot(d, b.track, *c).is_some())
                        })
                })
        })
}
pub fn hashes(d: &Document) -> bool {
    d.scene
        .timelines
        .entries
        .iter()
        .filter_map(|p| p.saved.as_ref())
        .all(|a| {
            a.audio
                .iter()
                .zip(&a.audio_hashes)
                .all(|(s, h)| crate::timeline_timing::pcm_hash(&s.samples) == *h)
                && a.timing
                    .originals
                    .iter()
                    .all(|s| crate::timeline_timing::pcm_hash(&s.samples) == s.hash)
        })
}
/// Selecting an existing arrangement changes time metadata, not any protected source content.
pub(crate) fn switch_only(d: &Document, s: &DocumentState) -> bool {
    if d.scene.timelines.active == 0
        || s.scene.timelines.active == 0
        || d.scene.timelines.active == s.scene.timelines.active
        || !valid_state(s)
    {
        return false;
    }
    if d.width != s.width
        || d.height != s.height
        || d.dpi != s.dpi
        || d.color != s.color
        || d.layers != s.layers
    {
        return false;
    }
    let (Some(a), Some(b)) = (&d.animation, &s.animation) else {
        return false;
    };
    if a.tracks.len() != b.tracks.len()
        || a.tracks
            .iter()
            .zip(&b.tracks)
            .any(|(a, b)| a.name != b.name || a.cells != b.cells)
    {
        return false;
    }
    let clear = |s: &mut crate::scene::Scene| {
        *s.timelines = Default::default();
        *s.timing = Default::default();
        *s.keyframes = Default::default();
        s.shots = Default::default();
        s.animation_output = Default::default();
        s.cels.labels.clear();
    };
    let mut a = d.scene.clone();
    let mut b = s.scene.clone();
    clear(&mut a);
    clear(&mut b);
    a == b
}
/// Unassigned source copies for one new inactive arrangement do not edit animated originals.
pub(crate) fn inactive_copy_only(d: &Document, s: &DocumentState) -> bool {
    use crate::layer_properties::Target;
    let old = &d.scene.timelines;
    let new = &s.scene.timelines;
    let active = if old.is_default() { 1 } else { old.active };
    let count = if old.is_default() {
        1
    } else {
        old.entries.len()
    };
    if new.active != active
        || new.entries.len() != count + 1
        || !valid_state(s)
        || (!old.is_default() && old.entries.iter().any(|e| !new.entries.contains(e)))
    {
        return false;
    }
    if d.width != s.width
        || d.height != s.height
        || d.dpi != s.dpi
        || d.color != s.color
        || d.layers != s.layers
    {
        return false;
    }
    let (Some(a), Some(b)) = (&d.animation, &s.animation) else {
        return false;
    };
    if a.fps != b.fps
        || a.frames != b.frames
        || a.audio != b.audio
        || a.tracks.len() != b.tracks.len()
    {
        return false;
    }
    let mut before = d.scene.clone();
    let mut after = s.scene.clone();
    let mut added = BTreeSet::new();
    let mut targets = BTreeSet::new();
    if before.layer_ids.initialize(d.layers.len()).is_err()
        || before.catalog.tracks.initialize(a.tracks.len()).is_err()
        || before
            .catalog
            .objects
            .initialize(before.objects.len())
            .is_err()
        || before
            .catalog
            .worlds
            .initialize(before.worlds.len())
            .is_err()
    {
        return false;
    }
    for (n, (a, b)) in a.tracks.iter().zip(&b.tracks).enumerate() {
        if a.name != b.name
            || a.end != b.end
            || a.exposures != b.exposures
            || b.cells.len() < a.cells.len()
            || a.cells != b.cells[..a.cells.len()]
        {
            return false;
        }
        let track = before.catalog.tracks.id(n);
        let original = before.catalog.cells.entry(track).or_default();
        if original.initialize(a.cells.len()).is_err() {
            return false;
        }
        let Some(ids) = after.catalog.cells.get_mut(&track) else {
            return false;
        };
        if ids.values.len() != b.cells.len() || ids.values[..a.cells.len()] != original.values {
            return false;
        }
        for c in a.cells.len()..b.cells.len() {
            added.insert(Key::Cell {
                track,
                cell: ids.id(c),
            });
            targets.insert(Target::Cell {
                track: n as u16,
                cell: c as u16,
            });
        }
        *ids = original.clone();
    }
    if after.cels.definitions.len() < before.cels.definitions.len()
        || after.cels.definitions[..before.cels.definitions.len()] != before.cels.definitions
    {
        return false;
    }
    after
        .cels
        .definitions
        .truncate(before.cels.definitions.len());
    after.cels.next = before.cels.next;
    after.cels.links.retain(|l| {
        !added.contains(&Key::Cell {
            track: l.track,
            cell: l.cell,
        })
    });
    after.catalog.cell_info.retain(|track, m| {
        m.retain(|cell, _| {
            !added.contains(&Key::Cell {
                track: *track,
                cell: *cell,
            })
        });
        !m.is_empty() || before.catalog.cell_info.contains_key(track)
    });
    after.layer_properties.cells.retain(|key, _| {
        !targets.contains(&Target::Cell {
            track: (key >> 32) as u16,
            cell: (key & 0xffff_ffff) as u16,
        })
    });
    after.clipping.alpha.retain(|t| !targets.contains(t));
    after
        .file_objects
        .entries
        .retain(|l| !added.contains(&l.target));
    after
        .layer_groups
        .transforms
        .retain(|f| !added.contains(&f.key));
    after.comics.masks.retain(|m| !added.contains(&m.target));
    *before.timelines = Default::default();
    *after.timelines = Default::default();
    before == after
}
pub enum Edit {
    Enable,
    New { name: String, frames: u32, fps: Fps },
    Duplicate { name: String, independent: bool },
    Switch(u64),
    Rename { id: u64, name: String },
    Delete(u64),
    DropFrame(bool),
}
pub struct Prepared {
    pub state: DocumentState,
    pub summary: String,
}
fn initialize(s: &mut DocumentState) -> Result<(), String> {
    let a = s.animation.as_ref().ok_or("Create a timeline first")?;
    s.scene.catalog.tracks.initialize(a.tracks.len())?;
    for (n, t) in a.tracks.iter().enumerate() {
        s.scene
            .catalog
            .cells
            .entry(s.scene.catalog.tracks.id(n))
            .or_default()
            .initialize(t.cells.len())?;
    }
    if s.scene.timelines.is_default() {
        *s.scene.timelines = Data {
            next: 2,
            active: 1,
            entries: vec![Entry {
                id: 1,
                name: "Timeline 1".into(),
                drop_frame: false,
                saved: None,
            }],
        };
    }
    Ok(())
}
fn switch(s: &mut DocumentState, id: u64) -> Result<(), String> {
    if s.scene.timelines.active == id {
        return Ok(());
    }
    let next = s
        .scene
        .timelines
        .entries
        .iter()
        .position(|e| e.id == id)
        .ok_or("Timeline removed")?;
    let outgoing = Arrangement::capture(s)?;
    let old = s.scene.timelines.active;
    let incoming = s.scene.timelines.entries[next]
        .saved
        .take()
        .ok_or("Timeline arrangement missing")?;
    s.scene
        .timelines
        .entries
        .iter_mut()
        .find(|e| e.id == old)
        .unwrap()
        .saved = Some(outgoing);
    s.scene.timelines.active = id;
    incoming.apply(s)
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Timeline arrangement snapshot budget exceeded".into());
    }
    let mut s = d.snapshot();
    initialize(&mut s)?;
    let summary = match edit {
        Edit::Enable => "Manage multiple timelines using the existing arrangement".into(),
        Edit::Switch(id) => {
            switch(&mut s, id)?;
            "Switch the active timeline; shared source layers and cel content are retained".into()
        }
        Edit::Rename { id, name: n } => {
            if !name(&n) {
                return Err("Timeline name limit256 bytes".into());
            }
            s.scene
                .timelines
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Timeline removed")?
                .name = n;
            "Rename timeline".into()
        }
        Edit::Delete(id) => {
            if s.scene.timelines.entries.len() <= 1 {
                return Err("Keep at least one timeline".into());
            }
            if s.scene.timelines.active == id {
                let to = s
                    .scene
                    .timelines
                    .entries
                    .iter()
                    .find(|p| p.id != id)
                    .unwrap()
                    .id;
                switch(&mut s, to)?;
            }
            let old = s.scene.timelines.entries.len();
            s.scene.timelines.entries.retain(|e| e.id != id);
            if old == s.scene.timelines.entries.len() {
                return Err("Timeline removed".into());
            }
            "Delete the arrangement only; shared cel/source definitions remain, and one Undo restores the timeline".into()
        }
        Edit::DropFrame(value) => {
            if value && drop_rate(s.animation.as_ref().unwrap().fps).is_none() {
                return Err("Drop-frame display requires30000/1001 or60000/1001 FPS".into());
            }
            let id = s.scene.timelines.active;
            s.scene
                .timelines
                .entries
                .iter_mut()
                .find(|p| p.id == id)
                .unwrap()
                .drop_frame = value;
            "Change timecode notation only; no frame or cel is deleted".into()
        }
        Edit::New {
            name: n,
            frames,
            fps,
        } => {
            let mut a = Arrangement::capture(&s)?;
            a.frames = frames;
            a.fps = fps;
            a.tracks.iter_mut().for_each(|t| {
                t.end = frames;
                t.exposures.clear();
            });
            a.labels.clear();
            a.output = Default::default();
            a.keys = Default::default();
            a.timing = Default::default();
            a.shots = Default::default();
            a.audio.clear();
            a.audio_hashes.clear();
            add(&mut s, n, a, false)?;
            "Create an empty time arrangement on the common source layers; switch to it to assign cels".into()
        }
        Edit::Duplicate {
            name: n,
            independent,
        } => {
            let mut a = Arrangement::capture(&s)?;
            if independent {
                independent_cells(&mut s, &mut a, budget, cancel)?;
            }
            let drop = s.scene.timelines.active_entry().unwrap().drop_frame;
            add(&mut s, n, a, drop)?;
            format!(
                "Duplicate time placement, keys, labels, audio and shots with {} cel sources; the original time arrangement remains independent",
                if independent { "independent" } else { "shared" }
            )
        }
    };
    reconcile(d, &mut s)?;
    crate::book_io::check_cancel(cancel)?;
    if !valid_state(&s) {
        return Err("Invalid saved timeline arrangement or references".into());
    }
    crate::layer_groups::finish(d, &s)?;
    Ok(Prepared { state: s, summary })
}
fn add(s: &mut DocumentState, n: String, a: Arrangement, drop_frame: bool) -> Result<(), String> {
    if !name(&n) || !a.valid(false) || s.scene.timelines.entries.len() >= 32 {
        return Err(
            "Choose a bounded name,1..10000 frames, valid FPS and at most32 timelines".into(),
        );
    }
    let id = s.scene.timelines.next;
    s.scene.timelines.next = id.checked_add(1).ok_or("Timeline ID exhausted")?;
    s.scene.timelines.entries.push(Entry {
        id,
        name: n,
        drop_frame,
        saved: Some(a),
    });
    Ok(())
}
fn independent_cells(
    s: &mut DocumentState,
    a: &mut Arrangement,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<(), String> {
    use crate::layer_properties::Target;
    let mut used = a
        .tracks
        .iter()
        .flat_map(|t| t.exposures.iter().map(move |(_, c)| (t.track, *c)))
        .collect::<BTreeSet<_>>();
    for t in &a.keys.tracks {
        if let crate::keyframes::Owner::Layer(Key::Cell { track, cell }) = t.owner {
            used.insert((track, cell));
        }
    }
    for b in &a.shots.entries {
        for c in &b.cells {
            used.insert((b.track, *c));
        }
    }
    let original = s.scene.clone();
    let mut map = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for (track, cell) in used {
        crate::book_io::check_cancel(cancel)?;
        let (t, c) = slot(s, track, cell).ok_or("Cel slot removed")?;
        if s.animation
            .as_ref()
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.cells.len())
            .sum::<usize>()
            >= 10000
        {
            return Err("Cel slot limit10000".into());
        }
        let clone = s.animation.as_ref().unwrap().tracks[t].cells[c].clone();
        let new = s.animation.as_ref().unwrap().tracks[t].cells.len();
        s.animation.as_mut().unwrap().tracks[t].cells.push(clone);
        let ids = s.scene.catalog.cells.get_mut(&track).unwrap();
        ids.append(new + 1)?;
        let id = ids.id(new);
        let oldkey = Key::Cell { track, cell };
        let newkey = Key::Cell { track, cell: id };
        map.insert(oldkey, newkey);
        if let Some(p) = original
            .layer_properties
            .cells
            .get(&(((t as u64) << 32) | c as u64))
        {
            s.scene
                .layer_properties
                .cells
                .insert(((t as u64) << 32) | new as u64, *p);
        }
        if original.clipping.alpha.contains(&Target::Cell {
            track: t as u16,
            cell: c as u16,
        }) {
            s.scene.clipping.alpha.insert(Target::Cell {
                track: t as u16,
                cell: new as u16,
            });
        }
        if let Some(info) = original
            .catalog
            .cell_info
            .get(&track)
            .and_then(|m| m.get(&cell))
        {
            s.scene
                .catalog
                .cell_info
                .entry(track)
                .or_default()
                .insert(id, info.clone());
        }
        crate::file_objects::duplicate(s, oldkey, newkey);
        if let Some(source) = original.cels.source(track, cell) {
            let source = if let Some(id) = sources.get(&source) {
                *id
            } else {
                let p = s.scene.cels.get(source).ok_or("Cel source removed")?;
                let child = p.open()?;
                let n = p.name.clone();
                let id = crate::cels::insert(s, &child, &n, budget)?;
                sources.insert(source, id);
                id
            };
            s.scene.cels.links.push(crate::cels::Link {
                track,
                cell: id,
                source,
            });
        }
        for f in original
            .layer_groups
            .transforms
            .iter()
            .filter(|f| f.key == oldkey)
        {
            let mut f = f.clone();
            f.key = newkey;
            s.scene.layer_groups.transforms.push(f);
        }
        for mask in original.comics.masks.iter().filter(|m| m.target == oldkey) {
            let mut m = mask.clone();
            m.target = newkey;
            s.scene.comics.masks.push(m);
        }
    }
    for t in &mut a.tracks {
        for (_, c) in &mut t.exposures {
            if let Some(Key::Cell { cell, .. }) = map.get(&Key::Cell {
                track: t.track,
                cell: *c,
            }) {
                *c = *cell;
            }
        }
    }
    for t in &mut a.keys.tracks {
        if let crate::keyframes::Owner::Layer(k) = &mut t.owner
            && let Some(new) = map.get(k)
        {
            *k = *new;
        }
    }
    for b in &mut a.shots.entries {
        for c in &mut b.cells {
            if let Some(Key::Cell { cell, .. }) = map.get(&Key::Cell {
                track: b.track,
                cell: *c,
            }) {
                *c = *cell;
            }
        }
    }
    Ok(())
}
/// Global source topology changes affect every arrangement; time edits affect the active one only.
pub(crate) fn reconcile(before: &Document, s: &mut DocumentState) -> Result<(), String> {
    if s.scene.timelines.is_default() {
        return Ok(());
    }
    if s.animation.is_none() {
        return Err("Delete saved timelines before removing the animation".into());
    }
    let live = keys(s);
    let mut moves = BTreeMap::new();
    for old in &before.scene.cels.links {
        let key = Key::Cell {
            track: old.track,
            cell: old.cell,
        };
        if !live.contains(&key) {
            let candidates = s
                .scene
                .cels
                .links
                .iter()
                .filter(|n| {
                    n.source == old.source
                        && !before
                            .scene
                            .cels
                            .links
                            .iter()
                            .any(|b| b.track == n.track && b.cell == n.cell)
                })
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                moves.insert(
                    key,
                    Key::Cell {
                        track: candidates[0].track,
                        cell: candidates[0].cell,
                    },
                );
            }
        }
    }
    for p in &mut s.scene.timelines.entries {
        let Some(a) = &mut p.saved else {
            continue;
        };
        if before.scene.transform != s.scene.transform && a.keys.tracks.iter().any(|t| t.enabled) {
            return Err("Disable animation keys in every saved timeline before transforming shared source coordinates".into());
        }
        let mut migrated = vec![];
        for t in &mut a.tracks {
            t.exposures.retain(|(f, c)| {
                let old = Key::Cell {
                    track: t.track,
                    cell: *c,
                };
                if let Some(Key::Cell { track, cell }) = moves.get(&old) {
                    migrated.push((*track, *f, *cell));
                    false
                } else {
                    live.contains(&old)
                }
            });
        }
        a.tracks.retain(|t| live.contains(&Key::Track(t.track)));
        for (track, frame, cell) in migrated {
            let n = if let Some(n) = a.tracks.iter().position(|t| t.track == track) {
                n
            } else {
                a.tracks.push(Pattern {
                    track,
                    end: a.frames,
                    exposures: vec![],
                });
                a.tracks.len() - 1
            };
            let t = &mut a.tracks[n];
            t.end = t.end.max(frame + 1);
            match t.exposures.binary_search_by_key(&frame, |e| e.0) {
                Ok(n) => t.exposures[n].1 = cell,
                Err(n) => t.exposures.insert(n, (frame, cell)),
            }
        }
        a.keys.tracks.retain_mut(|t| match &mut t.owner {
            crate::keyframes::Owner::Camera => true,
            crate::keyframes::Owner::Layer(k) => {
                if let Some(n) = moves.get(k) {
                    *k = *n;
                }
                live.contains(k)
            }
        });
        if a.keys.get(crate::keyframes::Owner::Camera).is_none() {
            a.keys.camera_output = false;
        }
        a.timing
            .clips
            .retain(|c| a.tracks.iter().any(|t| t.track == c.track));
        crate::clip_edit::prune(&mut a.timing);
        a.shots.entries.retain(|b| {
            live.contains(&Key::Track(b.track))
                && b.cells.iter().all(|c| {
                    live.contains(&Key::Cell {
                        track: b.track,
                        cell: *c,
                    })
                })
        });
    }
    if !valid_state(s) {
        return Err("Saved timeline references/notation are invalid; turn off drop-frame before changing to another FPS".into());
    }
    Ok(())
}
pub(crate) fn write(scene: &crate::scene::Scene, w: &mut impl Write) -> io::Result<()> {
    for a in scene
        .timelines
        .entries
        .iter()
        .filter_map(|p| p.saved.as_ref())
    {
        for values in a
            .audio
            .iter()
            .map(|a| &a.samples)
            .chain(a.timing.originals.iter().map(|s| &s.samples))
        {
            for chunk in values.chunks(4096) {
                let mut b = [0; 8192];
                for (v, out) in chunk.iter().zip(b.as_chunks_mut::<2>().0) {
                    out.copy_from_slice(&v.to_le_bytes());
                }
                w.write_all(&b[..chunk.len() * 2])?;
            }
        }
    }
    Ok(())
}
fn pcm(r: &mut impl Read, count: usize, budget: usize) -> io::Result<Arc<Vec<i16>>> {
    if count.saturating_mul(2) > budget {
        return Err(io::Error::other("Saved timeline PCM budget"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(io::Error::other)?;
    for _ in 0..count {
        let mut b = [0; 2];
        r.read_exact(&mut b)?;
        out.push(i16::from_le_bytes(b));
    }
    Ok(Arc::new(out))
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> io::Result<()> {
    for n in 0..d.scene.timelines.entries.len() {
        let Some(a) = d.scene.timelines.entries[n].saved.as_ref() else {
            continue;
        };
        let counts = a
            .audio
            .iter()
            .map(|a| a.sample_count as usize)
            .collect::<Vec<_>>();
        let originals = a
            .timing
            .originals
            .iter()
            .map(|s| s.count as usize)
            .collect::<Vec<_>>();
        for (i, count) in counts.into_iter().enumerate() {
            let samples = pcm(r, count, budget.saturating_sub(d.memory_bytes()))?;
            d.scene.timelines.entries[n].saved.as_mut().unwrap().audio[i].samples = samples;
        }
        for (i, count) in originals.into_iter().enumerate() {
            let samples = pcm(r, count, budget.saturating_sub(d.memory_bytes()))?;
            d.scene.timelines.entries[n]
                .saved
                .as_mut()
                .unwrap()
                .timing
                .originals[i]
                .samples = samples;
        }
    }
    if !hashes(d) {
        return Err(io::Error::other("Saved timeline PCM hash mismatch"));
    }
    Ok(())
}
pub fn drop_rate(fps: Fps) -> Option<u32> {
    [30, 60]
        .into_iter()
        .find(|r| fps.numerator as u64 * 1001 == *r as u64 * 1000 * fps.denominator as u64)
}
pub fn timecode(frame: u64, fps: Fps, drop_frame: bool) -> Result<String, String> {
    if !fps.valid() {
        return Err("Invalid timecode FPS".into());
    }
    let nominal = if drop_frame {
        drop_rate(fps).ok_or("Drop-frame needs29.97 or59.94fps")?
    } else {
        fps.value().round().max(1.) as u32
    } as u64;
    let mut number = frame;
    if drop_frame {
        let drop = nominal / 15;
        let ten = nominal * 600 - drop * 9;
        let minute = nominal * 60 - drop;
        let blocks = number / ten;
        let rest = number % ten;
        number = number
            .checked_add(drop * 9 * blocks + drop * ((rest.saturating_sub(drop)) / minute))
            .ok_or("Timecode overflow")?;
    }
    Ok(format!(
        "{:02}:{:02}:{:02}{}{:02}",
        number / (nominal * 3600),
        number / (nominal * 60) % 60,
        number / nominal % 60,
        if drop_frame { ";" } else { ":" },
        number % nominal
    ))
}
/// Real source-sharing, persistence, cancellation and output checks for the installed application.
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("timelines-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        use crate::{
            document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
            keyframes::{self, Curve, Owner, Pose},
            timeline_editor::{self as editor, Edit as CelEdit},
        };
        let check = |ok: bool, message: &str| if ok { Ok(()) } else { Err(message.to_owned()) };
        let cancel = AtomicBool::new(false);
        let mut d = Document::create(
            CanvasSpec {
                width: 4,
                height: 3,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        d.commit_state(editor::create(
            &d,
            10,
            Fps {
                numerator: 30000,
                denominator: 1001,
            },
        )?)?;
        d.commit_state(editor::prepare(&d, CelEdit::NewTrack("Track".into()), B)?.state)?;
        let track = d.scene.catalog.tracks.id(0);
        let source = d.scene.cels.definitions[0].object;
        let mut child = d.scene.cels.get(source).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [255, 0, 0, 255]);
        d.commit_state(crate::cels::replace(&d, source, &child, B)?)?;
        d.commit_state(
            editor::prepare(
                &d,
                CelEdit::Expose {
                    track,
                    source,
                    frame: 4,
                },
                B,
            )?
            .state,
        )?;
        d.current_frame = 5;
        d.commit_state(
            editor::prepare(
                &d,
                CelEdit::NewCel {
                    track,
                    name: "Blue".into(),
                },
                B,
            )?
            .state,
        )?;
        let blue = d.scene.cels.definitions.last().unwrap().object;
        let mut child = d.scene.cels.get(blue).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [0, 0, 255, 255]);
        d.commit_state(crate::cels::replace(&d, blue, &child, B)?)?;
        let mut state = d.snapshot();
        state.animation.as_mut().unwrap().tracks[0].cells[0].offset = [1, 0];
        state.scene.layer_properties.cells.insert(
            0,
            crate::layer_properties::Properties {
                opacity: 73,
                ..Default::default()
            },
        );
        d.commit_state(state)?;
        d.commit_state(prepare(&d, Edit::Enable, B, &cancel)?.state)?;
        let before = d.animation.clone();
        d.commit_state(prepare(&d, Edit::DropFrame(true), B, &cancel)?.state)?;
        check(
            d.animation == before,
            "Timecode notation deleted or changed frames",
        )?;
        let original = d.animation.clone();
        d.commit_state(
            prepare(
                &d,
                Edit::Duplicate {
                    name: "Shared".into(),
                    independent: false,
                },
                B,
                &cancel,
            )?
            .state,
        )?;
        check(
            d.scene.cels.definitions.len() == 2,
            "Shared timeline duplicated source pixels",
        )?;
        d.commit_state(prepare(&d, Edit::Switch(2), B, &cancel)?.state)?;
        d.commit_state(
            editor::prepare(
                &d,
                CelEdit::Expose {
                    track,
                    source: blue,
                    frame: 2,
                },
                B,
            )?
            .state,
        )?;
        d.commit_state(
            editor::prepare(
                &d,
                CelEdit::Label {
                    name: "Local label".into(),
                    frame: 3,
                },
                B,
            )?
            .state,
        )?;
        let mut s = d.snapshot();
        s.animation.as_mut().unwrap().audio.push(Audio {
            name: "PCM".into(),
            start: 0,
            rate: 8000,
            channels: 1,
            sample_count: 8,
            samples: Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]),
        });
        crate::timeline_timing::populate(&mut s)?;
        d.commit_state(s)?;
        d.current_frame = 0;
        d.commit_state(keyframes::prepare(
            &d,
            Owner::Layer(Key::Track(track)),
            keyframes::Edit::Enable(true),
        )?)?;
        d.commit_state(keyframes::prepare(
            &d,
            Owner::Layer(Key::Track(track)),
            keyframes::Edit::Put {
                key: keyframes::Keyframe {
                    frame: 2,
                    pose: Pose {
                        offset: [1., 0.],
                        ..Default::default()
                    },
                    curve: Curve::Linear,
                },
                replace: true,
            },
        )?)?;
        d.commit_state(crate::animation_output::prepare_state(
            &d,
            crate::scene::Rect {
                min: [0., 0.],
                max: [3., 2.],
            },
            10,
        )?)?;
        let altered = d.animation.clone();
        let keys = d.scene.keyframes.clone();
        let output = d.scene.animation_output.clone();
        d.commit_state(prepare(&d, Edit::Switch(1), B, &cancel)?.state)?;
        check(
            d.animation == original
                && d.scene.cels.labels.is_empty()
                && d.scene.keyframes.is_default()
                && d.scene.animation_output.is_default(),
            "Independent time arrangement was overwritten",
        )?;
        check(
            d.edit_pixel(0, 3, 2, [30, 40, 50, 255]),
            "Common source layer edit failed",
        )?;
        let mut child = d.scene.cels.get(source).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [0, 255, 0, 255]);
        d.commit_state(crate::cels::replace(&d, source, &child, B)?)?;
        d.commit_state(prepare(&d, Edit::Switch(2), B, &cancel)?.state)?;
        check(
            d.animation
                == altered.as_ref().map(|a| {
                    let mut a = a.clone();
                    a.tracks[0].cells[0].pixels = d.animation.as_ref().unwrap().tracks[0].cells[0]
                        .pixels
                        .clone();
                    a
                })
                && d.scene.keyframes == keys
                && d.scene.animation_output == output
                && d.layers[0].pixels[11] == [30, 40, 50, 255],
            "Shared source or scoped key/audio/output layout lost",
        )?;
        let child = d.scene.cels.get(source).unwrap().document().unwrap();
        check(
            child.layers[0].pixels[0] == [0, 255, 0, 255],
            "Shared cel update not visible",
        )?;
        d.commit_state(
            prepare(
                &d,
                Edit::Duplicate {
                    name: "Independent".into(),
                    independent: true,
                },
                B,
                &cancel,
            )?
            .state,
        )?;
        check(
            d.scene.cels.definitions.len() == 4,
            "Independent timeline did not clone each distinct cel once",
        )?;
        d.commit_state(prepare(&d, Edit::Switch(3), B, &cancel)?.state)?;
        let a = d.animation.as_ref().unwrap();
        let cell = a.tracks[0].exposures[0].cell;
        let slot = d.scene.catalog.cell_id(track, cell);
        let independent = d.scene.cels.source(track, slot).unwrap();
        check(
            independent != source
                && a.tracks[0].cells[cell].offset == [1, 0]
                && d.scene
                    .layer_properties
                    .cells
                    .get(&(cell as u64))
                    .is_some_and(|p| p.opacity == 73),
            "Independent cel placement/properties were not preserved",
        )?;
        d.keyframe_content = Some(Owner::Layer(Key::Track(track)));
        let mut child = d.scene.cels.get(independent).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [255, 0, 255, 255]);
        d.commit_state(crate::cels::replace(&d, independent, &child, B)?)?;
        d.keyframe_content = None;
        d.commit_state(prepare(&d, Edit::Switch(2), B, &cancel)?.state)?;
        check(
            d.scene.cels.get(source).unwrap().document().unwrap().layers[0].pixels[0]
                == [0, 255, 0, 255],
            "Independent cel edit changed shared timeline",
        )?;
        let prior = d.snapshot();
        let dirty = d.dirty;
        check(
            prepare(
                &d,
                Edit::New {
                    name: "Bad".into(),
                    frames: 0,
                    fps: Default::default(),
                },
                B,
                &cancel,
            )
            .is_err()
                && d.snapshot() == prior
                && d.dirty == dirty,
            "Invalid arrangement modified document",
        )?;
        check(
            prepare(
                &d,
                Edit::Duplicate {
                    name: "cancel".into(),
                    independent: true,
                },
                B,
                &AtomicBool::new(true),
            )
            .is_err()
                && d.snapshot() == prior,
            "Cancelled arrangement modified document",
        )?;
        check(
            prepare(&d, Edit::Enable, 1, &cancel).is_err(),
            "Timeline snapshot budget ignored",
        )?;
        d.commit_state(prepare(&d, Edit::Switch(1), B, &cancel)?.state)?;
        d.undo();
        check(d.snapshot() == prior, "Timeline switch Undo failed")?;
        d.redo();
        let path = dir.join("bank.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let mut reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == d.snapshot() && !reopened.dirty,
            "Native multi-timeline/audio round trip failed",
        )?;
        reopened.commit_state(prepare(&reopened, Edit::Switch(3), B, &cancel)?.state)?;
        check(
            reopened.animation.as_ref().unwrap().audio[0]
                .samples
                .as_ref()
                == &[1, 2, 3, 4, 5, 6, 7, 8],
            "Inactive timeline PCM was not restored",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&103u32.to_le_bytes());
        let old = dir.join("old.drawer");
        std::fs::write(&old, &bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&old, B).is_err(),
            "Old native version accepted multiple timelines",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(dir.join("bad.drawer"), bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&dir.join("bad.drawer"), B).is_err(),
            "Corrupt inactive PCM was accepted",
        )?;
        d.commit_state(editor::prepare(&d, CelEdit::DeleteCel(blue), B)?.state)?;
        check(
            valid_document(&d),
            "Deleting a common cel left stale inactive references",
        )?;
        let ids = d
            .scene
            .timelines
            .entries
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
        for id in ids {
            d.commit_state(prepare(&d, Edit::Switch(id), B, &cancel)?.state)?;
            check(
                crate::cels::valid_document(&d) && crate::keyframes::valid_document(&d),
                "Saved topology cleanup failed",
            )?;
        }
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
#[path = "timelines_tests.rs"]
mod tests;
