//! Bounded local virtual input records. They are never evidence of hardware validation.
use crate::{
    assets::BrushTip,
    pointer_input::{Event, Kind, Phase, Source},
    pressure,
};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub micros: u64,
    pub event: Event,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub tool: BrushTip,
    pub size: f64,
    pub aspect: f64,
    pub angle: f64,
    pub smoothing: [f64; 4],
    pub color: [u8; 4],
    pub background: [u8; 4],
    pub pressure: pressure::Settings,
    pub pointer: crate::pointer_input::Settings,
    pub expected_strokes: usize,
    pub expected_checksum: Option<String>,
    pub pen_backend_available: bool,
    pub samples: Vec<Sample>,
}
impl Default for Sequence {
    fn default() -> Self {
        let source = Source {
            kind: Kind::Pen,
            device: 1,
            contact: 1,
        };
        let mut pressure = pressure::Settings::default();
        pressure.tools[0].opacity = true;
        Self {
            version: 1,
            width: 64,
            height: 48,
            tool: BrushTip::Pen,
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
            smoothing: [0.0; 4],
            color: [30, 80, 160, 255],
            background: [0; 4],
            pressure,
            pointer: Default::default(),
            expected_strokes: 1,
            expected_checksum: None,
            pen_backend_available: true,
            samples: [
                (0, Phase::Down, [8.5, 20.5], 0.0),
                (1000, Phase::Move, [24.5, 20.5], 1.0),
                (2000, Phase::Move, [48.5, 20.5], 0.5),
                (3000, Phase::Up, [52.5, 20.5], 0.0),
            ]
            .into_iter()
            .map(|(micros, phase, position, p)| Sample {
                micros,
                event: Event {
                    source,
                    phase,
                    position,
                    pressure: Some(p),
                    attributes: Default::default(),
                    origin: Default::default(),
                },
            })
            .collect(),
        }
    }
}
impl Sequence {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !(1..=256).contains(&self.width)
            || !(1..=256).contains(&self.height)
            || !(0.1..=256.0).contains(&self.size)
            || !(0.05..=1.0).contains(&self.aspect)
            || !(0.0..=360.0).contains(&self.angle)
            || self.smoothing.iter().any(|v| !(0.0..=0.95).contains(v))
            || !self.pressure.valid()
            || !self.pointer.valid()
            || self.expected_strokes > 100
            || self.samples.is_empty()
            || self.samples.len() > 4096
        {
            return Err("Invalid virtual record (1–256px, up to 4096 events / 100 strokes)".into());
        }
        if self
            .expected_checksum
            .as_ref()
            .is_some_and(|s| s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Expected checksum must contain 16 hexadecimal digits".into());
        }
        let mut last = 0;
        let mut frames = 0;
        let mut starts = 0;
        for (n, s) in self.samples.iter().enumerate() {
            if s.micros > 86_400_000_000 || (n > 0 && s.micros < last) {
                return Err(format!(
                    "Event {n}: times must be nondecreasing and within 24 hours"
                ));
            }
            if !s.event.valid()
                || s.event
                    .attributes
                    .tilt
                    .is_some_and(|v| v.iter().any(|n| !n.is_finite()))
            {
                return Err(format!("Event {n}: invalid coordinate, pressure or tilt"));
            }
            if n == 0 || s.micros != last {
                frames += 1;
            }
            starts += usize::from(s.event.phase == Phase::Down);
            last = s.micros;
        }
        if frames > 512 || starts > 100 {
            return Err("Virtual record exceeds 512 frames or 100 starts".into());
        }
        Ok(())
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > 2 * 1024 * 1024 {
            return Err(io::Error::other("Virtual record exceeds 2 MiB"));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        let s: Self = if value.get("sequence").is_some() {
            serde_json::from_value::<Record>(value)
                .map_err(io::Error::other)?
                .sequence
        } else {
            serde_json::from_value(value).map_err(io::Error::other)?
        };
        s.validate().map_err(io::Error::other)?;
        Ok(s)
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.validate().map_err(io::Error::other)?;
        crate::workspaces::write_json(path, self)
    }
}
/// Deterministic RGBA byte fingerprint for replay comparison, not a security hash.
pub fn checksum(pixels: &[[u8; 4]]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for value in pixels.iter().flatten() {
        hash ^= *value as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub sequence: Sequence,
    pub actual_strokes: usize,
    pub checksum: String,
    pub incomplete_cancelled: bool,
    pub passed: bool,
    pub frames: usize,
}
impl Record {
    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.sequence.validate().map_err(io::Error::other)?;
        crate::workspaces::write_json(path, self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_records_validate_pressure_jumps_missing_end_and_reject_reverse_time() {
        let mut s = Sequence::default();
        s.validate().unwrap();
        s.samples.pop();
        s.expected_strokes = 0;
        s.validate().unwrap();
        s.samples[1].micros = 3000;
        assert!(s.validate().is_err());
        s = Sequence::default();
        s.samples[1].event.pressure = Some(-1.0);
        assert!(s.validate().is_err());
        s.samples[1].event.pressure = Some(f64::NAN);
        assert!(s.validate().is_err());
        s = Sequence::default();
        s.width = 0;
        assert!(s.validate().is_err());
        assert_eq!(checksum(&[[0; 4]]), checksum(&[[0; 4]]));
        assert_ne!(checksum(&[[0; 4]]), checksum(&[[0, 0, 0, 1]]));
    }
}
