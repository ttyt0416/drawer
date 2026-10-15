//! Local mapping from an explicit device-normalized tablet area to a drawing
//! canvas or physical monitor. Device absence never becomes a guessed screen UV.
use crate::{
    display_view::Monitor,
    pointer_input::{Event, Kind, Phase},
    scene::Rect,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Target {
    Canvas,
    Monitor { position: [i32; 2], size: [u32; 2] },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub source: Rect,
    pub target: Target,
    pub touch_offset: [f64; 2],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            source: Rect {
                min: [0.0; 2],
                max: [1.0; 2],
            },
            target: Target::Canvas,
            touch_offset: [0.0; 2],
        }
    }
}
fn rect_valid(r: Rect) -> bool {
    r.min
        .iter()
        .chain(r.max.iter())
        .all(|v| v.is_finite() && v.abs() <= 1e12)
        && (0..2).all(|n| r.max[n] - r.min[n] >= 1e-6)
}
impl Settings {
    pub fn valid(self) -> bool {
        rect_valid(self.source)
            && self.source.min.iter().all(|v| *v >= 0.0)
            && self.source.max.iter().all(|v| *v <= 1.0)
            && self
                .touch_offset
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 4096.0)
            && match self.target {
                Target::Canvas => true,
                Target::Monitor { size, .. } => size.iter().all(|n| (1..=65536).contains(n)),
            }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub origin: [f64; 2],
    pub scale: f64,
    pub canvas: Option<Rect>,
    pub monitors: Vec<Monitor>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            origin: [0.0; 2],
            scale: 1.0,
            canvas: None,
            monitors: vec![],
        }
    }
}
pub fn normalized(point: [f64; 2], bounds: Rect) -> Option<[f64; 2]> {
    if !rect_valid(bounds) || point.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(std::array::from_fn(|n| {
        (point[n] - bounds.min[n]) / (bounds.max[n] - bounds.min[n])
    }))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    MissingDeviceArea,
    OutsideArea,
    MissingCanvas,
    MissingMonitor,
}
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => "Invalid tablet mapping coordinates",
            Self::MissingDeviceArea => {
                "This input does not report a tablet area; mapping requires device data or an explicit virtual test"
            }
            Self::OutsideArea => {
                "Pen is outside the selected tablet area; lift before drawing again"
            }
            Self::MissingCanvas => "Tablet mapping needs a visible drawing canvas",
            Self::MissingMonitor => "Selected mapping monitor is unavailable; select it again",
        }
    }
}
impl Context {
    pub fn target(&self, target: Target) -> Result<Rect, Error> {
        if !self.scale.is_finite()
            || !(0.05..=32.0).contains(&self.scale)
            || self.origin.iter().any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err(Error::Invalid);
        }
        let r = match target {
            Target::Canvas => self.canvas.ok_or(Error::MissingCanvas)?,
            Target::Monitor { position, size } => {
                if !self
                    .monitors
                    .iter()
                    .any(|m| m.valid() && m.position == position && m.size == size)
                {
                    return Err(Error::MissingMonitor);
                }
                let min =
                    std::array::from_fn(|n| (position[n] as f64 - self.origin[n]) / self.scale);
                Rect {
                    min,
                    max: std::array::from_fn(|n| min[n] + size[n] as f64 / self.scale),
                }
            }
        };
        if !rect_valid(r) {
            return Err(Error::Invalid);
        }
        Ok(r)
    }
    pub fn map(&self, s: Settings, mut event: Event) -> Result<Event, Error> {
        if event.phase == Phase::Cancel {
            return Ok(event);
        }
        if !s.valid() || !event.valid() {
            return Err(Error::Invalid);
        }
        if event.source.kind == Kind::Touch {
            if !self.scale.is_finite() || self.scale <= 0.0 {
                return Err(Error::Invalid);
            }
            event.position =
                std::array::from_fn(|n| event.position[n] + s.touch_offset[n] / self.scale);
        } else if event.source.kind == Kind::Pen && s.enabled {
            let uv = event.attributes.tablet.ok_or(Error::MissingDeviceArea)?;
            if uv.iter().any(|v| !v.is_finite()) {
                return Err(Error::Invalid);
            }
            if (0..2).any(|n| uv[n] < s.source.min[n] || uv[n] > s.source.max[n]) {
                return Err(Error::OutsideArea);
            }
            let target = self.target(s.target)?;
            event.position = std::array::from_fn(|n| {
                target.min[n]
                    + (uv[n] - s.source.min[n]) / (s.source.max[n] - s.source.min[n])
                        * (target.max[n] - target.min[n])
            });
        }
        if !event.valid() {
            return Err(Error::Invalid);
        }
        Ok(event)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer_input::{Attributes, Source};
    fn event(uv: Option<[f64; 2]>) -> Event {
        Event {
            source: Source {
                kind: Kind::Pen,
                device: 1,
                contact: 1,
            },
            phase: Phase::Down,
            position: [7.0, 9.0],
            pressure: None,
            attributes: Attributes {
                tablet: uv,
                ..Default::default()
            },
            origin: Default::default(),
        }
    }
    #[test]
    fn crop_edges_negative_monitor_coordinates_and_receiving_dpi() {
        let monitor = Monitor {
            key: "negative".into(),
            index: 1,
            position: [-3840, -200],
            size: [3840, 2160],
            scale: 2.0,
            primary: false,
        };
        let s = Settings {
            enabled: true,
            source: Rect {
                min: [0.25; 2],
                max: [0.75; 2],
            },
            target: Target::Monitor {
                position: monitor.position,
                size: monitor.size,
            },
            ..Default::default()
        };
        for scale in [1.0, 1.5, 2.0, 2.5] {
            let ctx = Context {
                origin: [-3000.0, 100.0],
                scale,
                canvas: None,
                monitors: vec![monitor.clone()],
            };
            for (uv, expected) in [
                ([0.25; 2], [-3840.0, -200.0]),
                ([0.5; 2], [-1920.0, 880.0]),
                ([0.75; 2], [0.0, 1960.0]),
            ] {
                let p = ctx.map(s, event(Some(uv))).unwrap().position;
                for n in 0..2 {
                    assert!((p[n] * scale + ctx.origin[n] - expected[n]).abs() < 1e-9);
                }
            }
            assert_eq!(
                ctx.map(s, event(Some([0.249, 0.5]))),
                Err(Error::OutsideArea)
            );
            let mut removed = ctx.clone();
            removed.monitors.clear();
            assert_eq!(
                removed.map(s, event(Some([0.5; 2]))),
                Err(Error::MissingMonitor)
            );
        }
    }
    #[test]
    fn canvas_and_touch_offsets_do_not_guess_device_data_or_change_mouse() {
        let ctx = Context {
            canvas: Some(Rect {
                min: [10.0, 20.0],
                max: [210.0, 120.0],
            }),
            scale: 2.0,
            ..Default::default()
        };
        let s = Settings {
            enabled: true,
            touch_offset: [20.0, -10.0],
            ..Default::default()
        };
        assert_eq!(
            ctx.map(s, event(Some([0.5; 2]))).unwrap().position,
            [110.0, 70.0]
        );
        assert_eq!(ctx.map(s, event(None)), Err(Error::MissingDeviceArea));
        let mut mouse = event(None);
        mouse.source = Source::MOUSE;
        assert_eq!(ctx.map(s, mouse).unwrap(), mouse);
        mouse.source.kind = Kind::Touch;
        assert_eq!(ctx.map(s, mouse).unwrap().position, [17.0, 4.0]);
        mouse.source.kind = Kind::Unknown;
        assert_eq!(ctx.map(s, mouse).unwrap(), mouse);
        let invalid = Settings {
            source: Rect {
                min: [0.5; 2],
                max: [0.5; 2],
            },
            ..s
        };
        assert!(!invalid.valid());
        assert_eq!(ctx.map(invalid, event(Some([0.5; 2]))), Err(Error::Invalid));
        assert_eq!(
            normalized(
                [500.0, 250.0],
                Rect {
                    min: [0.0; 2],
                    max: [1000.0, 500.0]
                }
            ),
            Some([0.5; 2])
        );
        assert!(
            normalized(
                [0.0; 2],
                Rect {
                    min: [0.0; 2],
                    max: [0.0; 2]
                }
            )
            .is_none()
        );
        let mut missing = ctx.clone();
        missing.canvas = None;
        assert_eq!(
            missing.map(s, event(Some([0.5; 2]))),
            Err(Error::MissingCanvas)
        );
        assert!(
            !Settings {
                touch_offset: [f64::NAN, 0.0],
                ..s
            }
            .valid()
        );
    }
}
