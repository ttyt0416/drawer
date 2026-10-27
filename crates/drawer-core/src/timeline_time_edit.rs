use crate::{
    animation::{Cell, Exposure, Fps},
    cels,
    document::Document,
    layer_properties::Target,
    sizing::DocumentState,
    timeline_timing::{self as time, AudioClip, CelClip, Playback, Span},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub type Choices = BTreeMap<String, usize>;
#[derive(Clone, Debug)]
pub struct Issue {
    pub id: String,
    pub message: String,
    pub options: Vec<String>,
    pub selected: Option<usize>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Scope {
    pub all: bool,
    pub tracks: Vec<u64>,
    pub audio: Vec<u64>,
    pub globals: bool,
}
#[derive(Clone, Debug)]
pub enum Request {
    Playback(Playback),
    Fps {
        fps: Fps,
        keep_time: bool,
        labels_linked: bool,
    },
    Splice {
        at: u32,
        count: u32,
        remove: bool,
        scope: Scope,
        split: bool,
        labels_linked: bool,
    },
    Batch(crate::timeline_batch::Pattern),
    AudioLock {
        id: u64,
        locked: bool,
    },
}
pub struct Prepared {
    state: DocumentState,
    pub rows: Vec<String>,
    pub issues: Vec<Issue>,
    pub request: Request,
    id: u64,
    revision: u64,
}
impl Prepared {
    pub fn ready(&self) -> bool {
        self.issues.iter().all(|i| i.selected.is_some())
    }
    pub fn commit(&self, d: &mut Document) -> Result<bool, String> {
        if d.id != self.id || d.revision != self.revision {
            return Err("Timeline changed; prepare the conversion again".into());
        }
        if !self.ready() {
            return Err(
                "Resolve every listed collision/minimum-length/removal before applying".into(),
            );
        }
        d.commit_state(self.state.clone())
    }
    pub fn preview(&self) -> Result<Document, String> {
        crate::layer_merge::preview(&self.state, 0)
    }
}
fn issue(
    issues: &mut Vec<Issue>,
    choices: &Choices,
    id: String,
    message: String,
    options: Vec<String>,
) -> Result<usize, String> {
    let selected = choices.get(&id).copied();
    if selected.is_some_and(|i| i >= options.len()) {
        return Err("Invalid conversion-table choice".into());
    }
    issues.push(Issue {
        id,
        message,
        options,
        selected,
    });
    Ok(selected.unwrap_or(0))
}
fn ack(
    issues: &mut Vec<Issue>,
    choices: &Choices,
    id: String,
    message: String,
) -> Result<(), String> {
    issue(
        issues,
        choices,
        id,
        message,
        vec!["Confirm the listed result".into()],
    )?;
    Ok(())
}
fn track_index(s: &DocumentState, id: u64) -> Result<usize, String> {
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
        .ok_or_else(|| "Selected track removed".into())
}
fn writable(
    d: &Document,
    indices: &[usize],
    audio: &[u64],
    s: &DocumentState,
) -> Result<(), String> {
    for i in indices {
        let flags = crate::layer_properties::inherited(d, Target::Track(*i as u16));
        if flags.locked || flags.pixels_locked || flags.position_locked {
            return Err("Unlock every affected track before changing time".into());
        }
        for c in 0..d.animation.as_ref().unwrap().tracks[*i].cells.len() {
            let flags = crate::layer_properties::inherited(
                d,
                Target::Cell {
                    track: *i as u16,
                    cell: c as u16,
                },
            );
            if flags.locked || flags.pixels_locked || flags.position_locked {
                return Err("Unlock every affected cel before changing time".into());
            }
        }
    }
    if s.scene
        .timing
        .audio
        .iter()
        .any(|c| audio.contains(&c.id) && c.locked)
    {
        return Err("Unlock every affected audio clip before changing time".into());
    }
    Ok(())
}
pub(crate) fn dense(s: &DocumentState, index: usize) -> Vec<Option<usize>> {
    let t = &s.animation.as_ref().unwrap().tracks[index];
    let mut out = vec![None; t.end as usize];
    for (n, e) in t.exposures.iter().enumerate() {
        let end = t.exposures.get(n + 1).map_or(t.end, |e| e.frame);
        out[e.frame as usize..end as usize].fill(Some(e.cell));
    }
    out
}
pub(crate) fn encode(
    s: &mut DocumentState,
    index: usize,
    values: &[Option<usize>],
) -> Result<(), String> {
    let track = s.scene.catalog.tracks.id(index);
    let t = &mut s.animation.as_mut().unwrap().tracks[index];
    let mut blank = None;
    let mut previous = None;
    let mut exposures = vec![];
    for (frame, value) in values.iter().copied().enumerate() {
        if value != previous {
            if value.is_some() || previous.is_some() {
                let cell = if let Some(c) = value {
                    c
                } else {
                    if blank.is_none() {
                        let c = t.cells.len();
                        t.cells.push(Cell {
                            name: "Empty frame".into(),
                            width: 1,
                            height: 1,
                            offset: [0, 0],
                            pixels: Arc::new(vec![[0; 4]]),
                        });
                        s.scene
                            .catalog
                            .cells
                            .entry(track)
                            .or_default()
                            .append(t.cells.len())?;
                        blank = Some(c);
                    }
                    blank.unwrap()
                };
                exposures.push(Exposure {
                    frame: frame as u32,
                    cell,
                });
            }
            previous = value;
        }
    }
    t.end = values.len() as u32;
    t.exposures = exposures;
    Ok(())
}
fn map_splice(frame: u32, at: u32, count: u32, remove: bool) -> Result<u32, String> {
    if remove {
        Ok(if frame >= at + count {
            frame - count
        } else {
            frame.min(at)
        })
    } else if frame >= at {
        frame.checked_add(count).ok_or("Frame overflow".into())
    } else {
        Ok(frame)
    }
}
fn update_audio(
    s: &mut DocumentState,
    clips: Vec<(AudioClip, crate::animation::Audio)>,
) -> Result<(), String> {
    let mut bindings = vec![];
    let mut audio = vec![];
    for (mut c, mut a) in clips {
        c.index = audio.len();
        let samples = time::pcm(&s.scene.timing, &c)?;
        a.sample_count = u32::try_from(samples.len()).map_err(|_| "Audio sample limit")?;
        a.samples = Arc::new(samples);
        bindings.push(c);
        audio.push(a);
    }
    if audio.len() > 32 {
        return Err("Audio split would exceed32 clips".into());
    }
    s.scene.timing.audio = bindings;
    s.animation.as_mut().unwrap().audio = audio;
    Ok(())
}
fn retime(
    s: &mut DocumentState,
    fps: Fps,
    keep_time: bool,
    linked: bool,
    choices: &Choices,
    issues: &mut Vec<Issue>,
    rows: &mut Vec<String>,
) -> Result<(), String> {
    if !fps.valid() {
        return Err(
            "FPS numerator/denominator must be positive, at most100000 and at most240fps".into(),
        );
    }
    let old = s.animation.as_ref().unwrap().clone();
    let n = fps.numerator as u128 * old.fps.denominator as u128;
    let den = fps.denominator as u128 * old.fps.numerator as u128;
    let map = |f| {
        if keep_time {
            time::round_ratio(f, n, den)
        } else {
            Ok(f)
        }
    };
    let mut frames = map(old.frames)?.max(1);
    let mut boundaries: BTreeMap<u64, BTreeMap<u32, u32>> = BTreeMap::new();
    if keep_time {
        for (t, original) in old.tracks.iter().enumerate() {
            let id = s.scene.catalog.tracks.id(t);
            let mut exposures = vec![];
            let mut required = 0;
            let mut boundary = BTreeMap::from([(0, 0)]);
            for (i, e) in original.exposures.iter().enumerate() {
                let old_end = original
                    .exposures
                    .get(i + 1)
                    .map_or(original.end, |e| e.frame);
                let raw = map(e.frame)?;
                let raw_end = map(old_end)?;
                let start = raw.max(required);
                let end = raw_end.max(start + 1);
                if start != raw || end != raw_end {
                    ack(
                        issues,
                        choices,
                        format!("minimum-exposure-{id}-{}", e.frame),
                        format!(
                            "Track{id} exposure {}–{} rounds to{}–{}; keep at least1 frame at{}–{} and shift following starts",
                            e.frame + 1,
                            old_end,
                            raw + 1,
                            raw_end,
                            start + 1,
                            end
                        ),
                    )?;
                }
                rows.push(format!(
                    "Track{id}: {}–{} → {}–{} (cel slot{})",
                    e.frame + 1,
                    old_end,
                    start + 1,
                    end,
                    e.cell + 1
                ));
                exposures.push(Exposure {
                    frame: start,
                    cell: e.cell,
                });
                required = end;
                boundary.insert(e.frame, start);
                boundary.insert(old_end, end);
            }
            let end = map(original.end)?.max(required);
            frames = frames.max(end);
            boundary.insert(original.end, end);
            boundaries.insert(id, boundary);
            let target = &mut s.animation.as_mut().unwrap().tracks[t];
            target.exposures = exposures;
            target.end = end;
        }
        let mut groups: BTreeMap<u32, Vec<crate::animation_output::Frame>> = BTreeMap::new();
        for key in &s.scene.animation_output.keys {
            groups.entry(map(key.frame)?).or_default().push(*key);
        }
        let mut keys = vec![];
        for (frame, group) in groups {
            let selected = if group.len() > 1 {
                issue(
                    issues,
                    choices,
                    format!("key-{frame}"),
                    format!("{} output keys collide at frame{}", group.len(), frame + 1),
                    group
                        .iter()
                        .map(|k| format!("Keep original frame{}", k.frame + 1))
                        .collect(),
                )?
            } else {
                0
            };
            let mut key = group[selected];
            rows.push(format!("Output key{} → {}", key.frame + 1, frame + 1));
            key.frame = frame;
            frames = frames.max(frame + 1);
            keys.push(key);
        }
        s.scene.animation_output.keys = keys;
        let mut clips = vec![];
        let mut original_clips = s.scene.timing.clips.clone();
        original_clips.sort_by_key(|c| (c.track, c.first));
        let mut clip_end: BTreeMap<u64, u32> = BTreeMap::new();
        for c in &original_clips {
            let mut c = c.clone();
            let boundary = boundaries.get(&c.track);
            let raw_first = boundary
                .and_then(|b| b.get(&c.first))
                .copied()
                .unwrap_or(map(c.first)?);
            let first = raw_first.max(*clip_end.get(&c.track).unwrap_or(&0));
            let raw_end = boundary
                .and_then(|b| b.get(&c.end))
                .copied()
                .unwrap_or(map(c.end)?);
            let end = raw_end.max(first + 1);
            if first != raw_first || end != raw_end {
                ack(
                    issues,
                    choices,
                    format!("minimum-clip-{}", c.id),
                    format!("Cel clip{} needs at least1 frame", c.id),
                )?;
            }
            c.first = first;
            c.end = end;
            clip_end.insert(c.track, end);
            let i = track_index(s, c.track)?;
            s.animation.as_mut().unwrap().tracks[i].end =
                s.animation.as_ref().unwrap().tracks[i].end.max(end);
            frames = frames.max(end);
            clips.push(c);
        }
        s.scene.timing.clips = clips;
    }
    s.scene.timing.labels_linked = linked;
    if keep_time && linked {
        for l in &mut s.scene.cels.labels {
            let old = l.frame;
            l.frame = map(old)?;
            frames = frames.max(l.frame + 1);
            rows.push(format!("Label {}: {} → {}", l.name, old + 1, l.frame + 1));
        }
    }
    if linked {
        for l in &s.scene.cels.labels {
            if l.frame >= frames {
                ack(
                    issues,
                    choices,
                    format!("retain-label-{}", l.id),
                    format!(
                        "Extend the timeline to retain linked label {} at{}",
                        l.name,
                        l.frame + 1
                    ),
                )?;
                frames = l.frame + 1;
            }
        }
    }
    if let Some(p) = &mut s.scene.timing.playback {
        p.first = map(p.first)?;
        p.end = map(p.end)?.max(p.first + 1);
        frames = frames.max(p.end);
    }
    let mut audio = vec![];
    for c in &s.scene.timing.audio {
        let mut c = c.clone();
        let mut a = old.audio[c.index].clone();
        let start = map(a.start)?;
        let raw_end = map(c.end)?;
        let end = raw_end.max(start + 1);
        if raw_end <= start {
            ack(
                issues,
                choices,
                format!("minimum-audio-{}", c.id),
                format!("Audio clip{} needs at least1 frame", c.id),
            )?;
        }
        let count = time::sample_at(end - start, fps, a.rate)?.max(1);
        let old_count = time::length(&c.spans);
        c.spans = time::resize(&s.scene.timing, &c, count)?;
        rows.push(format!(
            "Audio{}: frame{}–{} → {}–{}; samples{} → {} (original/rate unchanged)",
            c.id,
            a.start + 1,
            c.end,
            start + 1,
            end,
            old_count,
            count
        ));
        a.start = start;
        c.end = end;
        frames = frames.max(start + 1);
        audio.push((c, a));
    }
    if frames > 10000 {
        return Err("Converted timeline exceeds10000 frames; choose another FPS or range".into());
    }
    s.animation.as_mut().unwrap().frames = frames;
    s.animation.as_mut().unwrap().fps = fps;
    update_audio(s, audio)?;
    rows.insert(
        0,
        format!(
            "FPS {}/{} → {}/{}; {} → {} frames; {}",
            old.fps.numerator,
            old.fps.denominator,
            fps.numerator,
            fps.denominator,
            old.frames,
            frames,
            if keep_time {
                "preserve real time (round half up)"
            } else {
                "preserve frame positions"
            }
        ),
    );
    Ok(())
}
fn splice(
    s: &mut DocumentState,
    operation: &Request,
    choices: &Choices,
    issues: &mut Vec<Issue>,
    rows: &mut Vec<String>,
) -> Result<(), String> {
    let Request::Splice {
        at,
        count,
        remove,
        scope,
        split,
        labels_linked,
    } = operation
    else {
        return Err("Expected a frame insertion/deletion request".into());
    };
    let (at, count, remove, split, linked) = (*at, *count, *remove, *split, *labels_linked);
    let old = s.animation.as_ref().unwrap().clone();
    if count == 0
        || count > 10000
        || at > old.frames
        || remove && (at >= old.frames || count > old.frames - at)
    {
        return Err(
            "Choose a positive count inside the timeline (insertion may be at the end)".into(),
        );
    }
    let tracks = if scope.all {
        (0..old.tracks.len()).collect::<Vec<_>>()
    } else {
        scope
            .tracks
            .iter()
            .map(|id| track_index(s, *id))
            .collect::<Result<Vec<_>, _>>()?
    };
    let audio_ids = if scope.all {
        s.scene
            .timing
            .audio
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
    } else {
        scope.audio.clone()
    };
    let globals = scope.all || scope.globals;
    if tracks.is_empty() && audio_ids.is_empty() && !globals {
        return Err("Select at least one cel/audio track or global timing row".into());
    }
    let mut frames = if scope.all {
        if remove {
            old.frames - count
        } else {
            old.frames.checked_add(count).ok_or("Frame overflow")?
        }
    } else {
        old.frames
    };
    frames = frames.max(1);
    if remove {
        ack(
            issues,
            choices,
            "remove-time-range".into(),
            format!(
                "Remove frames{}–{} from the listed owners; one Undo restores their contents and references",
                at + 1,
                at + count
            ),
        )?;
    }
    for i in &tracks {
        let values = dense(s, *i);
        let mut out = values.clone();
        if (at as usize) < values.len() {
            if remove {
                let end = (at + count).min(values.len() as u32);
                out.drain(at as usize..end as usize);
            } else {
                let value = if split {
                    None
                } else {
                    at.checked_sub(1)
                        .and_then(|f| values.get(f as usize).copied().flatten())
                };
                out.splice(
                    at as usize..at as usize,
                    std::iter::repeat_n(value, count as usize),
                );
            }
        }
        if out.len() > 10000 {
            return Err("Inserted track exceeds10000 frames".into());
        }
        frames = frames.max(out.len() as u32);
        encode(s, *i, &out)?;
        rows.push(format!(
            "Track{}: end{} → {}; {} exposure starts",
            s.scene.catalog.tracks.id(*i),
            values.len(),
            out.len(),
            s.animation.as_ref().unwrap().tracks[*i].exposures.len()
        ));
    }
    let track_ids = tracks
        .iter()
        .map(|i| s.scene.catalog.tracks.id(*i))
        .collect::<BTreeSet<_>>();
    let mut clips = vec![];
    let old_clips = s.scene.timing.clips.clone();
    for c in old_clips {
        if !track_ids.contains(&c.track) {
            clips.push(c);
            continue;
        }
        let parts = if split && c.first < at && c.end > at {
            if remove {
                vec![(c.first, at), (at + count, c.end)]
            } else {
                vec![(c.first, at), (at, c.end)]
            }
        } else {
            vec![(c.first, c.end)]
        };
        for (j, (first, end)) in parts.into_iter().enumerate() {
            if first >= end {
                continue;
            }
            let (first, end) = if split && !remove && c.first < at && c.end > at && j == 0 {
                (first, end)
            } else {
                (
                    map_splice(first, at, count, remove)?,
                    map_splice(end, at, count, remove)?,
                )
            };
            if first < end {
                let id = if j == 0 { c.id } else { s.scene.timing.id()? };
                clips.push(CelClip {
                    id,
                    track: c.track,
                    first,
                    end,
                });
            }
        }
    }
    s.scene.timing.clips = clips;
    let mut audio = vec![];
    for binding in s.scene.timing.audio.clone() {
        let a = &old.audio[binding.index];
        if !audio_ids.contains(&binding.id) {
            audio.push((binding, a.clone()));
            continue;
        }
        let mut c = binding.clone();
        let mut a = a.clone();
        let before = a.start;
        let old_end = c.end;
        if remove {
            let first = at.max(a.start);
            let end = (at + count).min(c.end);
            if first < end {
                let from =
                    time::sample_at(first - a.start, old.fps, a.rate)?.min(time::length(&c.spans));
                let to =
                    time::sample_at(end - a.start, old.fps, a.rate)?.min(time::length(&c.spans));
                let left = time::slice(&c.spans, 0, from);
                let right = time::slice(&c.spans, to, time::length(&c.spans));
                if split {
                    if !left.is_empty() {
                        let mut part = c.clone();
                        part.end = at;
                        part.spans = left;
                        audio.push((part, a.clone()));
                    }
                    if !right.is_empty() {
                        let mut part = c;
                        part.id = s.scene.timing.id()?;
                        part.spans = right;
                        part.end = map_splice(old_end, at, count, true)?;
                        a.start = map_splice(end, at, count, true)?;
                        audio.push((part, a));
                    }
                    rows.push(format!(
                        "Audio{} split/delete at{}–{}; original preserved",
                        binding.id,
                        at + 1,
                        at + count
                    ));
                    continue;
                }
                c.spans = left;
                c.spans.extend(right);
            }
            a.start = map_splice(a.start, at, count, true)?;
            c.end = map_splice(c.end, at, count, true)?;
        } else if a.start < at && c.end > at {
            let cut = time::sample_at(at - a.start, old.fps, a.rate)?.min(time::length(&c.spans));
            let left = time::slice(&c.spans, 0, cut);
            let right = time::slice(&c.spans, cut, time::length(&c.spans));
            if split {
                if !left.is_empty() {
                    let mut part = c.clone();
                    part.spans = left;
                    part.end = at;
                    audio.push((part, a.clone()));
                }
                if !right.is_empty() {
                    let mut part = c;
                    part.id = s.scene.timing.id()?;
                    part.spans = right;
                    part.end = old_end.checked_add(count).ok_or("Audio end overflow")?;
                    a.start = at + count;
                    audio.push((part, a));
                }
                rows.push(format!(
                    "Audio{} split around inserted frames{}–{}",
                    binding.id,
                    at + 1,
                    at + count
                ));
                continue;
            }
            c.spans = left;
            let silence = time::sample_at(count, old.fps, a.rate)?;
            if silence > 0 {
                c.spans.push(Span {
                    from: None,
                    count: silence,
                });
            }
            c.spans.extend(right);
            c.end = c.end.checked_add(count).ok_or("Audio end overflow")?;
        } else if a.start >= at {
            a.start = map_splice(a.start, at, count, false)?;
            c.end = map_splice(c.end, at, count, false)?;
        }
        if c.end > a.start && !c.spans.is_empty() {
            frames = frames.max(a.start + 1);
            rows.push(format!(
                "Audio{}: {}–{} → {}–{}; {} spans",
                c.id,
                before + 1,
                old_end,
                a.start + 1,
                c.end,
                c.spans.len()
            ));
            audio.push((c, a));
        }
    }
    update_audio(s, audio)?;
    for a in &s.animation.as_ref().unwrap().audio {
        frames = frames.max(a.start + 1);
    }
    s.scene.timing.labels_linked = linked;
    if globals {
        if s.scene
            .animation_output
            .keys
            .iter()
            .any(|k| k.frame >= old.frames)
        {
            return Err(
                "Move/remove output keys outside this timeline before editing global time".into(),
            );
        }
        s.scene.animation_output.keys.retain_mut(|k| {
            if remove && (at..at + count).contains(&k.frame) {
                rows.push(format!("Remove output key{}", k.frame + 1));
                return false;
            }
            if let Ok(frame) = map_splice(k.frame, at, count, remove) {
                k.frame = frame;
                true
            } else {
                false
            }
        });
        if linked {
            s.scene.cels.labels.retain_mut(|l| {
                if remove && (at..at + count).contains(&l.frame) {
                    rows.push(format!("Remove label {} at{}", l.name, l.frame + 1));
                    return false;
                }
                if let Ok(frame) = map_splice(l.frame, at, count, remove) {
                    l.frame = frame;
                    true
                } else {
                    false
                }
            });
        }
        if let Some(p) = &mut s.scene.timing.playback {
            p.first = map_splice(p.first, at, count, remove)?;
            p.end = map_splice(p.end, at, count, remove)?.max(p.first + 1);
            p.first = p.first.min(frames - 1);
            p.end = p.end.min(frames).max(p.first + 1);
        }
    }
    if frames > 10000 {
        return Err("Edited timeline exceeds10000 frames".into());
    }
    s.animation.as_mut().unwrap().frames = frames;
    rows.insert(
        0,
        format!(
            "{} {} frames at{}; {} → {} frames; {} clips",
            if remove { "Delete" } else { "Insert" },
            count,
            at + 1,
            old.frames,
            frames,
            if split {
                "split crossing"
            } else {
                "retain/resize"
            }
        ),
    );
    Ok(())
}
pub fn prepare(
    d: &Document,
    request: Request,
    choices: &Choices,
    budget: usize,
) -> Result<Prepared, String> {
    if let Request::Splice { scope, .. } = &request
        && (scope.tracks.iter().collect::<BTreeSet<_>>().len() != scope.tracks.len()
            || scope.audio.iter().collect::<BTreeSet<_>>().len() != scope.audio.len())
    {
        return Err("Duplicate selected owner IDs".into());
    }
    let mut s = time::enable(d, budget)?;
    let mut rows = vec![];
    let mut issues = vec![];
    let a = s.animation.as_ref().unwrap();
    let (tracks, audio) = match &request {
        Request::Fps { .. } => (
            (0..a.tracks.len()).collect::<Vec<_>>(),
            s.scene
                .timing
                .audio
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
        ),
        Request::Splice { scope, .. } => {
            let tracks = if scope.all {
                (0..a.tracks.len()).collect()
            } else {
                scope
                    .tracks
                    .iter()
                    .map(|id| track_index(&s, *id))
                    .collect::<Result<Vec<_>, _>>()?
            };
            let audio = if scope.all {
                s.scene.timing.audio.iter().map(|c| c.id).collect()
            } else {
                scope.audio.clone()
            };
            if audio
                .iter()
                .any(|id| !s.scene.timing.audio.iter().any(|c| c.id == *id))
            {
                return Err("Selected audio clip removed".into());
            }
            (tracks, audio)
        }
        Request::Batch(p) => (vec![track_index(&s, p.track)?], vec![]),
        _ => (vec![], vec![]),
    };
    writable(d, &tracks, &audio, &s)?;
    match &request {
        Request::Playback(p) => {
            if p.first >= p.end || p.end > s.animation.as_ref().unwrap().frames {
                return Err("Playback range must be nonempty and inside the timeline".into());
            }
            s.scene.timing.playback = Some(*p);
            rows.push(format!(
                "Playback {}–{}, repeat={}",
                p.first + 1,
                p.end,
                p.repeat
            ));
        }
        Request::Fps {
            fps,
            keep_time,
            labels_linked,
        } => retime(
            &mut s,
            *fps,
            *keep_time,
            *labels_linked,
            choices,
            &mut issues,
            &mut rows,
        )?,
        Request::Splice { .. } => splice(&mut s, &request, choices, &mut issues, &mut rows)?,
        Request::Batch(p) => {
            let collisions = crate::timeline_batch::apply(&mut s, p, &mut rows, budget)?;
            if collisions > 0 {
                ack(
                    &mut issues,
                    choices,
                    "batch-overwrite".into(),
                    format!(
                        "Replace {collisions} already assigned frames in the listed batch range"
                    ),
                )?;
            }
        }
        Request::AudioLock { id, locked } => {
            s.scene
                .timing
                .audio
                .iter_mut()
                .find(|c| c.id == *id)
                .ok_or("Audio clip removed")?
                .locked = *locked;
            rows.push(format!("Audio{id} lock={locked}"));
        }
    }
    if !d.scene.shots.is_default() {
        let old = s.scene.shots.entries.clone();
        let w = crate::layer_merge::preview(&s, 0)?;
        if s.animation
            .as_ref()
            .unwrap()
            .tracks
            .iter()
            .any(|t| !t.exposures.is_empty())
        {
            s = crate::shot_exchange::from_tracks(&w)?;
            for mut b in old {
                if !s.scene.shots.entries.iter().any(|v| v.track == b.track) {
                    b.shot.fps = s.animation.as_ref().unwrap().fps;
                    s.scene.shots.entries.push(b);
                }
            }
        }
    }
    for c in &s.scene.timing.clips {
        rows.push(format!(
            "Cel clip{} · track{} · {}–{}",
            c.id,
            c.track,
            c.first + 1,
            c.end
        ));
    }
    for c in &s.scene.timing.audio {
        rows.push(format!(
            "Audio clip{} · original{} · {} spans · {} sample frames",
            c.id,
            c.source,
            c.spans.len(),
            time::length(&c.spans)
        ));
    }
    time::populate(&mut s)?;
    time::reconcile(&mut s);
    cels::reconcile(d, &mut s)?;
    crate::layer_groups::finish(d, &s)?;
    let preview = crate::layer_merge::preview(&s, 0)?;
    if !time::valid_document(&preview)
        || !cels::valid_document(&preview)
        || !preview.animation.as_ref().unwrap().valid()
    {
        return Err("Invalid time edit, clip bindings, labels or audio cache".into());
    }
    if s.memory_bytes().saturating_add(d.memory_bytes()) > budget {
        return Err("Time edit exceeds snapshot budget".into());
    }
    Ok(Prepared {
        state: s,
        rows,
        issues,
        request,
        id: d.id,
        revision: d.revision,
    })
}
