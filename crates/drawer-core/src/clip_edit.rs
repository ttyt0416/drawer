//! Persistent clip relationships and transactional, bounded clip time edits.
use crate::{
    document::Document,
    keyframes::Owner,
    layer_search::Key,
    sizing::DocumentState,
    timeline_timing::{CelClip, Data as Timing},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Node {
    Clip(u64),
    Folder(u64),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub id: u64,
    pub name: String,
    pub children: Vec<Node>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub id: u64,
    pub clip: Option<u64>,
    pub name: String,
    pub first: u32,
    pub end: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub auto_expand: BTreeSet<u64>,
    pub labels: Vec<Label>,
    pub folders: Vec<Folder>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            auto_expand: BTreeSet::new(),
            labels: vec![],
            folders: vec![],
        }
    }
}
fn name(n: &str) -> bool {
    !n.trim().is_empty() && n.len() <= 256 && !n.chars().any(char::is_control)
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            64 + self.auto_expand.len() * 16
                + self.labels.iter().map(|l| 48 + l.name.len()).sum::<usize>()
                + self
                    .folders
                    .iter()
                    .map(|f| 48 + f.name.len() + f.children.len() * 16)
                    .sum::<usize>()
        }
    }
    fn id(&mut self) -> Result<u64, String> {
        let n = self.next;
        self.next = n.checked_add(1).ok_or("Clip relationship ID exhausted")?;
        Ok(n)
    }
    pub fn valid(&self, clips: &[CelClip]) -> bool {
        let ids = self
            .labels
            .iter()
            .map(|l| l.id)
            .chain(self.folders.iter().map(|f| f.id))
            .collect::<Vec<_>>();
        self.next > 0
            && self.labels.len() <= 2048
            && self.folders.len() <= 256
            && ids.iter().all(|n| *n > 0 && *n < self.next)
            && ids.iter().copied().collect::<BTreeSet<_>>().len() == ids.len()
            && self
                .auto_expand
                .iter()
                .all(|n| clips.iter().any(|c| c.id == *n))
            && self.labels.iter().all(|l| {
                name(&l.name)
                    && l.first < l.end
                    && l.end <= 10000
                    && l.clip.is_none_or(|n| clips.iter().any(|c| c.id == n))
            })
            && self.folders.iter().all(|f| {
                name(&f.name)
                    && !f.children.is_empty()
                    && f.children.len() <= 2048
                    && f.children.iter().copied().collect::<BTreeSet<_>>().len() == f.children.len()
                    && self.range(Node::Folder(f.id), clips).is_some()
            })
    }
    pub fn range(&self, node: Node, clips: &[CelClip]) -> Option<(u32, u32)> {
        type Bounds = (u32, u32, usize);
        fn visit(
            d: &Data,
            n: Node,
            clips: &[CelClip],
            path: &mut BTreeSet<u64>,
            memo: &mut BTreeMap<Node, Bounds>,
        ) -> Option<Bounds> {
            if matches!(n,Node::Folder(id)if path.contains(&id)) {
                return None;
            }
            if let Some(bounds) = memo.get(&n) {
                return (path.len() + bounds.2 <= 16).then_some(*bounds);
            }
            let result = match n {
                Node::Clip(id) => clips
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| (c.first, c.end, 0))?,
                Node::Folder(id) => {
                    if path.len() >= 16 || !path.insert(id) {
                        return None;
                    }
                    let folder = d.folders.iter().find(|f| f.id == id)?;
                    let mut r = (10000, 0, 0);
                    for node in &folder.children {
                        let (a, b, h) = visit(d, *node, clips, path, memo)?;
                        r.0 = r.0.min(a);
                        r.1 = r.1.max(b);
                        r.2 = r.2.max(h);
                    }
                    path.remove(&id);
                    if r.0 >= r.1 {
                        return None;
                    }
                    (r.0, r.1, r.2 + 1)
                }
            };
            memo.insert(n, result);
            Some(result)
        }
        visit(
            self,
            node,
            clips,
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
        )
        .map(|(first, end, _)| (first, end))
    }
    pub fn leaves(&self, node: Node, clips: &[CelClip]) -> Result<Vec<u64>, String> {
        self.range(node, clips)
            .ok_or("Invalid or cyclic clip folder")?;
        fn walk(d: &Data, n: Node, out: &mut BTreeSet<u64>, seen: &mut BTreeSet<u64>) {
            match n {
                Node::Clip(id) => {
                    out.insert(id);
                }
                Node::Folder(id) => {
                    if seen.insert(id) {
                        for c in &d.folders.iter().find(|f| f.id == id).unwrap().children {
                            walk(d, *c, out, seen);
                        }
                    }
                }
            }
        }
        let mut out = BTreeSet::new();
        walk(self, node, &mut out, &mut BTreeSet::new());
        Ok(out.into_iter().collect())
    }
}
fn capture(d: &Document, budget: usize) -> Result<DocumentState, String> {
    let mut s = crate::timeline_timing::enable(d, budget)?;
    let existing = &d.scene.timing.clips;
    s.scene.timing.clips.retain(|c| {
        !existing.iter().any(|old| old.track == c.track)
            || existing.iter().any(|old| old.id == c.id)
    });
    Ok(s)
}
pub fn inventory(d: &Document, budget: usize) -> Result<Timing, String> {
    Ok(*capture(d, budget)?.scene.timing)
}
fn index(s: &DocumentState, id: u64) -> Result<usize, String> {
    s.scene
        .catalog
        .tracks
        .index(
            id,
            s.animation
                .as_ref()
                .ok_or("Choose an animation")?
                .tracks
                .len(),
        )
        .ok_or("Track removed".into())
}
fn check_track(d: &Document, id: u64) -> Result<(), String> {
    let n = d
        .scene
        .catalog
        .tracks
        .index(
            id,
            d.animation
                .as_ref()
                .ok_or("Choose an animation")?
                .tracks
                .len(),
        )
        .ok_or("Track removed")?;
    crate::cels::editable(d, crate::layer_properties::Target::Track(n as u16))?;
    for c in 0..d.animation.as_ref().unwrap().tracks[n].cells.len() {
        crate::cels::editable(
            d,
            crate::layer_properties::Target::Cell {
                track: n as u16,
                cell: c as u16,
            },
        )?;
    }
    Ok(())
}
fn clip(s: &DocumentState, id: u64) -> Result<CelClip, String> {
    s.scene
        .timing
        .clips
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or("Clip removed".into())
}
fn blank(s: &DocumentState, t: usize, c: usize) -> bool {
    let c = &s.animation.as_ref().unwrap().tracks[t].cells[c];
    c.name == "Empty frame" && c.width == 1 && c.height == 1 && c.pixels.as_ref() == &[[0; 4]]
}
fn dense(s: &DocumentState, t: usize) -> Vec<Option<usize>> {
    let a = s.animation.as_ref().unwrap();
    let mut v = crate::timeline_time_edit::dense(s, t);
    v.resize(a.frames as usize, None);
    for c in &mut v {
        if c.is_some_and(|c| blank(s, t, c)) {
            *c = None;
        }
    }
    v
}
fn owner_track(o: Owner) -> Option<u64> {
    match o {
        Owner::Layer(Key::Track(t)) | Owner::Layer(Key::Cell { track: t, .. }) => Some(t),
        _ => None,
    }
}
fn labels_split(time: &mut Timing, id: u64, right: u64, at: u32) -> Result<(), String> {
    if !time.labels_linked {
        return Ok(());
    }
    let mut add = vec![];
    for l in &mut time.edit.labels {
        if l.clip != Some(id) {
            continue;
        }
        if l.first >= at {
            l.clip = Some(right);
        } else if l.end > at {
            let mut r = l.clone();
            r.clip = Some(right);
            r.first = at;
            l.end = at;
            add.push(r);
        }
    }
    for mut l in add {
        l.id = time.edit.id()?;
        time.edit.labels.push(l);
    }
    Ok(())
}
fn split(s: &mut DocumentState, id: u64, at: u32) -> Result<u64, String> {
    let c = clip(s, id)?;
    if !(c.first < at && at < c.end) {
        return Err("Split must be strictly inside the clip".into());
    }
    let right = s.scene.timing.id()?;
    s.scene
        .timing
        .clips
        .iter_mut()
        .find(|c| c.id == id)
        .unwrap()
        .end = at;
    s.scene.timing.clips.push(CelClip {
        id: right,
        first: at,
        ..c
    });
    labels_split(&mut s.scene.timing, id, right, at)?;
    if s.scene.timing.edit.auto_expand.contains(&id) {
        s.scene.timing.edit.auto_expand.insert(right);
    }
    for f in &mut s.scene.timing.edit.folders {
        if let Some(p) = f.children.iter().position(|n| *n == Node::Clip(id)) {
            f.children.insert(p + 1, Node::Clip(right));
        }
    }
    Ok(right)
}
#[derive(Clone)]
struct Piece {
    clip: CelClip,
    values: Vec<Option<u64>>,
    keys: Vec<(Owner, crate::keyframes::Keyframe, bool)>,
    labels: Vec<Label>,
}
#[derive(Clone)]
pub struct Clipboard {
    document: u64,
    timeline: u64,
    first: u32,
    pieces: Vec<Piece>,
}
impl Clipboard {
    pub fn bounds(&self) -> (u32, u32) {
        (
            self.first,
            self.pieces
                .iter()
                .map(|p| p.clip.end)
                .max()
                .unwrap_or(self.first),
        )
    }
}
/// A range clipboard keeps source slots in the same document, including after cutting.
pub fn copy(d: &Document, track: u64, first: u32, end: u32) -> Result<Clipboard, String> {
    let mut s = capture(d, crate::document::DEFAULT_DOCUMENT_BUDGET)?;
    let t = index(&s, track)?;
    let a = s.animation.as_ref().unwrap();
    if first >= end || end > a.frames {
        return Err("Choose a nonempty clip interval".into());
    }
    // Split a frozen copy only; boundary labels are intersected without changing the original.
    for c in s.scene.timing.clips.clone() {
        if c.track == track && c.first < first && first < c.end {
            split(&mut s, c.id, first)?;
        }
    }
    for c in s.scene.timing.clips.clone() {
        if c.track == track && c.first < end && end < c.end {
            split(&mut s, c.id, end)?;
        }
    }
    let values = dense(&s, t);
    let mut pieces = vec![];
    for c in &s.scene.timing.clips {
        if c.track != track || c.first < first || c.end > end {
            continue;
        }
        let keys = s
            .scene
            .keyframes
            .tracks
            .iter()
            .filter(|k| owner_track(k.owner) == Some(track))
            .flat_map(|k| {
                k.keys
                    .iter()
                    .filter(move |k| (c.first..c.end).contains(&k.frame))
                    .map(move |key| (k.owner, *key, k.enabled))
            })
            .collect();
        pieces.push(Piece {
            clip: c.clone(),
            values: values[c.first as usize..c.end as usize]
                .iter()
                .map(|c| c.map(|c| s.scene.catalog.cell_id(track, c)))
                .collect(),
            keys,
            labels: s
                .scene
                .timing
                .edit
                .labels
                .iter()
                .filter(|l| l.clip == Some(c.id))
                .cloned()
                .collect(),
        });
    }
    if pieces.is_empty() {
        return Err("The selection contains no clip".into());
    }
    Ok(Clipboard {
        document: d.id,
        timeline: d.scene.timelines.active,
        first,
        pieces,
    })
}
pub enum Edit {
    Split {
        id: u64,
        at: u32,
    },
    Join(Vec<u64>),
    Bounds {
        id: u64,
        first: u32,
        end: u32,
    },
    Move {
        ids: Vec<u64>,
        at: u32,
        copy: bool,
        replace: bool,
    },
    Delete(Vec<u64>),
    Cut {
        track: u64,
        first: u32,
        end: u32,
    },
    Paste {
        clipboard: Clipboard,
        at: u32,
        replace: bool,
    },
    AutoExpand {
        id: u64,
        on: bool,
    },
    LinkLabels(bool),
    Label {
        clip: u64,
        first: u32,
        end: u32,
        name: String,
    },
    Folder {
        name: String,
        children: Vec<Node>,
    },
    SplitTrack {
        track: u64,
        at: u32,
    },
    MergeTracks {
        from: u64,
        to: u64,
        replace: bool,
    },
}
pub struct Prepared {
    state: DocumentState,
    id: u64,
    revision: u64,
    pub summary: String,
    pub rows: Vec<String>,
}
impl Prepared {
    pub fn memory_bytes(&self) -> usize {
        self.state.memory_bytes()
    }

