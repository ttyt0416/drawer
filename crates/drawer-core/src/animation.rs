use crate::{
    document::{Background, CanvasSpec, Document, MAX_DIMENSION},
    scene::Point,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    sync::Arc,
};
pub const MAX_FRAMES: u32 = 10000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fps {
    pub numerator: u32,
    pub denominator: u32,
}
impl Default for Fps {
    fn default() -> Self {
        Self {
            numerator: 24,
            denominator: 1,
        }
    }
}
impl Fps {
    pub fn valid(self) -> bool {
        self.numerator > 0
            && self.numerator <= 100000
            && self.denominator > 0
            && self.denominator <= 100000
            && self.value() <= 240.0
    }
    pub fn value(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub offset: [i64; 2],
    #[serde(skip)]
    pub pixels: Arc<Vec<[u8; 4]>>,
}
impl Cell {
    fn metadata_valid(&self) -> bool {
        (1..=MAX_DIMENSION).contains(&self.width)
            && (1..=MAX_DIMENSION).contains(&self.height)
            && self
                .offset
                .iter()
                .all(|n| n.unsigned_abs() <= 1_000_000_000)
            && !self.name.is_empty()
            && self.name.len() <= 4096
    }
    pub fn valid(&self) -> bool {
        self.metadata_valid() && self.pixels.len() == self.width as usize * self.height as usize
    }
    pub fn pixel(&self, x: i64, y: i64) -> [u8; 4] {
        let x = x - self.offset[0];
        let y = y - self.offset[1];
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            [0; 4]
        } else {
            self.pixels[(y * self.width as i64 + x) as usize]
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exposure {
    pub frame: u32,
    pub cell: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub name: String,
    pub cells: Vec<Cell>,
    pub exposures: Vec<Exposure>,
    pub end: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    pub name: String,
    pub start: u32,
    pub rate: u32,
    pub channels: u16,
    pub sample_count: u32,
    #[serde(skip)]
    pub samples: Arc<Vec<i16>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub fps: Fps,
    pub frames: u32,
    pub tracks: Vec<Track>,
    pub audio: Vec<Audio>,
}
impl Timeline {
    pub fn metadata_valid(&self) -> bool {
        self.fps.valid()
            && (1..=MAX_FRAMES).contains(&self.frames)
            && self.tracks.len() <= 32
            && self.audio.len() <= 32
            && self.tracks.iter().map(|t| t.cells.len()).sum::<usize>() <= MAX_FRAMES as usize
            && self.tracks.iter().all(|t| {
                !t.name.is_empty()
                    && t.name.len() <= 4096
                    && !t.cells.is_empty()
                    && t.cells.iter().all(Cell::metadata_valid)
                    && !t.exposures.is_empty()
                    && t.exposures.len() <= MAX_FRAMES as usize
                    && t.end <= self.frames
                    && t.exposures
                        .iter()
                        .all(|e| e.frame < t.end && e.cell < t.cells.len())
                    && t.exposures.windows(2).all(|e| e[0].frame < e[1].frame)
            })
            && self.audio.iter().all(|a| {
                !a.name.is_empty()
                    && a.name.len() <= 4096
                    && a.start < self.frames
                    && (8000..=192000).contains(&a.rate)
                    && (1..=8).contains(&a.channels)
                    && a.sample_count > 0
                    && a.sample_count <= 32 * 1024 * 1024
                    && a.sample_count.is_multiple_of(a.channels as u32)
            })
    }
    pub fn valid(&self) -> bool {
        self.metadata_valid()
            && self.tracks.iter().flat_map(|t| &t.cells).all(Cell::valid)
            && self
                .audio
                .iter()
                .all(|a| a.sample_count as usize == a.samples.len())
    }
    pub fn memory_bytes(&self) -> usize {
        self.tracks
            .iter()
            .map(|t| {
                t.name.len()
                    + t.exposures.len() * 16
                    + t.cells
                        .iter()
                        .map(|c| c.name.len() + c.pixels.len() * 4 + 64)
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self
                .audio
                .iter()
                .map(|a| a.name.len() + a.samples.len() * 2 + 64)
                .sum::<usize>()
    }
    pub fn indexed_visible_cells(&self, frame: u32) -> impl Iterator<Item = (usize, usize, &Cell)> {
        self.tracks
            .iter()
            .enumerate()
            .filter_map(move |(track, t)| {
                if frame >= t.end {
                    return None;
                }
                let n = t.exposures.partition_point(|e| e.frame <= frame);
                let cell = t.exposures.get(n.checked_sub(1)?)?.cell;
                t.cells.get(cell).map(|c| (track, cell, c))
            })
    }
    pub fn visible_cells(&self, frame: u32) -> impl Iterator<Item = &Cell> {
        self.tracks.iter().filter_map(move |t| {
            if frame >= t.end {
                return None;
            }
            let n = t.exposures.partition_point(|e| e.frame <= frame);
            if n == 0 {
                None
            } else {
                t.cells.get(t.exposures[n - 1].cell)
            }
        })
    }
    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid animation"));
        }
        let metadata = serde_json::to_vec(self).map_err(io::Error::other)?;
        if metadata.len() > 1024 * 1024 {
            return Err(io::Error::other("Animation metadata too large"));
        }
        writer.write_all(&(metadata.len() as u32).to_le_bytes())?;
        writer.write_all(&metadata)?;
        for cell in self.tracks.iter().flat_map(|t| &t.cells) {
            for chunk in cell.pixels.chunks(4096) {
                let mut buf = [0u8; 16384];
                for (bytes, pixel) in buf.as_chunks_mut::<4>().0.iter_mut().zip(chunk) {
                    bytes.copy_from_slice(pixel);
                }
                writer.write_all(&buf[..chunk.len() * 4])?;
            }
        }
        for a in &self.audio {
            for samples in a.samples.chunks(4096) {
                let mut buf = [0u8; 8192];
                for (bytes, sample) in buf.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                    bytes.copy_from_slice(&sample.to_le_bytes());
                }
                writer.write_all(&buf[..samples.len() * 2])?;
            }
        }
        Ok(())
    }
    pub fn read(reader: &mut impl Read, budget: usize) -> io::Result<Self> {
        let mut count = [0; 4];
        reader.read_exact(&mut count)?;
        let length = u32::from_le_bytes(count) as usize;
        if length > 1024 * 1024 || length > budget {
            return Err(io::Error::other("Animation metadata exceeds budget"));
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes)?;
        let mut value: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !value.metadata_valid() {
            return Err(io::Error::other("Invalid animation metadata"));
        }
        let payload = value
            .tracks
            .iter()
            .flat_map(|t| &t.cells)
            .map(|c| c.width as usize * c.height as usize * 4)
            .sum::<usize>()
            + value
                .audio
                .iter()
                .map(|a| a.sample_count as usize * 2)
                .sum::<usize>();
        if payload.saturating_add(length) > budget {
            return Err(io::Error::other("Animation exceeds memory budget"));
        }
        for cell in value.tracks.iter_mut().flat_map(|t| &mut t.cells) {
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(cell.width as usize * cell.height as usize)
                .map_err(io::Error::other)?;
            for _ in 0..cell.width as usize * cell.height as usize {
                let mut p = [0; 4];
                reader.read_exact(&mut p)?;
                pixels.push(p);
            }
            cell.pixels = Arc::new(pixels);
        }
        for audio in &mut value.audio {
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(audio.sample_count as usize)
                .map_err(io::Error::other)?;
            for _ in 0..audio.sample_count {
                let mut p = [0; 2];
                reader.read_exact(&mut p)?;
                samples.push(i16::from_le_bytes(p));
            }
            audio.samples = Arc::new(samples);
        }
        Ok(value)
    }
}
#[derive(Clone, Debug)]
pub struct Clip {
    pub fps: Fps,
    pub cells: Vec<Cell>,
    pub source_times: Vec<f64>,
    pub audio: Option<Audio>,
}
impl Clip {
    pub fn valid(&self) -> bool {
        self.fps.valid()
            && !self.cells.is_empty()
            && self.cells.len() <= MAX_FRAMES as usize
            && self.cells.iter().all(Cell::valid)
            && self.source_times.len() == self.cells.len()
            && self.source_times.iter().all(|t| t.is_finite() && *t >= 0.0)
            && self
                .audio
                .as_ref()
                .is_none_or(|a| a.samples.len() == a.sample_count as usize)
    }
    pub fn mapping(&self, fps: Fps, start: u32) -> Result<Vec<Exposure>, String> {
        if !self.valid() || !fps.valid() {
            return Err("Invalid movie clip or frame rate".into());
        }
        let count = (self.cells.len() as u64 * fps.numerator as u64 * self.fps.denominator as u64)
            .div_ceil(fps.denominator as u64 * self.fps.numerator as u64);
        if count == 0 || count > MAX_FRAMES as u64 || start as u64 + count > MAX_FRAMES as u64 {
            return Err("The insertion exceeds the frame limit".into());
        }
        Ok((0..count as u32)
            .map(|n| Exposure {
                frame: start + n,
                cell: ((n as u64 * self.fps.numerator as u64 * fps.denominator as u64)
                    / (self.fps.denominator as u64 * fps.numerator as u64))
                    .min(self.cells.len() as u64 - 1) as usize,
            })
            .collect())
    }
    pub fn track(&self, fps: Fps, start: u32, size: Point) -> Result<Track, String> {
        let mapping = self.mapping(fps, start)?;
        let end = mapping.last().unwrap().frame + 1;
        let mut cells = self.cells.clone();
        for c in &mut cells {
            c.offset = [
                ((size[0] as i64 - c.width as i64).div_euclid(2)),
                ((size[1] as i64 - c.height as i64).div_euclid(2)),
            ];
        }
        let mut exposures = Vec::new();
        for e in mapping {
            if exposures.last().is_none_or(|p: &Exposure| p.cell != e.cell) {
                exposures.push(e);
            }
        }
        Ok(Track {
            name: "Imported movie".into(),
            cells,
            exposures,
            end,
        })
    }
    pub fn document(&self, budget: usize) -> Result<Document, String> {
        if !self.valid() {
            return Err("Invalid decoded movie".into());
        }
        let first = &self.cells[0];
        let track = self.track(self.fps, 0, [first.width as f64, first.height as f64])?;
        let timeline = Timeline {
            fps: self.fps,
            frames: track.end,
            tracks: vec![track],
            audio: self.audio.clone().into_iter().collect(),
        };
        if !timeline.valid()
            || timeline
                .memory_bytes()
                .saturating_add(first.width as usize * first.height as usize * 4)
                > budget
        {
            return Err("Animation exceeds document memory budget".into());
        }
        let mut doc = Document::create(
            CanvasSpec {
                width: first.width,
                height: first.height,
                dpi: 300.0,
                background: Background::Transparent,
            },
            budget,
        )
        .map_err(|e| e.to_string())?;
        doc.animation = Some(timeline);
        Ok(doc)
    }
    pub fn insert(&self, document: &mut Document, start: u32) -> Result<(), String> {
        let mut state = document.snapshot();
        let animation = state
            .animation
            .as_mut()
            .ok_or("Choose an animation document for timeline insertion")?;
        let track = self.track(
            animation.fps,
            start,
            [document.width as f64, document.height as f64],
        )?;
        if track.end > animation.frames {
            return Err("The imported range exceeds the current timeline".into());
        }
        animation.tracks.push(track);
        if let Some(mut audio) = self.audio.clone() {
            audio.start = start;
            animation.audio.push(audio);
        }
        document.commit_state(state)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        files,
        interpolation::Filter,
        sizing::{self, Change},
        test_support::TempDir,
    };
    use std::sync::atomic::AtomicBool;
    fn clip() -> Clip {
        Clip {
            fps: Fps {
                numerator: 2,
                denominator: 1,
            },
            cells: vec![
                Cell {
                    name: "A".into(),
                    width: 2,
                    height: 1,
                    offset: [0; 2],
                    pixels: Arc::new(vec![[255, 0, 0, 128], [9, 8, 7, 0]]),
                },
                Cell {
                    name: "B".into(),
                    width: 2,
                    height: 1,
                    offset: [0; 2],
                    pixels: Arc::new(vec![[0, 0, 255, 255], [4, 5, 6, 255]]),
                },
            ],
            source_times: vec![0.0, 0.5],
            audio: Some(Audio {
                name: "Audio".into(),
                start: 0,
                rate: 48000,
                channels: 1,
                sample_count: 3,
                samples: Arc::new(vec![-32768, 0, 32767]),
            }),
        }
    }
    #[test]
    fn independent_cells_exposures_audio_native_roundtrip_insert_undo_and_sizing() {
        let root = TempDir::new();
        let clip = clip();
        let mut doc = clip.document(100000).unwrap();
        assert!(doc.dirty && doc.path.is_none() && doc.source_path.is_none());
        let saved = doc.snapshot();
        let file = root.0.join("animation.drawer");
        files::save(&mut doc, &file).unwrap();
        let mut reopened = files::open(&file, 100000).unwrap();
        assert_eq!(reopened.snapshot(), saved);
        assert!(files::open(&file, 12).is_err());
        assert_eq!(doc.composite_pixel(0, 0), [255, 0, 0, 128]);
        let history = doc.history_counts();
        doc.current_frame = 1;
        assert_eq!(doc.composite_pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(doc.snapshot(), saved);
        assert!(!doc.dirty);
        assert_eq!(doc.history_counts(), history);
        reopened.animation.as_mut().unwrap().fps = Fps {
            numerator: 4,
            denominator: 1,
        };
        reopened.animation.as_mut().unwrap().frames = 4;
        reopened.dpi = 600.0;
        let before = reopened.snapshot();
        let old_path = reopened.path.clone();
        let mapping = clip
            .mapping(reopened.animation.as_ref().unwrap().fps, 0)
            .unwrap();
        assert_eq!(
            mapping.iter().map(|e| e.cell).collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        clip.insert(&mut reopened, 0).unwrap();
        assert_eq!(reopened.animation.as_ref().unwrap().tracks.len(), 2);
        assert_eq!(reopened.animation.as_ref().unwrap().audio.len(), 2);
        assert_eq!(reopened.dpi, 600.0);
        assert_eq!(reopened.history_counts(), (1, 0));
        assert_eq!(reopened.path, old_path);
        assert!(reopened.undo());
        assert_eq!(reopened.snapshot(), before);
        assert!(reopened.redo());
        let after = reopened.snapshot();
        assert!(clip.insert(&mut reopened, 3).is_err());
        assert_eq!(reopened.snapshot(), after);
        for change in [
            Change::Dpi(300.0),
            Change::FlipHorizontal,
            Change::Rotate90 { clockwise: true },
            Change::Resample {
                width: 4,
                height: 2,
                dpi: 300.0,
                filter: Filter::Nearest,
            },
            Change::CanvasEdges {
                left: 1,
                top: 1,
                right: 0,
                bottom: 0,
            },
        ] {
            let output = sizing::prepare(&after, change, 100000, &AtomicBool::new(false)).unwrap();
            assert!(output.animation.as_ref().unwrap().valid());
            assert_eq!(
                output.animation.as_ref().unwrap().fps,
                after.animation.as_ref().unwrap().fps
            );
            assert_eq!(
                output.animation.as_ref().unwrap().audio,
                after.animation.as_ref().unwrap().audio
            );
        }
        let mut bytes = Vec::new();
        clip.document(100000)
            .unwrap()
            .animation
            .as_ref()
            .unwrap()
            .write(&mut bytes)
            .unwrap();
        bytes.pop();
        assert!(Timeline::read(&mut bytes.as_slice(), 100000).is_err());
    }
}
