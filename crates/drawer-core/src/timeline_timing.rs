//! Persistent playback bounds, clip boundaries and immutable PCM backing for time edits.
use crate::{
    animation::{Audio, Fps},
    document::Document,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playback {
    pub first: u32,
    pub end: u32,
    pub repeat: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelClip {
    pub id: u64,
    pub track: u64,
    pub first: u32,
    pub end: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Original {
    pub hash: [u8; 32],
    pub id: u64,
    pub rate: u32,
    pub channels: u16,
    pub count: u32,
    #[serde(skip)]
    pub samples: Arc<Vec<i16>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub from: Option<u32>,
    pub count: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    #[serde(default, skip_serializing_if = "crate::audio::Sound::is_default")]
    pub sound: crate::audio::Sound,
    pub id: u64,
    pub index: usize,
    pub source: u64,
    pub end: u32,
    pub locked: bool,
    pub spans: Vec<Span>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub playback: Option<Playback>,
    pub labels_linked: bool,
    pub clips: Vec<CelClip>,
    pub audio: Vec<AudioClip>,
    pub originals: Vec<Original>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            playback: None,
            labels_linked: true,
            clips: vec![],
            audio: vec![],
            originals: vec![],
        }
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            256 + self.clips.len() * 40
                + self
                    .audio
                    .iter()
                    .map(|a| {
                        std::mem::size_of::<AudioClip>() + a.spans.len() * 16 + a.sound.bytes()
                    })
                    .sum::<usize>()
                + self.originals.len() * 40
                + self.payload_bytes()
        }
    }
    pub fn payload_bytes(&self) -> usize {
        self.originals.iter().map(|s| s.samples.len() * 2).sum()
    }
    pub fn valid(&self, missing: bool) -> bool {
        let ids = self
            .clips
            .iter()
            .map(|c| c.id)
            .chain(self.audio.iter().map(|a| a.id))
            .chain(self.originals.iter().map(|s| s.id))
            .collect::<Vec<_>>();
        let mut clips = self.clips.iter().collect::<Vec<_>>();
        clips.sort_by_key(|c| (c.track, c.first));
        self.next > 0
            && clips
                .windows(2)
                .all(|c| c[0].track != c[1].track || c[0].end <= c[1].first)
            && self.clips.len() <= 2048
            && self.audio.len() <= 32
            && self.originals.len() <= 128
            && ids.iter().all(|id| *id > 0 && *id < self.next)
            && ids.iter().copied().collect::<BTreeSet<_>>().len() == ids.len()
            && self
                .playback
                .is_none_or(|r| r.first < r.end && r.end <= 10000)
            && self
                .clips
                .iter()
                .all(|c| c.track > 0 && c.first < c.end && c.end <= 10000)
            && self.originals.iter().all(|s| {
                (8000..=192000).contains(&s.rate)
                    && (1..=8).contains(&s.channels)
                    && s.count > 0
                    && s.count <= 32 * 1024 * 1024
                    && s.count.is_multiple_of(s.channels as u32)
                    && (missing || s.samples.len() == s.count as usize)
            })
            && self.audio.iter().all(|a| {
                a.sound.valid()
                    && a.index < 32
                    && a.end > 0
                    && a.spans.len() <= 4096
                    && !a.spans.is_empty()
                    && self
                        .originals
                        .iter()
                        .find(|s| s.id == a.source)
                        .is_some_and(|s| {
                            a.spans.iter().map(|p| p.count as u64).sum::<u64>()
                                <= 32 * 1024 * 1024 / s.channels as u64
                                && a.spans.iter().all(|p| {
                                    p.count > 0
                                        && p.count <= 32 * 1024 * 1024
                                        && p.from.is_none_or(|f| {
                                            f.checked_add(p.count).is_some_and(|end| {
                                                end <= s.count / s.channels as u32
                                            })
                                        })
                                })
                        })
            })
            && self
                .audio
                .iter()
                .map(|a| a.index)
                .collect::<BTreeSet<_>>()
                .len()
                == self.audio.len()
    }
    pub fn id(&mut self) -> Result<u64, String> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or("Timing IDs exhausted")?;
        Ok(id)
    }
}
pub fn round_ratio(value: u32, numerator: u128, denominator: u128) -> Result<u32, String> {
    if denominator == 0 {
        return Err("Time denominator must be positive".into());
    }
    let n = (value as u128)
        .checked_mul(numerator)
        .ok_or("Time boundary overflow")?;
    u32::try_from((n + denominator / 2) / denominator).map_err(|_| "Time boundary overflow".into())
}
pub fn sample_at(frame: u32, fps: Fps, rate: u32) -> Result<u32, String> {
    round_ratio(
        frame,
        rate as u128 * fps.denominator as u128,
        fps.numerator as u128,
    )
}
pub fn audio_end(a: &Audio, fps: Fps) -> Result<u32, String> {
    Ok(a.start
        .checked_add(
            round_ratio(
                a.sample_count / a.channels as u32,
                fps.numerator as u128,
                a.rate as u128 * fps.denominator as u128,
            )?
            .max(1),
        )
        .ok_or("Audio boundary overflow")?)
}
pub fn pcm(data: &Data, c: &AudioClip) -> Result<Vec<i16>, String> {
    let s = data
        .originals
        .iter()
        .find(|s| s.id == c.source)
        .ok_or("Audio original removed")?;
    let count = c
        .spans
        .iter()
        .try_fold(0usize, |n, p| {
            n.checked_add(p.count as usize * s.channels as usize)
        })
        .ok_or("Audio size overflow")?;
    if count == 0 || count > 32 * 1024 * 1024 {
        return Err("Audio cache limit32Mi samples".into());
    }
    let mut out = Vec::with_capacity(count);
    for p in &c.spans {
        if let Some(from) = p.from {
            let first = from as usize * s.channels as usize;
            let end = first + p.count as usize * s.channels as usize;
            out.extend_from_slice(s.samples.get(first..end).ok_or("Invalid audio trim")?);
        } else {
            out.resize(out.len() + p.count as usize * s.channels as usize, 0);
        }
    }
    Ok(out)
}
pub fn slice(spans: &[Span], first: u32, end: u32) -> Vec<Span> {
    let mut at = 0u32;
    let mut out = vec![];
    for s in spans {
        let a = at.max(first);
        let b = at.saturating_add(s.count).min(end);
        if a < b {
            out.push(Span {
                from: s.from.map(|f| f + a - at),
                count: b - a,
            });
        }
        at = at.saturating_add(s.count);
    }
    out
}
pub fn length(spans: &[Span]) -> u32 {
    spans.iter().fold(0u32, |n, s| n.saturating_add(s.count))
}
pub fn resize(data: &Data, clip: &AudioClip, count: u32) -> Result<Vec<Span>, String> {
    let old = length(&clip.spans);
    let mut out = slice(&clip.spans, 0, count);
    if count > old {
        let source = data
            .originals
            .iter()
            .find(|s| s.id == clip.source)
            .ok_or("Audio original removed")?;
        let mut remaining = count - old;
        if let Some(last) = out.last_mut()
            && let Some(from) = last.from
        {
            let available =
                (source.count / source.channels as u32).saturating_sub(from + last.count);
            let take = remaining.min(available);
            last.count += take;
            remaining -= take;
        }
        if remaining > 0 {
            out.push(Span {
                from: None,
                count: remaining,
            });
        }
    }
    Ok(out)
}
pub(crate) fn matches(data: &Data, c: &AudioClip, a: &Audio) -> bool {
    let Some(s) = data.originals.iter().find(|s| s.id == c.source) else {
        return false;
    };
    if s.rate != a.rate || s.channels != a.channels || a.samples.len() != a.sample_count as usize {
        return false;
    }
    let mut offset = 0usize;
    for span in &c.spans {
        let size = span.count as usize * s.channels as usize;
        let Some(samples) = a.samples.get(offset..offset.saturating_add(size)) else {
            return false;
        };
        if let Some(from) = span.from {
            let first = from as usize * s.channels as usize;
            if s.samples.get(first..first.saturating_add(size)) != Some(samples) {
                return false;
            }
        } else if samples.iter().any(|v| *v != 0) {
            return false;
        }
        offset += size;
    }
    offset == a.samples.len()
}
pub fn valid_document(d: &Document) -> bool {
    let data = &d.scene.timing;
    data.valid(false)
        && data
            .playback
            .is_none_or(|r| d.animation.as_ref().is_some_and(|a| r.end <= a.frames))
        && data.clips.iter().all(|c| {
            d.animation.as_ref().is_some_and(|a| {
                d.scene
                    .catalog
                    .tracks
                    .index(c.track, a.tracks.len())
                    .is_some_and(|i| c.end <= a.tracks[i].end)
            })
        })
        && data.audio.iter().all(|c| {
            d.animation
                .as_ref()
                .and_then(|a| a.audio.get(c.index))
                .is_some_and(|a| c.end > a.start && matches(data, c, a))
        })
}
pub fn enable(d: &Document, budget: usize) -> Result<DocumentState, String> {
    let mut state = crate::cels::enable(d, budget)?;
    reconcile(&mut state);
    populate(&mut state)?;
    Ok(state)
}
pub(crate) fn populate(state: &mut DocumentState) -> Result<(), String> {
    let a = state.animation.as_ref().ok_or("Choose an animation")?;
    for (n, t) in a.tracks.iter().enumerate() {
        let track = state.scene.catalog.tracks.id(n);
        if t.end > 0 && !state.scene.timing.clips.iter().any(|c| c.track == track) {
            let id = state.scene.timing.id()?;
            state.scene.timing.clips.push(CelClip {
                id,
                track,
                first: 0,
                end: t.end,
            });
        }
    }
    for (index, audio) in a.audio.iter().enumerate() {
        if !state.scene.timing.audio.iter().any(|c| c.index == index) {
            let source = state.scene.timing.id()?;
            let id = state.scene.timing.id()?;
            state.scene.timing.originals.push(Original {
                id: source,
                hash: pcm_hash(&audio.samples),
                rate: audio.rate,
                channels: audio.channels,
                count: audio.sample_count,
                samples: audio.samples.clone(),
            });
            state.scene.timing.audio.push(AudioClip {
                sound: Default::default(),
                id,
                index,
                source,
                end: audio_end(audio, a.fps)?,
                locked: false,
                spans: vec![Span {
                    from: Some(0),
                    count: audio.sample_count / audio.channels as u32,
                }],
            });
        }
    }
    for (n, t) in a.tracks.iter().enumerate() {
        let track = state.scene.catalog.tracks.id(n);
        let mut covered = vec![false; t.end as usize];
        for c in state.scene.timing.clips.iter().filter(|c| c.track == track) {
            covered[c.first as usize..c.end as usize].fill(true);
        }
        for (i, e) in t.exposures.iter().enumerate() {
            let cell = &t.cells[e.cell];
            if cell.name == "Empty frame"
                && cell.width == 1
                && cell.height == 1
                && cell.pixels.as_ref() == &[[0; 4]]
            {
                continue;
            }
            let end = t.exposures.get(i + 1).map_or(t.end, |e| e.frame);
            let mut at = e.frame;
            while at < end {
                if covered[at as usize] {
                    at += 1;
                    continue;
                }
                let first = at;
                while at < end && !covered[at as usize] {
                    covered[at as usize] = true;
                    at += 1;
                }
                let id = state.scene.timing.id()?;
                state.scene.timing.clips.push(CelClip {
                    id,
                    track,
                    first,
                    end: at,
                });
            }
        }
    }
    state.scene.timing.audio.sort_by_key(|c| c.index);
    Ok(())
}
pub fn inventory(d: &Document) -> Result<Data, String> {
    let mut state = d.snapshot();
    reconcile(&mut state);
    populate(&mut state)?;
    Ok(*state.scene.timing)
}
pub(crate) fn reconcile(s: &mut DocumentState) {
    let Some(a) = &s.animation else {
        s.scene.timing.clips.clear();
        s.scene.timing.audio.clear();
        s.scene.timing.playback = None;
        return;
    };
    s.scene.timing.clips.retain_mut(|c| {
        s.scene
            .catalog
            .tracks
            .index(c.track, a.tracks.len())
            .is_some_and(|i| {
                c.end = c.end.min(a.tracks[i].end);
                c.first < c.end
            })
    });
    let data = s.scene.timing.clone();
    s.scene.timing.audio.retain(|c| {
        a.audio
            .get(c.index)
            .is_some_and(|a| c.end > a.start && matches(&data, c, a))
    });
    if let Some(p) = &mut s.scene.timing.playback {
        p.end = p.end.min(a.frames);
        p.first = p.first.min(p.end - 1);
    }
}
pub fn playback(
    d: &Document,
    start: u32,
    seconds: f64,
    fallback_repeat: bool,
) -> Result<(u32, bool), String> {
    let a = d.animation.as_ref().ok_or("Choose an animation")?;
    let p = d.scene.timing.playback.unwrap_or(Playback {
        first: 0,
        end: a.frames,
        repeat: fallback_repeat,
    });
    let start = if (p.first..p.end).contains(&start) {
        start
    } else {
        p.first
    };
    let (frame, end) = crate::timeline_editor::playback(
        start - p.first,
        seconds,
        a.fps,
        p.end - p.first,
        p.repeat,
    )?;
    Ok((frame + p.first, end))
}
pub fn pcm_hash(samples: &[i16]) -> [u8; 32] {
    let mut hash = Sha256::new();
    let mut bytes = [0; 8192];
    for chunk in samples.chunks(4096) {
        for (v, b) in chunk.iter().zip(bytes.as_chunks_mut::<2>().0) {
            b.copy_from_slice(&v.to_le_bytes());
        }
        hash.update(&bytes[..chunk.len() * 2]);
    }
    hash.finalize().into()
}
pub(crate) fn original_hashes(d: &Document) -> bool {
    d.scene
        .timing
        .originals
        .iter()
        .all(|s| pcm_hash(&s.samples) == s.hash)
}
pub(crate) fn duplicate(
    scene: &mut crate::scene::Scene,
    original: &crate::scene::Scene,
    map: &std::collections::BTreeMap<crate::layer_search::Key, crate::layer_search::Key>,
) -> Result<(), String> {
    for c in &original.timing.clips {
        if let Some(crate::layer_search::Key::Track(track)) =
            map.get(&crate::layer_search::Key::Track(c.track))
        {
            let mut new = c.clone();
            new.id = scene.timing.id()?;
            new.track = *track;
            scene.timing.clips.push(new);
        }
    }
    Ok(())
}
pub(crate) fn write(s: &crate::scene::Scene, w: &mut impl Write) -> io::Result<()> {
    for source in &s.timing.originals {
        for value in source.samples.iter() {
            w.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> io::Result<()> {
    for n in 0..d.scene.timing.originals.len() {
        let count = d.scene.timing.originals[n].count as usize;
        if count.saturating_mul(2) > budget.saturating_sub(d.memory_bytes()) {
            return Err(io::Error::other("PCM backing allocation budget"));
        }
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            let mut b = [0; 2];
            r.read_exact(&mut b)?;
            samples.push(i16::from_le_bytes(b));
        }
        if pcm_hash(&samples) != d.scene.timing.originals[n].hash {
            return Err(io::Error::other("PCM backing hash mismatch"));
        }
        d.scene.timing.originals[n].samples = Arc::new(samples);
    }
    Ok(())
}
