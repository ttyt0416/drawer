//! Sample-clock transport and deterministic bounded-block audio mixing/WAV publication.
use crate::{
    animation::{Audio, Fps},
    audio::Sound,
    document::Document,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Clone)]
struct Clip {
    audio: Audio,
    sound: Sound,
    end: u32,
}
#[derive(Clone)]
pub struct Plan {
    pub fps: Fps,
    pub frames: u32,
    pub rate: u32,
    pub channels: u16,
    clips: Vec<Clip>,
    document_paths: Vec<PathBuf>,
}
#[derive(Clone)]
pub struct Stream {
    pub plan: Arc<Plan>,
    pub start: u32,
    pub first: u32,
    pub end: u32,
    pub repeat: bool,
    start_sample: u64,
    first_sample: u64,
    end_sample: u64,
    limit: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub frame: u32,
    pub finished: bool,
}
pub fn boundary(frame: u32, fps: Fps, rate: u32) -> u64 {
    let n = frame as u128 * fps.denominator as u128 * rate as u128;
    ((n + fps.numerator as u128 / 2) / fps.numerator as u128) as u64
}
impl Plan {
    pub fn capture(d: &Document, rate: u32, channels: u16) -> Result<Self, String> {
        let a = d
            .animation
            .as_ref()
            .filter(|a| a.valid())
            .ok_or("Choose a valid animation document")?;
        if !(8000..=192000).contains(&rate) || !(1..=2).contains(&channels) {
            return Err("Output rate must be8..192kHz, mono/stereo".into());
        }
        let data = crate::timeline_timing::inventory(d)?;
        if !data.valid(false) {
            return Err("Invalid audio clip data".into());
        }
        let clips = a
            .audio
            .iter()
            .enumerate()
            .map(|(n, a)| {
                let c = data
                    .audio
                    .iter()
                    .find(|c| c.index == n)
                    .ok_or("Audio binding missing")?;
                Ok(Clip {
                    audio: a.clone(),
                    sound: c.sound.clone(),
                    end: c.end,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            fps: a.fps,
            frames: a.frames,
            rate,
            channels,
            clips,
            document_paths: d.path.iter().chain(d.source_path.iter()).cloned().collect(),
        })
    }
    pub fn stream(
        self: Arc<Self>,
        start: u32,
        first: u32,
        end: u32,
        repeat: bool,
    ) -> Result<Stream, String> {
        if first >= end || end > self.frames {
            return Err("Invalid included playback / output range".into());
        }
        let start = if (first..end).contains(&start) {
            start
        } else {
            first
        };
        let start_sample = boundary(start, self.fps, self.rate);
        let first_sample = boundary(first, self.fps, self.rate);
        let end_sample = boundary(end, self.fps, self.rate);
        if end_sample <= start_sample || end_sample <= first_sample {
            return Err("Playback range is shorter than one output sample".into());
        }
        Ok(Stream {
            plan: self,
            start,
            first,
            end,
            repeat,
            start_sample,
            first_sample,
            end_sample,
            limit: None,
        })
    }
    fn sample(&self, n: u64) -> ([i16; 2], u32) {
        let mut sum = [0.; 2];
        for c in &self.clips {
            if c.sound.mute || c.sound.gain == 0 || n >= boundary(c.end, self.fps, self.rate) {
                continue;
            }
            let a = &c.audio;
            let diff = n as i128 * self.fps.numerator as i128
                - a.start as i128 * self.fps.denominator as i128 * self.rate as i128;
            if diff < 0 {
                continue;
            }
            let den = self.rate as u128 * self.fps.numerator as u128;
            let index = diff as u128 * a.rate as u128;
            let lo = index / den;
            let count = a.sample_count as usize / a.channels as usize;
            if lo >= count as u128 {
                continue;
            }
            let lo = lo as usize;
            let hi = (lo + 1).min(count - 1);
            let w = (index % den) as f64 / den as f64;
            let pair = |i: usize| {
                let base = i * a.channels as usize;
                if a.channels == 1 {
                    [a.samples[base] as f64; 2]
                } else if a.channels == 2 {
                    [a.samples[base] as f64, a.samples[base + 1] as f64]
                } else {
                    [a.samples[base..base + a.channels as usize]
                        .iter()
                        .map(|v| *v as f64)
                        .sum::<f64>()
                        / a.channels as f64; 2]
                }
            };
            let l = pair(lo);
            let h = pair(hi);
            for ch in 0..2 {
                sum[ch] += (l[ch] + (h[ch] - l[ch]) * w) * c.sound.gain as f64 / 100.;
            }
        }
        if self.channels == 1 {
            sum = [(sum[0] + sum[1]) / 2.; 2];
        }
        let clipped = sum[..self.channels as usize]
            .iter()
            .filter(|v| **v > 32767. || **v < -32768.)
            .count() as u32;
        (
            sum.map(|v| v.round().clamp(-32768., 32767.) as i16),
            clipped,
        )
    }
    pub fn protected_paths(&self) -> impl Iterator<Item = &Path> {
        self.document_paths.iter().map(|p| p.as_path()).chain(
            self.clips
                .iter()
                .filter_map(|c| c.sound.source.as_ref().map(|s| s.path.as_path())),
        )
    }
}
impl Stream {
    pub fn limited(mut self, count: u64) -> Result<Self, String> {
        if count == 0 {
            return Err("Empty audio audition".into());
        }
        self.limit = Some(count);
        Ok(self)
    }
    pub fn duration(&self) -> Option<u64> {
        match (self.limit, self.repeat) {
            (Some(n), false) => Some(n.min(self.end_sample - self.start_sample)),
            (Some(n), true) => Some(n),
            (None, false) => Some(self.end_sample - self.start_sample),
            (None, true) => None,
        }
    }

    pub fn absolute_sample(&self, played: u64) -> Option<u64> {
        if self.limit.is_some_and(|n| played >= n) {
            return None;
        }
        let first = self.end_sample - self.start_sample;
        if played < first {
            Some(self.start_sample + played)
        } else if self.repeat {
            Some(self.first_sample + (played - first) % (self.end_sample - self.first_sample))
        } else {
            None
        }
    }
    pub fn position(&self, played: u64) -> Position {
        let finished = self.absolute_sample(played).is_none();
        let Some(n) = self.absolute_sample(played).or_else(|| {
            self.duration()
                .and_then(|n| self.absolute_sample(n.saturating_sub(1)))
        }) else {
            return Position {
                frame: self.end - 1,
                finished: true,
            };
        };
        let mut frame = (n as u128 * self.plan.fps.numerator as u128
            / (self.plan.rate as u128 * self.plan.fps.denominator as u128))
            as u32;
        while frame + 1 < self.end && boundary(frame + 1, self.plan.fps, self.plan.rate) <= n {
            frame += 1;
        }
        while frame > self.first && boundary(frame, self.plan.fps, self.plan.rate) > n {
            frame -= 1;
        }
        Position {
            frame: frame.clamp(self.first, self.end - 1),
            finished,
        }
    }
    /// At most4096 sample frames; each call addresses the immutable transport snapshot.
    pub fn block(&self, played: u64, count: usize) -> Result<(Vec<i16>, u64), String> {
        if count > 4096 {
            return Err("Audio mix block exceeds4096 sample frames".into());
        }
        let mut pcm = Vec::with_capacity(count * self.plan.channels as usize);
        let mut clipped = 0;
        for i in 0..count {
            let Some(n) = self.absolute_sample(played + i as u64) else {
                break;
            };
            let (pair, c) = self.plan.sample(n);
            clipped += c as u64;
            pcm.extend_from_slice(&pair[..self.plan.channels as usize]);
        }
        Ok((pcm, clipped))
    }
}
#[derive(Clone, Copy)]
pub struct Options {
    pub first: u32,
    pub last: u32,
    pub rate: u32,
    pub channels: u16,
}
pub fn prepare(d: &Document, o: Options) -> Result<Stream, String> {
    if o.first > o.last {
        return Err("Reversed audio output range".into());
    }
    Arc::new(Plan::capture(d, o.rate, o.channels)?).stream(
        o.first,
        o.first,
        o.last.checked_add(1).ok_or("Frame overflow")?,
        false,
    )
}
pub fn wav_header(samples: u64, rate: u32, channels: u16) -> Result<Vec<u8>, String> {
    if !(8000..=192000).contains(&rate) || !(1..=2).contains(&channels) {
        return Err("Invalid WAV output format".into());
    }
    let bytes = samples
        .checked_mul(channels as u64 * 2)
        .filter(|n| *n <= u32::MAX as u64 - 36)
        .ok_or("WAV exceeds RIFF4GiB limit; reduce the range")? as u32;
    let mut b = Vec::from(&b"RIFF"[..]);
    b.extend((bytes + 36).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * channels as u32 * 2).to_le_bytes());
    b.extend((channels * 2).to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(bytes.to_le_bytes());
    Ok(b)
}
fn fingerprint(path: &Path) -> Result<Option<[u8; 32]>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(m) => {
            if !m.is_file() || m.file_type().is_symlink() {
                return Err("Choose a regular output file".into());
            }
            let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut b = [0u8; 65536];
            loop {
                let n = f.read(&mut b).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hash.update(&b[..n]);
            }
            Ok(Some(hash.finalize().into()))
        }
    }
}
#[derive(Clone)]
pub struct Destination {
    path: PathBuf,
    before: Option<[u8; 32]>,
}
impl Destination {
    pub fn capture(s: &Stream, path: &Path, overwrite: bool) -> Result<Self, String> {
        if !crate::font_library::local(path)
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        {
            return Err("Choose a local .wav output".into());
        }
        let parent = path
            .parent()
            .ok_or("Choose an output directory")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let path = parent.join(path.file_name().ok_or("Choose output filename")?);
        if s.plan.protected_paths().any(|p| p == path) {
            return Err("Choose another output to preserve the imported source".into());
        }
        let before = fingerprint(&path)?;
        if before.is_some() {
            let existing = path.canonicalize().map_err(|e| e.to_string())?;
            if s.plan.protected_paths().any(|p| p == existing) {
                return Err("Choose another output to preserve imported source".into());
            }
        }
        if before.is_some() && !overwrite {
            return Err("Confirm replacement of the existing WAV file".into());
        }
        Ok(Self { path, before })
    }
}
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn write(
    s: &Stream,
    dest: &Destination,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<u64, String> {
    let total = s.duration().ok_or("WAV output cannot loop forever")?;
    let header = wav_header(total, s.plan.rate, s.plan.channels)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("WAV output cancelled".into());
    }
    if fingerprint(&dest.path)? != dest.before {
        return Err("WAV destination changed since confirmation".into());
    }
    let temporary = dest.path.parent().unwrap().join(format!(
        ".drawer-audio-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let temp = Temporary(temporary);
    let mut writer = std::io::BufWriter::new(f);
    writer.write_all(&header).map_err(|e| e.to_string())?;
    let (mut at, mut clipped) = (0, 0);
    while at < total {
        if cancel.load(Ordering::Relaxed) {
            return Err("WAV output cancelled; existing file preserved".into());
        }
        let (pcm, c) = s.block(at, 4096)?;
        clipped += c;
        for v in &pcm {
            writer
                .write_all(&v.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        at += pcm.len() as u64 / s.plan.channels as u64;
        progress.store(at, Ordering::Relaxed);
    }
    writer
        .flush()
        .and_then(|_| writer.get_ref().sync_all())
        .map_err(|e| e.to_string())?;
    drop(writer);
    if cancel.load(Ordering::Relaxed) {
        return Err("WAV output cancelled before publication".into());
    }
    if fingerprint(&dest.path)? != dest.before {
        return Err("WAV destination changed during rendering; existing file preserved".into());
    }
    std::fs::rename(&temp.0, &dest.path).map_err(|e| e.to_string())?;
    Ok(clipped)
}