    pub fn matches(&self, d: &Document) -> bool {
        self.id == d.id && self.revision == d.revision
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if self.id != d.id || self.revision != d.revision {
            return Err("Clip preparation is stale; drawing unchanged".into());
        }
        d.commit_state(self.state)
    }
}
fn remove(s: &mut DocumentState, ids: &[u64]) -> Result<(), String> {
    let mut by = BTreeMap::<usize, Vec<Option<usize>>>::new();
    let cs = ids
        .iter()
        .map(|id| clip(s, *id))
        .collect::<Result<Vec<_>, _>>()?;
    for c in &cs {
        let t = index(s, c.track)?;
        let v = by.entry(t).or_insert_with(|| dense(s, t));
        v[c.first as usize..c.end as usize].fill(None);
    }
    for (t, v) in by {
        crate::timeline_time_edit::encode(s, t, &v)?;
    }
    for k in &mut s.scene.keyframes.tracks {
        if let Some(t) = owner_track(k.owner) {
            let hold = k.at(0);
            k.keys.retain(|k| {
                !cs.iter()
                    .any(|c| c.track == t && (c.first..c.end).contains(&k.frame))
            });
            if k.keys.is_empty() {
                k.enabled = false;
                k.hold = hold;
            }
        }
    }
    s.scene.timing.clips.retain(|c| !ids.contains(&c.id));
    if s.scene.timing.labels_linked {
        s.scene
            .timing
            .edit
            .labels
            .retain(|l| !l.clip.is_some_and(|id| ids.contains(&id)));
    } else {
        for l in &mut s.scene.timing.edit.labels {
            if l.clip.is_some_and(|id| ids.contains(&id)) {
                l.clip = None;
            }
        }
    }
    Ok(())
}
fn insert(
    s: &mut DocumentState,
    pieces: &[Piece],
    base: u32,
    at: u32,
    replace: bool,
    copying: bool,
) -> Result<(), String> {
    let frames = s.animation.as_ref().unwrap().frames;
    let delta = at as i64 - base as i64;
    let map = |f: u32| -> Result<u32, String> {
        let n = f as i64 + delta;
        if !(0..=frames as i64).contains(&n) {
            Err("Clip exceeds the timeline; extend its frame count first".into())
        } else {
            Ok(n as u32)
        }
    };
    let mut by = BTreeMap::<usize, Vec<Option<usize>>>::new();
    for p in pieces {
        let t = index(s, p.clip.track)?;
        let first = map(p.clip.first)?;
        let end = map(p.clip.end)?;
        if first >= end {
            return Err("Empty pasted clip".into());
        }
        let collisions = s
            .scene
            .timing
            .clips
            .iter()
            .filter(|c| c.track == p.clip.track && c.first < end && first < c.end)
            .map(|c| c.id)
            .collect::<Vec<_>>();
        if !collisions.is_empty() {
            if !replace {
                return Err(
                    "Destination clips overlap; choose explicit replacement after reviewing".into(),
                );
            }
            for id in collisions {
                let mut c = clip(s, id)?;
                if c.first < first {
                    let right = split(s, id, first)?;
                    c = clip(s, right)?;
                }
                if c.end > end {
                    split(s, c.id, end)?;
                }
                remove(s, &[c.id])?;
            }
        }
        let v = by.entry(t).or_insert_with(|| dense(s, t));
        if !replace && v[first as usize..end as usize].iter().any(Option::is_some) {
            return Err("Destination exposures collide; review explicit replacement".into());
        }
        let resolved = p
            .values
            .iter()
            .map(|cell| {
                cell.map(|id| {
                    s.scene
                        .catalog
                        .cells
                        .get(&p.clip.track)
                        .cloned()
                        .unwrap_or_default()
                        .index(id, s.animation.as_ref().unwrap().tracks[t].cells.len())
                        .ok_or("Clipboard source slot removed; copy again")
                })
            })
            .map(|n| n.transpose())
            .collect::<Result<Vec<_>, _>>()?;
        v[first as usize..end as usize].clone_from_slice(&resolved);
        let id = if copying {
            s.scene.timing.id()?
        } else {
            p.clip.id
        };
        s.scene.timing.clips.push(CelClip {
            id,
            first,
            end,
            ..p.clip.clone()
        });
        for (owner, key, enabled) in &p.keys {
            let mut key = *key;
            key.frame = map(key.frame)?;
            let k = s
                .scene
                .keyframes
                .tracks
                .iter_mut()
                .find(|k| k.owner == *owner)
                .ok_or("Pasted key owner removed")?;
            if k.keys.iter().any(|k| k.frame == key.frame) && !replace {
                return Err("Destination keys collide; review explicit replacement".into());
            }
            k.keys.retain(|k| k.frame != key.frame);
            k.keys.push(key);
            k.enabled = *enabled;
            k.keys.sort_by_key(|k| k.frame);
        }
        if s.scene.timing.labels_linked {
            for l in &p.labels {
                let mut l = l.clone();
                if copying {
                    l.id = s.scene.timing.edit.id()?;
                }
                l.clip = Some(id);
                l.first = map(l.first)?;
                l.end = map(l.end)?;
                s.scene.timing.edit.labels.push(l);
            }
        }
    }
    for (t, v) in by {
        crate::timeline_time_edit::encode(s, t, &v)?;
    }
    Ok(())
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes().saturating_mul(3) > budget {
        return Err("Clip preparation exceeds working memory".into());
    }
    let mut s = capture(d, budget)?;
    let previous = s.scene.timing.clone();
    let summary = match edit {
        Edit::Split { id, at } => {
            check_track(d, clip(&s, id)?.track)?;
            split(&mut s, id, at)?;
            "Split clip bounds and linked range labels; frame placement unchanged".into()
        }
        Edit::Join(ids) => {
            if ids.len() < 2 {
                return Err("Choose at least two adjacent clips".into());
            }
            let mut cs = ids
                .iter()
                .map(|id| clip(&s, *id))
                .collect::<Result<Vec<_>, _>>()?;
            cs.sort_by_key(|c| c.first);
            if cs
                .windows(2)
                .any(|p| p[0].track != p[1].track || p[0].end != p[1].first)
            {
                return Err("Join requires adjacent clips on the same track".into());
            }
            check_track(d, cs[0].track)?;
            let keep = cs[0].id;
            let end = cs.last().unwrap().end;
            s.scene
                .timing
                .clips
                .retain(|c| !ids.contains(&c.id) || c.id == keep);
            s.scene
                .timing
                .clips
                .iter_mut()
                .find(|c| c.id == keep)
                .unwrap()
                .end = end;
            for l in &mut s.scene.timing.edit.labels {
                if l.clip.is_some_and(|id| ids.contains(&id)) {
                    l.clip = Some(keep);
                }
            }
            for f in &mut s.scene.timing.edit.folders {
                for c in &mut f.children {
                    if matches!(c,Node::Clip(id)if ids.contains(id)) {
                        *c = Node::Clip(keep);
                    }
                }
                let mut seen = BTreeSet::new();
                f.children.retain(|c| seen.insert(*c));
            }
            "Join adjacent clips without changing exposure/key positions".into()
        }
        Edit::Bounds { id, first, end } => {
            let c = clip(&s, id)?;
            check_track(d, c.track)?;
            let t = index(&s, c.track)?;
            if first >= end || end > s.animation.as_ref().unwrap().tracks[t].end {
                return Err("Invalid clip bounds".into());
            }
            if s.scene.timing.clips.iter().any(|c| {
                c.id != id
                    && c.track
                        == s.scene
                            .timing
                            .clips
                            .iter()
                            .find(|c| c.id == id)
                            .unwrap()
                            .track
                    && c.first < end
                    && first < c.end
            }) {
                return Err("Clip bounds overlap a neighbour".into());
            }
            let c = s
                .scene
                .timing
                .clips
                .iter_mut()
                .find(|c| c.id == id)
                .unwrap();
            c.first = first;
            c.end = end;
            "Adjust clip bounds only; source frame placement unchanged".into()
        }
        Edit::Move {
            ids,
            at,
            copy: copying,
            replace,
        } => {
            if ids.is_empty() || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len() {
                return Err("Choose distinct clips".into());
            }
            let cs = ids
                .iter()
                .map(|id| clip(&s, *id))
                .collect::<Result<Vec<_>, _>>()?;
            let first = cs.iter().map(|c| c.first).min().unwrap();
            let mut pieces = vec![];
            for c in &cs {
                check_track(d, c.track)?;
                let t = index(&s, c.track)?;
                let values = dense(&s, t)[c.first as usize..c.end as usize]
                    .iter()
                    .map(|cell| cell.map(|cell| s.scene.catalog.cell_id(c.track, cell)))
                    .collect();
                let keys = s
                    .scene
                    .keyframes
                    .tracks
                    .iter()
                    .filter(|k| owner_track(k.owner) == Some(c.track))
                    .flat_map(|k| {
                        k.keys
                            .iter()
                            .filter(move |k| (c.first..c.end).contains(&k.frame))
                            .map(move |key| (k.owner, *key, k.enabled))
                    })
                    .collect();
                pieces.push(Piece {
                    clip: c.clone(),
                    values,
                    keys,
                    labels: s
                        .scene
                        .timing
                        .edit
                        .labels
                        .iter()
                        .filter(|l| l.clip == Some(c.id))
                        .cloned()
                        .collect(),
                });
            }
            if !copying {
                remove(&mut s, &ids)?;
            }
            insert(&mut s, &pieces, first, at, replace, copying)?;
            format!(
                "{} selected clips and in-range keys{}; explicit replacement={replace}",
                if copying { "Copy" } else { "Move" },
                if s.scene.timing.labels_linked {
                    " with linked range labels"
                } else {
                    "; labels remain at original positions"
                }
            )
        }
        Edit::Delete(ids) => {
            if ids.is_empty() {
                return Err("Choose clips".into());
            }
            for id in &ids {
                check_track(d, clip(&s, *id)?.track)?;
            }
            remove(&mut s, &ids)?;
            "Delete selected clip exposures/keys and linked labels; source definitions retained"
                .into()
        }
        Edit::Cut { track, first, end } => {
            check_track(d, track)?;
            let _ = copy(d, track, first, end)?;
            for c in s.scene.timing.clips.clone() {
                if c.track == track && c.first < first && first < c.end {
                    split(&mut s, c.id, first)?;
                }
            }
            for c in s.scene.timing.clips.clone() {
                if c.track == track && c.first < end && end < c.end {
                    split(&mut s, c.id, end)?;
                }
            }
            let ids = s
                .scene
                .timing
                .clips
                .iter()
                .filter(|c| c.track == track && first <= c.first && c.end <= end)
                .map(|c| c.id)
                .collect::<Vec<_>>();
            remove(&mut s, &ids)?;
            "Cut selected interval; boundary labels split and outside frames preserved".into()
        }
        Edit::Paste {
            clipboard,
            at,
            replace,
        } => {
            if clipboard.document != d.id || clipboard.timeline != d.scene.timelines.active {
                return Err("Clip clipboard belongs to another document or timeline".into());
            }
            for p in &clipboard.pieces {
                check_track(d, p.clip.track)?;
            }
            insert(
                &mut s,
                &clipboard.pieces,
                clipboard.first,
                at,
                replace,
                true,
            )?;
            "Paste range clipboard with new clip/label IDs; existing sources shared".into()
        }
        Edit::AutoExpand { id, on } => {
            let c = clip(&s, id)?;
            check_track(d, c.track)?;
            if on {
                s.scene.timing.edit.auto_expand.insert(id);
            } else {
                s.scene.timing.edit.auto_expand.remove(&id);
            }
            "Set automatic clip bounds from exposure/key endpoints".into()
        }
        Edit::LinkLabels(on) => {
            s.scene.timing.labels_linked = on;
            "Set document clip/range-label linking".into()
        }
        Edit::Label {
            clip: id,
            first,
            end,
            name: n,
        } => {
            let c = clip(&s, id)?;
            check_track(d, c.track)?;
            if !name(&n) || first >= end || first < c.first || end > c.end {
                return Err("Choose a bounded label name and an interval inside the clip".into());
            }
            let id = s.scene.timing.edit.id()?;
            s.scene.timing.edit.labels.push(Label {
                id,
                clip: Some(c.id),
                name: n,
                first,
                end,
            });
            "Create a linked clip range label".into()
        }
        Edit::Folder { name: n, children } => {
            if !name(&n) || children.is_empty() {
                return Err("Choose a folder name and child clips/folders".into());
            }
            let id = s.scene.timing.edit.id()?;
            s.scene.timing.edit.folders.push(Folder {
                id,
                name: n,
                children,
            });
            if !s.scene.timing.edit.valid(&s.scene.timing.clips) {
                return Err("Invalid, cyclic, duplicate or over-limit clip folder children".into());
            }
            "Create a clip folder; displayed bounds include all descendants".into()
        }
        Edit::SplitTrack { track, at } => {
            s = split_track(d, track, at, budget)?;
            "Split track at selected frame; cel sources shared, appearance and frame positions preserved".into()
        }
        Edit::MergeTracks { from, to, replace } => {
            s = merge_tracks(d, from, to, replace, budget)?;
            format!(
                "Merge adjacent compatible tracks; explicit frame/key replacement={replace}; source track removed in every arrangement"
            )
        }
    };
    reconcile(d, &mut s)?;
    crate::book_io::check_cancel(cancel)?;
    if !s.scene.timing.valid(false) {
        return Err("Clip/label/folder count, bounds or relationship limit".into());
    }
    crate::layer_groups::finish(d, &s)?;
    let mut rows = vec![];
    for id in previous
        .clips
        .iter()
        .map(|c| c.id)
        .chain(s.scene.timing.clips.iter().map(|c| c.id))
        .collect::<BTreeSet<_>>()
    {
        let old = previous.clips.iter().find(|c| c.id == id);
        let new = s.scene.timing.clips.iter().find(|c| c.id == id);
        if old != new {
            let describe = |c: Option<&CelClip>| {
                c.map_or_else(
                    || "absent".into(),
                    |c| format!("track{} frames{}–{}", c.track, c.first + 1, c.end),
                )
            };
            rows.push(format!("Clip{id}: {} → {}", describe(old), describe(new)));
        }
    }
    for id in previous
        .edit
        .labels
        .iter()
        .map(|l| l.id)
        .chain(s.scene.timing.edit.labels.iter().map(|l| l.id))
        .collect::<BTreeSet<_>>()
    {
        let old = previous.edit.labels.iter().find(|l| l.id == id);
        let new = s.scene.timing.edit.labels.iter().find(|l| l.id == id);
        if old != new {
            let describe = |l: Option<&Label>| {
                l.map_or_else(
                    || "absent".into(),
                    |l| format!("{} [{}–{}] clip{:?}", l.name, l.first + 1, l.end, l.clip),
                )
            };
            rows.push(format!("Label{id}: {} → {}", describe(old), describe(new)));
        }
    }
    rows.push(format!(
        "Clip folders: {} → {}; linked labels={}",
        previous.edit.folders.len(),
        s.scene.timing.edit.folders.len(),
        s.scene.timing.labels_linked
    ));
    Ok(Prepared {
        state: s,
        id: d.id,
        revision: d.revision,
        summary,
        rows,
    })
}
/// Called at document commit so ordinary exposure/key edits also extend opted-in clip bounds.
pub(crate) fn reconcile(before: &Document, s: &mut DocumentState) -> Result<(), String> {
    let clips = s.scene.timing.clips.clone();
    for id in s.scene.timing.edit.auto_expand.clone() {
        let Some(c) = clips.iter().find(|c| c.id == id) else {
            continue;
        };
        let t = index(s, c.track)?;
        let a = s.animation.as_ref().unwrap();
        let low = clips
            .iter()
            .filter(|n| n.track == c.track && n.end <= c.first && n.id != c.id)
            .map(|n| n.end)
            .max()
            .unwrap_or(0);
        let high = clips
            .iter()
            .filter(|n| n.track == c.track && n.first >= c.end && n.id != c.id)
            .map(|n| n.first)
            .min()
            .unwrap_or(a.tracks[t].end);
        let mut points = a.tracks[t]
            .exposures
            .iter()
            .filter(|e| !blank(s, t, e.cell))
            .map(|e| e.frame)
            .chain(
                s.scene
                    .keyframes
                    .tracks
                    .iter()
                    .filter(|k| owner_track(k.owner) == Some(c.track))
                    .flat_map(|k| k.keys.iter().map(|k| k.frame)),
            )
            .filter(|f| low <= *f && *f < high);
        if let Some(first) = points.next() {
            let (min, max) = points.fold((first, first), |(a, b), f| (a.min(f), b.max(f)));
            let target = s
                .scene
                .timing
                .clips
                .iter_mut()
                .find(|c| c.id == id)
                .unwrap();
            target.first = target.first.min(min);
            target.end = target.end.max(max + 1).min(high);
        }
    }
    let time = &mut s.scene.timing;

    if time.labels_linked {
        let fps_changed =
            before.animation.as_ref().map(|a| a.fps) != s.animation.as_ref().map(|a| a.fps);
        time.edit.labels.retain_mut(|l| {
            if before
                .scene
                .timing
                .edit
                .labels
                .iter()
                .find(|old| old.id == l.id)
                == Some(l)
                && let Some(id) = l.clip
                && let (Some(a), Some(b)) = (
                    before.scene.timing.clips.iter().find(|c| c.id == id),
                    time.clips.iter().find(|c| c.id == id),
                )
                && a != b
            {
                if fps_changed {
                    let map = |f: u32| {
                        b.first
                            + ((f.saturating_sub(a.first) as u64 * (b.end - b.first) as u64)
                                / (a.end - a.first) as u64) as u32
                    };
                    l.first = map(l.first);
                    l.end = map(l.end);
                } else if a.end - a.first == b.end - b.first {
                    let shift = b.first as i64 - a.first as i64;
                    l.first = (l.first as i64 + shift).max(0) as u32;
                    l.end = (l.end as i64 + shift).max(0) as u32;
                }
                l.first = l.first.max(b.first);
                l.end = l.end.min(b.end);
            }
            l.first < l.end
        });
    }
    prune(time);
    Ok(())
}
pub(crate) fn prune(time: &mut Timing) {
    let live = time.clips.iter().map(|c| c.id).collect::<BTreeSet<_>>();
    time.edit.auto_expand.retain(|id| live.contains(id));
    for l in &mut time.edit.labels {
        if l.clip.is_some_and(|id| !live.contains(&id)) {
            l.clip = None;
        }
    }
    for _ in 0..17 {
        let folders = time
            .edit
            .folders
            .iter()
            .map(|f| f.id)
            .collect::<BTreeSet<_>>();
        for f in &mut time.edit.folders {
            f.children.retain(|n| match n {
                Node::Clip(n) => live.contains(n),
                Node::Folder(n) => folders.contains(n),
            });
        }
        let n = time.edit.folders.len();
        time.edit.folders.retain(|f| !f.children.is_empty());
        if n == time.edit.folders.len() {
            break;
        }
    }
}

