//! Device identity and contact ownership; independent of a particular window backend.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Mouse,
    Pen,
    Touch,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: Kind,
    pub device: u64,
    pub contact: u64,
}
impl Source {
    /// The GUI backend supplies an aggregate mouse, not a physical mouse ID.
    pub const MOUSE: Self = Self {
        kind: Kind::Mouse,
        device: 0,
        contact: 0,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    /// Absolute X/Y tilt from perpendicular in radians.
    pub tilt: Option<[f64; 2]>,
    pub eraser: Option<bool>,
    /// Any button reported by the pen backend; no assumed hardware button number.
    pub button: Option<bool>,
}
impl Attributes {
    pub fn normalized(self) -> Self {
        Self {
            tilt: self
                .tilt
                .filter(|v| v.iter().all(|n| n.is_finite()))
                .map(|v| {
                    v.map(|n| n.clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2))
                }),
            ..self
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PenFeatures {
    pub tilt_angle: bool,
    pub eraser: bool,
    pub button_eraser: bool,
}
impl Default for PenFeatures {
    fn default() -> Self {
        Self {
            tilt_angle: false,
            eraser: true,
            button_eraser: false,
        }
    }
}
impl PenFeatures {
    pub fn tip(
        self,
        kind: Kind,
        attributes: Attributes,
        current: crate::assets::BrushTip,
    ) -> crate::assets::BrushTip {
        if kind == Kind::Pen
            && ((self.eraser && attributes.eraser == Some(true))
                || (self.button_eraser && attributes.button == Some(true)))
        {
            crate::assets::BrushTip::Eraser
        } else {
            current
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub source: Source,
    pub phase: Phase,
    /// Window logical pixels. Conversion to document coordinates happens in the owning view.
    pub position: [f64; 2],
    pub pressure: Option<f64>,
    #[serde(default)]
    pub attributes: Attributes,
}
impl Event {
    pub fn valid(self) -> bool {
        self.position
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e12)
            && self.pressure.is_none_or(|v| (0.0..=1.0).contains(&v))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSettings {
    pub enabled: bool,
    pub stabilization: Option<f64>,
}
impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            stabilization: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mouse: DeviceSettings,
    pub pen: DeviceSettings,
    #[serde(default)]
    pub features: PenFeatures,
}
impl Settings {
    pub fn valid(self) -> bool {
        [self.mouse, self.pen]
            .into_iter()
            .all(|s| s.stabilization.is_none_or(|v| (0.0..=0.95).contains(&v)))
    }
    pub fn for_kind(self, kind: Kind) -> Option<DeviceSettings> {
        match kind {
            Kind::Mouse => Some(self.mouse),
            Kind::Pen => Some(self.pen),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Begin,
    Sample,
    Finish,
    Cancel,
    Ignore,
    Busy,
}
#[derive(Default)]
pub struct Ownership {
    owner: Option<Source>,
}
impl Ownership {
    pub fn owner(&self) -> Option<Source> {
        self.owner
    }
    pub fn clear(&mut self) {
        self.owner = None;
    }
    pub fn receive(&mut self, event: Event, settings: Settings) -> Decision {
        if !event.valid() || !settings.valid() {
            return Decision::Ignore;
        }
        if let Some(owner) = self.owner {
            if owner != event.source {
                return if event.phase == Phase::Down {
                    Decision::Busy
                } else {
                    Decision::Ignore
                };
            }
            match event.phase {
                Phase::Move => Decision::Sample,
                Phase::Up => {
                    self.clear();
                    Decision::Finish
                }
                // A repeated down without a corresponding up is a lost contact, not a second stroke.
                Phase::Down | Phase::Cancel => {
                    self.clear();
                    Decision::Cancel
                }
            }
        } else if event.phase == Phase::Down
            && settings
                .for_kind(event.source.kind)
                .is_some_and(|s| s.enabled)
        {
            self.owner = Some(event.source);
            Decision::Begin
        } else {
            Decision::Ignore
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_pen_attributes_normalize_tilt_and_do_not_change_other_kinds() {
        use crate::assets::BrushTip;
        let a = Attributes {
            tilt: Some([99.0, -99.0]),
            eraser: Some(true),
            button: Some(true),
        }
        .normalized();
        assert_eq!(
            a.tilt,
            Some([std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2])
        );
        assert_eq!(
            Attributes {
                tilt: Some([f64::NAN, 0.0]),
                ..a
            }
            .normalized()
            .tilt,
            None
        );
        let features = PenFeatures {
            tilt_angle: true,
            eraser: true,
            button_eraser: true,
        };
        assert_eq!(features.tip(Kind::Pen, a, BrushTip::Pen), BrushTip::Eraser);
        for kind in [Kind::Mouse, Kind::Touch, Kind::Unknown] {
            assert_eq!(features.tip(kind, a, BrushTip::Pen), BrushTip::Pen);
        }
        let off = PenFeatures {
            tilt_angle: false,
            eraser: false,
            button_eraser: false,
        };
        assert_eq!(off.tip(Kind::Pen, a, BrushTip::Pencil), BrushTip::Pencil);
        assert_eq!(
            features.tip(Kind::Pen, Attributes::default(), BrushTip::Pencil),
            BrushTip::Pencil
        );
        let old = r#"{"mouse":{"enabled":true,"stabilization":null},"pen":{"enabled":true,"stabilization":null}}"#;
        let restored: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(restored.features, PenFeatures::default());
    }
    fn event(source: Source, phase: Phase) -> Event {
        Event {
            source,
            phase,
            position: [1.0, 2.0],
            pressure: None,
            attributes: Default::default(),
        }
    }
    #[test]
    fn identity_order_ownership_release_cancel_and_separate_settings() {
        let pen = Source {
            kind: Kind::Pen,
            device: 67,
            contact: 923,
        };
        let other_pen = Source {
            contact: 924,
            ..pen
        };
        let mut settings = Settings::default();
        let mut o = Ownership::default();
        for (source, phase, expected) in [
            (pen, Phase::Down, Decision::Begin),
            (Source::MOUSE, Phase::Down, Decision::Busy),
            (other_pen, Phase::Up, Decision::Ignore),
            (pen, Phase::Move, Decision::Sample),
            (Source::MOUSE, Phase::Up, Decision::Ignore),
            (pen, Phase::Up, Decision::Finish),
            (Source::MOUSE, Phase::Move, Decision::Ignore),
            (Source::MOUSE, Phase::Down, Decision::Begin),
            (pen, Phase::Down, Decision::Busy),
            (Source::MOUSE, Phase::Cancel, Decision::Cancel),
        ] {
            assert_eq!(o.receive(event(source, phase), settings), expected);
        }
        assert_eq!(o.owner(), None);
        settings.pen.enabled = false;
        settings.mouse.stabilization = Some(0.4);
        assert_eq!(
            o.receive(event(pen, Phase::Down), settings),
            Decision::Ignore
        );
        assert_eq!(settings.pen.stabilization, None);
        assert_eq!(
            o.receive(event(Source::MOUSE, Phase::Down), settings),
            Decision::Begin
        );
        assert_eq!(
            o.receive(event(Source::MOUSE, Phase::Down), settings),
            Decision::Cancel
        );
        assert_eq!(
            o.receive(
                event(
                    Source {
                        kind: Kind::Touch,
                        ..pen
                    },
                    Phase::Down
                ),
                settings
            ),
            Decision::Ignore
        );
        let mut bad = event(pen, Phase::Down);
        bad.position[0] = f64::NAN;
        assert_eq!(o.receive(bad, settings), Decision::Ignore);
        settings.pen.stabilization = Some(f64::INFINITY);
        assert!(!settings.valid());
    }
}
