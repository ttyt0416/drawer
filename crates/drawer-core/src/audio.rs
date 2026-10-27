//! Local WAV/raw PCM snapshots, persistent mixing controls and reversible clip edits.
use crate::{animation::Audio, document::Document, sizing::DocumentState, timeline_timing as time};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
pub const MAX_INPUT: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raw {
    pub rate: u32,
    pub channels: u16,
}
impl Raw {
    pub fn valid(self) -> bool {
        (8000..=192000).contains(&self.rate) && (1..=2).contains(&self.channels)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    pub hash: [u8; 32],
    pub linked: bool,
    pub raw: Option<Raw>,
}
impl Source {
    pub fn valid(&self) -> bool {
        crate::font_library::local(&self.path)
            && self.path.to_string_lossy().len() <= 4096
            && self.raw.is_none_or(Raw::valid)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sound {
    pub gain: u16,
    pub mute: bool,
    pub source: Option<Source>,
}
impl Default for Sound {
    fn default() -> Self {
        Self {
            gain: 100,
            mute: false,
            source: None,
        }
    }
}
impl Sound {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.gain <= 400 && self.source.as_ref().is_none_or(Source::valid)
    }
    pub fn bytes(&self) -> usize {
        self.source
            .as_ref()
            .map_or(0, |s| s.path.as_os_str().len() * 2 + 128)
    }
}
#[derive(Clone)]
pub struct Decoded {
    pub rate: u32,
    pub channels: u16,
    pub samples: Arc<Vec<i16>>,
    pub description: String,
    pub quantized: bool,
}
impl Decoded {
    pub fn valid(&self) -> bool {
        Raw {
            rate: self.rate,
            channels: self.channels,
        }
        .valid()
            && !self.samples.is_empty()
            && self.samples.len() <= 32 * 1024 * 1024
            && self.samples.len().is_multiple_of(self.channels as usize)
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
#[derive(Clone)]
pub struct Input {
    pub source: Source,
    pub decoded: Decoded,
    pub name: String,
}
fn u16_at(b: &[u8], i: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        b.get(i..i + 2)
            .ok_or("Truncated WAV header")?
            .try_into()
            .unwrap(),
    ))
}
fn u32_at(b: &[u8], i: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4)
            .ok_or("Truncated WAV header")?
            .try_into()
            .unwrap(),
    ))
}
fn decode_pcm(data: &[u8], format: u16, raw: Raw, bits: u16) -> Result<Decoded, String> {
    let bytes = bits as usize / 8;
    if !raw.valid()
        || bytes == 0
        || data.is_empty()
        || data.len() > MAX_INPUT
        || !data.len().is_multiple_of(bytes * raw.channels as usize)
        || data.len() / bytes > 32 * 1024 * 1024
    {
        return Err("Invalid PCM length, rate, channels or64MiB/32M-sample limit".into());
    }
    let samples = data
        .chunks_exact(bytes)
        .map(|b| {
            let v = match (format, bits) {
                (1, 8) => (b[0] as i32 - 128) * 256,
                (1, 16) => i16::from_le_bytes(b.try_into().unwrap()) as i32,
                (1, 24) => {
                    let n = i32::from_le_bytes([
                        b[0],
                        b[1],
                        b[2],
                        if b[2] & 128 != 0 { 255 } else { 0 },
                    ]);
                    (n as f64 / 256.).round() as i32
                }
                (1, 32) => {
                    (i32::from_le_bytes(b.try_into().unwrap()) as f64 / 65536.).round() as i32
                }
                (3, 32) => {
                    let v = f32::from_le_bytes(b.try_into().unwrap());
                    if !v.is_finite() {
                        return Err("Nonfinite WAV float sample".to_string());
                    }
                    (v.clamp(-1., 1.) as f64 * 32768.).round() as i32
                }
                _ => return Err("Supported WAV: PCM8/16/24/32 or IEEE float32, mono/stereo".into()),
            };
            Ok(v.clamp(-32768, 32767) as i16)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Decoded {
        rate: raw.rate,
        channels: raw.channels,
        samples: Arc::new(samples),
        description: format!(
            "{}-bit {} · {}Hz · {}ch",
            bits,
            if format == 3 { "float" } else { "PCM" },
            raw.rate,
            raw.channels
        ),
        quantized: bits > 16,
    })
}
pub fn decode(bytes: &[u8], raw: Option<Raw>) -> Result<Decoded, String> {
    if bytes.len() > MAX_INPUT {
        return Err("Audio input exceeds64MiB".into());
    }
    if let Some(raw) = raw {
        return decode_pcm(bytes, 1, raw, 16);
    }
    if bytes.get(..4) != Some(b"RIFF")
        || bytes.get(8..12) != Some(b"WAVE")
        || u32_at(bytes, 4)? as usize + 8 != bytes.len()
    {
        return Err("Choose a local RIFF WAV or explicitly specify raw PCM16LE parameters; compressed/RF64/RIFX codecs are unsupported".into());
    }
    let (mut at, mut fmt, mut data) = (12, None, None);
    while at < bytes.len() {
        let tag = bytes.get(at..at + 4).ok_or("Truncated WAV chunk")?;
        let len = u32_at(bytes, at + 4)? as usize;
        at += 8;
        let chunk = bytes
            .get(at..at.checked_add(len).ok_or("WAV chunk overflow")?)
            .ok_or("Truncated WAV chunk payload")?;
        if tag == b"fmt " && fmt.replace(chunk).is_some() {
            return Err("Duplicate WAV fmt chunk".into());
        }
        if tag == b"data" && data.replace(chunk).is_some() {
            return Err("Duplicate WAV data chunk".into());
        }
        at = at.checked_add(len + len % 2).ok_or("WAV chunk overflow")?;
        if at > bytes.len() {
            return Err("Missing WAV chunk padding".into());
        }
    }
    let f = fmt.ok_or("WAV format chunk missing")?;
    let mut kind = u16_at(f, 0)?;
    let raw = Raw {
        channels: u16_at(f, 2)?,
        rate: u32_at(f, 4)?,
    };
    let bits = u16_at(f, 14)?;
    if kind == 65534 {
        if f.len() < 40
            || u16_at(f, 16)? < 22
            || u16_at(f, 18)? == 0
            || u16_at(f, 18)? > bits
            || f.get(26..40) != Some(&[0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113])
        {
            return Err("Unsupported WAV extensible format".into());
        }
        kind = u16_at(f, 24)?;
        if kind == 3 && u16_at(f, 18)? != 32 {
            return Err("Float WAV valid-bit width must be32".into());
        }
        let mask = u32_at(f, 20)?;
        if mask != 0 && mask != if raw.channels == 1 { 4 } else { 3 } {
            return Err("Unsupported WAV channel layout".into());
        }
    }
    if !matches!((kind, bits), (1, 8 | 16 | 24 | 32) | (3, 32))
        || !raw.valid()
        || u16_at(f, 12)? != raw.channels * (bits / 8)
        || u32_at(f, 8)? != raw.rate * raw.channels as u32 * (bits as u32 / 8)
    {
        return Err("Unsupported or inconsistent WAV sample format".into());
    }
    decode_pcm(data.ok_or("WAV data missing")?, kind, raw, bits)
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    if !crate::font_library::local(path)
        || std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("Choose a regular local audio file; network paths are unsupported".into());
    }
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > MAX_INPUT as u64 {
        return Err("Audio source exceeds64MiB".into());
    }
    let mut bytes = vec![];
    f.take(MAX_INPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_INPUT {
        return Err("Audio source changed beyond64MiB".into());
    }
    Ok(bytes)
}
pub fn load(path: &Path, raw: Option<Raw>, linked: bool) -> Result<Input, String> {
    let bytes = read(path)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let source = Source {
        path: path.clone(),
        raw,
        linked,
        hash: Sha256::digest(&bytes).into(),
    };
    if !source.valid() {
        return Err("Invalid local audio source".into());
    }
    Ok(Input {
        source,
        decoded: decode(&bytes, raw)?,
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    })
}
pub fn source_status(source: &Source) -> String {
    if !source.linked {
        return "Embedded PCM (independent of original file)".into();
    }
    match read(&source.path) {
        Ok(b) if <[u8; 32]>::from(Sha256::digest(&b)) == source.hash => {
            "Linked source unchanged; playback uses saved PCM".into()
        }
        Ok(_) => "Linked source changed; saved PCM retained until explicit refresh".into(),
        Err(e) => format!("Linked source unavailable; saved PCM retained: {e}"),
    }
}
fn finish(d: &Document, s: DocumentState, budget: usize) -> Result<DocumentState, String> {
    if s.memory_bytes().saturating_add(d.memory_bytes()) > budget
        || s.animation
            .as_ref()
            .is_none_or(|a| !a.valid() || a.frames > 10000)
        || !s.scene.timing.valid(false)
    {
        return Err("Audio edit exceeds document/timeline limits".into());
    }
    Ok(s)
}
pub fn insert(
    d: &Document,
    input: &Input,
    start: u32,
    confirm_quantization: bool,
    budget: usize,
) -> Result<DocumentState, String> {
    if input.decoded.quantized && !confirm_quantization {
        return Err("Confirm conversion of24/32-bit audio to stored16-bit PCM".into());
    }
    if !input.decoded.valid()
        || input.name.trim().is_empty()
        || input.name.len() > 4096
        || !input.source.valid()
        || start >= 10000
    {
        return Err("Invalid local source or audio start frame".into());
    }
    let mut s = time::enable(d, budget)?;
    let a = s.animation.as_mut().unwrap();
    if a.audio.len() >= 32 {
        return Err("Maximum32 audio clips".into());
    }
    let index = a.audio.len();
    let sound = Audio {
        name: input.name.clone(),
        start,
        rate: input.decoded.rate,
        channels: input.decoded.channels,
        sample_count: u32::try_from(input.decoded.samples.len())
            .map_err(|_| "Audio sample limit")?,
        samples: input.decoded.samples.clone(),
    };
    let end = time::audio_end(&sound, a.fps)?;
    if end > 10000 {
        return Err("Audio duration exceeds10000 timeline frames; trim the source first".into());
    }
    a.frames = a.frames.max(end);
    a.audio.push(sound);
    time::populate(&mut s)?;
    let c = s
        .scene
        .timing
        .audio
        .iter_mut()
        .find(|c| c.index == index)
        .unwrap();
    c.sound.source = Some(input.source.clone());
    finish(d, s, budget)
}
pub fn edit(
    d: &Document,
    id: u64,
    name: String,
    start: u32,
    gain: u16,
    mute: bool,
    budget: usize,
) -> Result<DocumentState, String> {
    if name.trim().is_empty() || name.len() > 4096 || start >= 10000 || gain > 400 {
        return Err("Name, start frame or volume is invalid".into());
    }
    let mut s = time::enable(d, budget)?;
    let c = s
        .scene
        .timing
        .audio
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or("Audio clip removed")?;
    if c.locked {
        return Err("Unlock the audio clip first".into());
    }
    let old = s.animation.as_ref().unwrap().audio[c.index].start;
    let duration = c.end.checked_sub(old).ok_or("Invalid audio span")?;
    c.end = start
        .checked_add(duration)
        .filter(|n| *n <= 10000)
        .ok_or("Audio extends beyond10000 frames")?;
    c.sound.gain = gain;
    c.sound.mute = mute;
    let a = s.animation.as_mut().unwrap();
    a.audio[c.index].name = name;
    a.audio[c.index].start = start;
    a.frames = a.frames.max(c.end);
    finish(d, s, budget)
}
pub fn remove(d: &Document, id: u64, budget: usize) -> Result<DocumentState, String> {
    let mut s = time::enable(d, budget)?;
    let c = s
        .scene
        .timing
        .audio
        .iter()
        .find(|c| c.id == id)
        .ok_or("Audio clip removed")?;
    if c.locked {
        return Err("Unlock audio clip first".into());
    }
    let index = c.index;
    s.animation.as_mut().unwrap().audio.remove(index);
    s.scene.timing.audio.retain(|c| c.id != id);
    for c in &mut s.scene.timing.audio {
        if c.index > index {
            c.index -= 1;
        }
    }
    let used = s
        .scene
        .timing
        .audio
        .iter()
        .map(|c| c.source)
        .collect::<std::collections::BTreeSet<_>>();
    s.scene.timing.originals.retain(|o| used.contains(&o.id));
    finish(d, s, budget)
}
pub fn refresh(
    d: &Document,
    id: u64,
    confirm: bool,
    budget: usize,
) -> Result<DocumentState, String> {
    if !confirm {
        return Err("Confirm refresh of all clips sharing this audio original".into());
    }
    let mut s = time::enable(d, budget)?;
    let selected = s
        .scene
        .timing
        .audio
        .iter()
        .find(|c| c.id == id)
        .ok_or("Audio clip removed")?;
    let source = selected
        .sound
        .source
        .as_ref()
        .filter(|s| s.linked)
        .ok_or("Select a linked source")?;
    let input = load(&source.path, source.raw, true)?;
    let original = selected.source;
    if s.scene
        .timing
        .audio
        .iter()
        .any(|c| c.source == original && c.locked)
    {
        return Err("Unlock every clip using this original first".into());
    }
    let o = s
        .scene
        .timing
        .originals
        .iter_mut()
        .find(|o| o.id == original)
        .ok_or("Audio original removed")?;
    if o.rate != input.decoded.rate || o.channels != input.decoded.channels {
        return Err("Refreshed original must retain sample rate and channels".into());
    }
    o.samples = input.decoded.samples.clone();
    o.count = o.samples.len() as u32;
    o.hash = time::pcm_hash(&o.samples);
    let cache_bytes = s
        .scene
        .timing
        .audio
        .iter()
        .filter(|c| c.source == original)
        .map(|c| time::length(&c.spans) as usize * o.channels as usize * 2)
        .sum::<usize>();
    if cache_bytes
        .saturating_add(s.memory_bytes())
        .saturating_add(d.memory_bytes())
        > budget
    {
        return Err("Audio refresh exceeds snapshot budget".into());
    }
    for index in 0..s.scene.timing.audio.len() {
        let c = &s.scene.timing.audio[index];
        if c.source != original {
            continue;
        }
        let samples = time::pcm(&s.scene.timing, c)?;
        let a = &mut s.animation.as_mut().unwrap().audio[c.index];
        a.sample_count = samples.len() as u32;
        a.samples = Arc::new(samples);
        s.scene.timing.audio[index].sound.source = Some(input.source.clone());
    }
    finish(d, s, budget)
}