fn split_track(d: &Document, track: u64, at: u32, budget: usize) -> Result<DocumentState, String> {
    check_track(d, track)?;
    let old = capture(d, budget)?;
    let t = index(&old, track)?;
    let end = old.animation.as_ref().unwrap().tracks[t].end;
    if at == 0 || at >= end {
        return Err("Choose a split strictly inside the track".into());
    }
    let w = crate::layer_merge::preview(&old, d.current_frame)?;
    let copy = crate::timeline_editor::prepare(
        &w,
        crate::timeline_editor::Edit::CopyTrack {
            track,
            independent: false,
        },
        budget,
    )?;
    let to = copy.track.ok_or("Copied track missing")?;
    let mut s = copy.state;
    let nt = index(&s, to)?;
    let left = dense(&s, t)[..at as usize].to_vec();
    let mut right = dense(&s, nt);
    right[..at as usize].fill(None);
    crate::timeline_time_edit::encode(&mut s, t, &left)?;
    crate::timeline_time_edit::encode(&mut s, nt, &right)?;
    // Pair copied clips in stable duplicate order, then intersect both halves.
    let originals = old
        .scene
        .timing
        .clips
        .iter()
        .filter(|c| c.track == track)
        .cloned()
        .collect::<Vec<_>>();
    let copies = s
        .scene
        .timing
        .clips
        .iter()
        .filter(|c| c.track == to)
        .cloned()
        .collect::<Vec<_>>();
    for (a, b) in originals.iter().zip(&copies) {
        for mut l in old
            .scene
            .timing
            .edit
            .labels
            .iter()
            .filter(|l| l.clip == Some(a.id))
            .cloned()
        {
            l.id = s.scene.timing.edit.id()?;
            l.clip = Some(b.id);
            s.scene.timing.edit.labels.push(l);
        }
        if old.scene.timing.edit.auto_expand.contains(&a.id) {
            s.scene.timing.edit.auto_expand.insert(b.id);
        }
        for f in &mut s.scene.timing.edit.folders {
            if let Some(n) = f.children.iter().position(|n| *n == Node::Clip(a.id)) {
                f.children.insert(n + 1, Node::Clip(b.id));
            }
        }
    }
    for c in &mut s.scene.timing.clips {
        if c.track == track {
            c.end = c.end.min(at);
        }
        if c.track == to {
            c.first = c.first.max(at);
        }
    }
    let removed = s
        .scene
        .timing
        .clips
        .iter()
        .filter(|c| c.first >= c.end)
        .map(|c| c.id)
        .collect::<BTreeSet<_>>();
    s.scene.timing.clips.retain(|c| !removed.contains(&c.id));
    if s.scene.timing.labels_linked {
        let clips = s.scene.timing.clips.clone();
        s.scene.timing.edit.labels.retain_mut(|l| {
            if let Some(id) = l.clip
                && originals.iter().chain(&copies).any(|c| c.id == id)
            {
                let Some(c) = clips.iter().find(|c| c.id == id) else {
                    return false;
                };
                l.first = l.first.max(c.first);
                l.end = l.end.min(c.end);
                l.first < l.end
            } else {
                true
            }
        });
    }
    // Other arrangements retain the source track's old layout and leave the new global track empty.
    for entry in &mut s.scene.timelines.entries {
        if let Some(a) = &mut entry.saved {
            a.tracks.retain(|p| p.track != to);
            a.keys.tracks.retain(|k| owner_track(k.owner) != Some(to));
        }
    }
    prune(&mut s.scene.timing);
    Ok(s)
}
fn compatible_keys(data: &crate::keyframes::Data, from: u64, to: u64) -> bool {
    let a = data.get(Owner::Layer(Key::Track(from))).cloned();
    let b = data.get(Owner::Layer(Key::Track(to))).cloned();
    match (a, b) {
        (None, None) => true,
        (Some(mut a), Some(b)) => {
            a.owner = b.owner;
            a == b
        }
        _ => false,
    }
}
fn coalesce(time: &mut Timing, from: u64, to: u64) {
    for c in &mut time.clips {
        if c.track == from {
            c.track = to;
        }
    }
    let mut cs = time
        .clips
        .iter()
        .filter(|c| c.track == to)
        .cloned()
        .collect::<Vec<_>>();
    cs.sort_by_key(|c| c.first);
    let mut map = BTreeMap::new();
    let mut joined: Vec<CelClip> = vec![];
    for c in cs {
        if let Some(last) = joined.last_mut()
            && c.first <= last.end
        {
            map.insert(c.id, last.id);
            last.end = last.end.max(c.end);
            continue;
        }
        map.insert(c.id, c.id);
        joined.push(c);
    }
    time.clips.retain(|c| c.track != to);
    time.clips.extend(joined);
    for l in &mut time.edit.labels {
        if let Some(n) = l.clip.and_then(|n| map.get(&n)) {
            l.clip = Some(*n);
        }
    }
    time.edit.auto_expand = time
        .edit
        .auto_expand
        .iter()
        .map(|n| *map.get(n).unwrap_or(n))
        .collect();
    for f in &mut time.edit.folders {
        let mut seen = BTreeSet::new();
        for n in &mut f.children {
            if let Node::Clip(id) = n
                && let Some(n) = map.get(id)
            {
                *id = *n;
            }
        }
        f.children.retain(|n| seen.insert(*n));
    }
}
fn copy_slot(
    s: &mut DocumentState,
    source: &DocumentState,
    from: u64,
    to: u64,
    c: usize,
) -> Result<(usize, u64), String> {
    use crate::layer_properties::Target;
    let a = index(source, from)?;
    let b = index(s, to)?;
    let old = source.scene.catalog.cell_id(from, c);
    let n = s.animation.as_ref().unwrap().tracks[b].cells.len();
    s.animation.as_mut().unwrap().tracks[b]
        .cells
        .push(source.animation.as_ref().unwrap().tracks[a].cells[c].clone());
    s.scene.catalog.cells.entry(to).or_default().append(n + 1)?;
    let id = s.scene.catalog.cell_id(to, n);
    let oldkey = Key::Cell {
        track: from,
        cell: old,
    };
    let newkey = Key::Cell {
        track: to,
        cell: id,
    };
    if let Some(p) = source
        .scene
        .layer_properties
        .cells
        .get(&(((a as u64) << 32) | c as u64))
    {
        s.scene
            .layer_properties
            .cells
            .insert(((b as u64) << 32) | n as u64, *p);
    }
    if source.scene.clipping.alpha.contains(&Target::Cell {
        track: a as u16,
        cell: c as u16,
    }) {
        s.scene.clipping.alpha.insert(Target::Cell {
            track: b as u16,
            cell: n as u16,
        });
    }
    if let Some(p) = source
        .scene
        .catalog
        .cell_info
        .get(&from)
        .and_then(|p| p.get(&old))
    {
        s.scene
            .catalog
            .cell_info
            .entry(to)
            .or_default()
            .insert(id, p.clone());
    }
    if let Some(source) = source.scene.cels.source(from, old) {
        s.scene.cels.links.push(crate::cels::Link {
            track: to,
            cell: id,
            source,
        });
    }
    crate::file_objects::duplicate(s, oldkey, newkey);
    for f in source
        .scene
        .layer_groups
        .transforms
        .iter()
        .filter(|f| f.key == oldkey)
    {
        let mut f = f.clone();
        f.key = newkey;
        s.scene.layer_groups.transforms.push(f);
    }
    for m in source
        .scene
        .comics
        .masks
        .iter()
        .filter(|m| m.target == oldkey)
    {
        let mut m = m.clone();
        m.target = newkey;
        s.scene.comics.masks.push(m);
    }
    if let Some(mut k) = source.scene.keyframes.get(Owner::Layer(oldkey)).cloned() {
        k.owner = Owner::Layer(newkey);
        s.scene.keyframes.tracks.push(k);
    }
    Ok((n, id))
}
fn merge_tracks(
    d: &Document,
    from: u64,
    to: u64,
    replace: bool,
    budget: usize,
) -> Result<DocumentState, String> {
    use crate::layer_properties::Target;
    if from == to {
        return Err("Choose two tracks".into());
    }
    check_track(d, from)?;
    check_track(d, to)?;
    let original = capture(d, budget)?;
    let a = index(&original, from)?;
    let b = index(&original, to)?;
    if a.abs_diff(b) != 1
        || original.scene.layer_properties.get(Target::Track(a as u16))
            != original.scene.layer_properties.get(Target::Track(b as u16))
        || !compatible_keys(&original.scene.keyframes, from, to)
    {
        return Err("Merge adjacent tracks with matching visibility/opacity/blend and track keyframes; resolve appearance differences first".into());
    }
    let transforms = |key| {
        original
            .scene
            .layer_groups
            .transforms
            .iter()
            .filter(|f| f.key == key)
            .map(|f| f.matrix)
            .collect::<Vec<_>>()
    };
    if transforms(Key::Track(from)) != transforms(Key::Track(to))
        || original
            .scene
            .clipping
            .alpha
            .contains(&Target::Track(a as u16))
        || original
            .scene
            .clipping
            .alpha
            .contains(&Target::Track(b as u16))
        || original.scene.file_objects.get(Key::Track(from)).is_some()
        || original.scene.file_objects.get(Key::Track(to)).is_some()
    {
        return Err(
            "Resolve track-level clipping/file-object/transform differences before merging".into(),
        );
    }
    let mut s = original.clone();
    let mut map = BTreeMap::new();
    for c in 0..original.animation.as_ref().unwrap().tracks[a].cells.len() {
        let (n, id) = copy_slot(&mut s, &original, from, to, c)?;
        map.insert(original.scene.catalog.cell_id(from, c), (n, id));
    }
    let mut left = dense(&original, b);
    let right = dense(&original, a);
    for (f, c) in right.into_iter().enumerate() {
        if let Some(c) = c {
            if left[f].is_some() && !replace {
                return Err(
                    "Track exposure ranges overlap; review explicit source-frame replacement"
                        .into(),
                );
            }
            let old = original.scene.catalog.cell_id(from, c);
            left[f] = Some(map[&old].0);
        }
    }
    crate::timeline_time_edit::encode(&mut s, b, &left)?;
    // Every inactive arrangement follows the same global merge, preserving its own time placement.
    let blank_index = if let Some(c) = s.animation.as_ref().unwrap().tracks[b]
        .cells
        .iter()
        .position(|c| c.name == "Empty frame" && c.pixels.as_ref() == &[[0; 4]])
    {
        c
    } else {
        let c = s.animation.as_ref().unwrap().tracks[b].cells.len();
        s.animation.as_mut().unwrap().tracks[b]
            .cells
            .push(crate::animation::Cell {
                name: "Empty frame".into(),
                width: 1,
                height: 1,
                offset: [0, 0],
                pixels: std::sync::Arc::new(vec![[0; 4]]),
            });
        s.scene.catalog.cells.entry(to).or_default().append(c + 1)?;
        c
    };
    let blank_id = s.scene.catalog.cell_id(to, blank_index);
    for e in &mut s.scene.timelines.entries {
        if let Some(saved) = &mut e.saved {
            let expand = |track| {
                let mut v = vec![None; saved.frames as usize];
                if let Some(p) = saved.tracks.iter().find(|p| p.track == track) {
                    for (n, (f, c)) in p.exposures.iter().enumerate() {
                        let end = p.exposures.get(n + 1).map_or(p.end, |p| p.0);
                        v[*f as usize..end as usize].fill(Some(*c));
                    }
                }
                v
            };
            let mut dst = expand(to);
            let src = expand(from);
            let occupied = |values: &[Option<u64>], track: u64, index: usize| {
                values.iter().flatten().any(|id| {
                    original
                        .scene
                        .catalog
                        .cells
                        .get(&track)
                        .and_then(|ids| {
                            ids.index(
                                *id,
                                original.animation.as_ref().unwrap().tracks[index]
                                    .cells
                                    .len(),
                            )
                        })
                        .is_some_and(|n| !blank(&original, index, n))
                })
            };
            let source_used = occupied(&src, from, a);
            let target_used = occupied(&dst, to, b);
            if source_used && target_used && !compatible_keys(&saved.keys, from, to) {
                return Err("A saved timeline has incompatible track keyframes".into());
            }
            if source_used && !target_used {
                saved
                    .keys
                    .tracks
                    .retain(|k| k.owner != Owner::Layer(Key::Track(to)));
                for k in &mut saved.keys.tracks {
                    if k.owner == Owner::Layer(Key::Track(from)) {
                        k.owner = Owner::Layer(Key::Track(to));
                    }
                }
            }
            for (f, c) in src.into_iter().enumerate() {
                if let Some(c) = c {
                    let old = original
                        .scene
                        .catalog
                        .cells
                        .get(&from)
                        .and_then(|ids| {
                            ids.index(
                                c,
                                original.animation.as_ref().unwrap().tracks[a].cells.len(),
                            )
                        })
                        .ok_or("Saved cel removed")?;
                    if blank(&original, a, old) {
                        continue;
                    }
                    if dst[f].is_some_and(|c| c != blank_id) && !replace {
                        return Err(
                            "A saved timeline has overlapping track exposures; review replacement"
                                .into(),
                        );
                    }
                    dst[f] = Some(map[&c].1);
                }
            }
            let mut exposures = vec![];
            let mut previous = None;
            for (f, c) in dst.into_iter().enumerate() {
                if c != previous {
                    if c.is_some() || previous.is_some() {
                        exposures.push((f as u32, c.unwrap_or(blank_id)));
                    }
                    previous = c;
                }
            }
            saved.tracks.retain(|p| p.track != from && p.track != to);
            saved.tracks.push(crate::timelines::Pattern {
                track: to,
                end: saved.frames,
                exposures,
            });
            saved.keys.tracks.retain_mut(|k| {
                if k.owner == Owner::Layer(Key::Track(from)) {
                    return false;
                }
                if let Owner::Layer(Key::Cell { track, cell }) = &mut k.owner
                    && *track == from
                {
                    *track = to;
                    *cell = map[cell].1;
                }
                true
            });
            coalesce(&mut saved.timing, from, to);
            for shot in &mut saved.shots.entries {
                if shot.track == from {
                    shot.track = to;
                    for c in &mut shot.cells {
                        *c = map[c].1;
                    }
                }
            }
        }
    }
    coalesce(&mut s.scene.timing, from, to);
    for shot in &mut s.scene.shots.entries {
        if shot.track == from {
            shot.track = to;
            for c in &mut shot.cells {
                *c = map[c].1;
            }
        }
    }
    let w = crate::layer_merge::preview(&s, d.current_frame)?;
    let mut keys = vec![Key::Track(from)];
    keys.extend(
        (0..w.animation.as_ref().unwrap().tracks[a].cells.len()).map(|c| Key::Cell {
            track: from,
            cell: w.scene.catalog.cell_id(from, c),
        }),
    );
    s = crate::search_edit::delete_batch(&w, &keys, true)?;
    Ok(s)
}

#[cfg(test)]
#[path = "clip_edit_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub split: bool,
    pub frames: [u32; 2],
    pub default_linked: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            split: true,
            frames: [0, 1],
            default_linked: true,
        }
    }
}
impl Preferences {
    pub fn valid(self) -> bool {
        self.version == 1 && self.frames.iter().all(|f| *f < 10000)
    }
    pub fn load(root: &std::path::Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local clip view settings storage".into());
        }
        let p = root.join("settings/timeline-clips.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let b = crate::book_io::read_local(&p, 8192)?;
        let p: Self = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid clip view settings; file preserved".into());
        }
        Ok(p)
    }
    pub fn save(self, root: &std::path::Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid clip view settings/storage".into());
        }
        let parent = root.join("settings");
        let mut stage = crate::animation_export::Stage::new(&parent)?;
        let mut f = stage.create("clips.json")?;
        f.write_all(&serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(
            stage.directory.join("clips.json"),
            parent.join("timeline-clips.json"),
        )
        .map_err(|e| e.to_string())
    }
}
