//! Per-application pen click and coordinate policy, shared by native and replay.
use crate::{
    assets::BrushTip,
    pointer_input::{Attributes, Event, Kind, Phase, Source},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Absolute,
    Relative,
}
#[cfg(test)]
mod tests {
    use super::*;
    const BOUNDS: [[f64; 2]; 2] = [[-100.0, -50.0], [100.0, 80.0]];
    fn event(phase: Phase, p: Option<f64>, position: [f64; 2]) -> Event {
        Event {
            source: Source {
                kind: Kind::Pen,
                device: 1,
                contact: 2,
            },
            phase,
            position,
            pressure: p,
            attributes: Default::default(),
            origin: Default::default(),
        }
    }
    fn send(a: &mut Adapter, phase: Phase, p: Option<f64>, position: [f64; 2]) -> Vec<Event> {
        a.event(event(phase, p, position), BOUNDS, [10.0, 20.0])
    }
    #[test]
    fn click_threshold_crossing_missing_pressure_release_and_detach() {
        let mut a = Adapter::default();
        a.configure(Settings {
            click_pressure: Some(0.5),
            ..Default::default()
        });
        assert!(send(&mut a, Phase::Down, Some(0.499), [0.0; 2]).is_empty());
        assert_eq!(
            send(&mut a, Phase::Move, Some(0.5), [1.0; 2])[0].phase,
            Phase::Down
        );
        assert_eq!(
            send(&mut a, Phase::Move, Some(0.501), [2.0; 2])[0].phase,
            Phase::Move
        );
        assert_eq!(
            send(&mut a, Phase::Move, Some(0.499), [3.0; 2])[0].phase,
            Phase::Up
        );
        assert_eq!(
            send(&mut a, Phase::Move, None, [4.0; 2])[0].phase,
            Phase::Down
        );
        assert_eq!(
            send(&mut a, Phase::Cancel, None, [0.0; 2])[0].phase,
            Phase::Cancel
        );
        assert!(send(&mut a, Phase::Move, Some(1.0), [0.0; 2]).is_empty());
        a.configure(Settings::default());
        assert_eq!(
            send(&mut a, Phase::Down, Some(0.0), [0.0; 2])[0].pressure,
            Some(0.0)
        );
        assert_eq!(
            send(&mut a, Phase::Down, Some(1.0), [0.0; 2])[0].phase,
            Phase::Cancel
        );
        assert!(send(&mut a, Phase::Move, Some(1.0), [1.0; 2]).is_empty());
        send(&mut a, Phase::Up, None, [1.0; 2]);
        assert_eq!(
            send(&mut a, Phase::Down, None, [0.0; 2])[0].phase,
            Phase::Down
        );
        assert_eq!(
            send(&mut a, Phase::Move, Some(1.0), [f64::NAN, 0.0])[0].phase,
            Phase::Cancel
        );
        assert!(send(&mut a, Phase::Move, None, [0.0; 2]).is_empty());
    }
    #[test]
    fn relative_speed_cursor_retention_clipping_and_absolute_mouse() {
        let mut a = Adapter::default();
        a.configure(Settings {
            mode: Mode::Relative,
            speed: 0.5,
            ..Default::default()
        });
        assert_eq!(
            send(&mut a, Phase::Down, None, [50.0, 60.0])[0].position,
            [10.0, 20.0]
        );
        assert_eq!(
            send(&mut a, Phase::Move, None, [70.0, 40.0])[0].position,
            [20.0, 10.0]
        );
        assert_eq!(
            send(&mut a, Phase::Up, None, [70.0, 40.0])[0].position,
            [20.0, 10.0]
        );
        assert_eq!(
            send(&mut a, Phase::Down, None, [-90.0, -40.0])[0].position,
            [20.0, 10.0]
        );
        assert_eq!(
            send(&mut a, Phase::Move, None, [1e10, -1e10])[0].position,
            [100.0, -50.0]
        );
        let mut mouse = event(Phase::Down, Some(0.0), [-3.0, 4.0]);
        mouse.source = Source::MOUSE;
        assert_eq!(a.event(mouse, BOUNDS, [0.0; 2]), vec![mouse]);
        a.reset();
        assert!(send(&mut a, Phase::Move, None, [0.0; 2]).is_empty());
        a.configure(Settings::default());
        assert_eq!(
            send(&mut a, Phase::Down, None, [-3.0, 4.0])[0].position,
            [-3.0, 4.0]
        );
    }
    #[test]
    fn button_mapping_priority_unsupported_defaults_and_ignore_until_lift() {
        let mut s = Settings::default();
        s.buttons[0] = Action::Eraser;
        s.buttons[1] = Action::CurrentBrush;
        s.buttons[31] = Action::Ignore;
        let mut attrs = Attributes {
            buttons: Some(3),
            ..Default::default()
        };
        assert_eq!(s.action(attrs), Action::Eraser);
        assert_eq!(s.action(Attributes::default()), Action::Default);
        assert_eq!(s.tip(attrs, BrushTip::Pen, BrushTip::Pen), BrushTip::Eraser);
        attrs.buttons = Some(2);
        assert_eq!(s.tip(attrs, BrushTip::Pen, BrushTip::Eraser), BrushTip::Pen);
        attrs.buttons = Some(0x8000_0001);
        assert_eq!(s.action(attrs), Action::Ignore);
        let mut a = Adapter::default();
        a.configure(s);
        let mut e = event(Phase::Down, None, [0.0; 2]);
        e.attributes = attrs;
        assert!(a.event(e, BOUNDS, [0.0; 2]).is_empty());
        assert!(send(&mut a, Phase::Move, None, [1.0; 2]).is_empty());
        send(&mut a, Phase::Up, None, [1.0; 2]);
        assert_eq!(
            send(&mut a, Phase::Down, None, [1.0; 2])[0].phase,
            Phase::Down
        );
        assert!(
            !Settings {
                speed: f64::NAN,
                ..s
            }
            .valid()
        );
        assert!(
            !Settings {
                click_pressure: Some(-0.1),
                ..s
            }
            .valid()
        );
        let legacy = r#"{"mouse":{"enabled":true,"stabilization":null},"pen":{"enabled":true,"stabilization":null}}"#;
        let parsed: crate::pointer_input::Settings = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.controls, Settings::default());
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    #[default]
    Default,
    CurrentBrush,
    Eraser,
    Ignore,
}
impl Action {
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::CurrentBrush,
        Self::Eraser,
        Self::Ignore,
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub click_pressure: Option<f64>,
    pub mode: Mode,
    pub speed: f64,
    pub buttons: [Action; 32],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            click_pressure: None,
            mode: Mode::Absolute,
            speed: 1.0,
            buttons: [Action::Default; 32],
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (0.05..=10.0).contains(&self.speed)
            && self.click_pressure.is_none_or(|v| (0.0..=1.0).contains(&v))
    }
    pub fn action(self, attributes: Attributes) -> Action {
        let bits = attributes.buttons.unwrap_or(0);
        let mut action = Action::Default;
        for (n, &value) in self.buttons.iter().enumerate() {
            if bits & (1 << n) == 0 {
                continue;
            }
            // Explicit ignore wins; eraser wins over the current brush.
            if value == Action::Ignore {
                return value;
            }
            if value == Action::Eraser
                || (value == Action::CurrentBrush && action == Action::Default)
            {
                action = value;
            }
        }
        action
    }
    pub fn tip(self, attributes: Attributes, current: BrushTip, default: BrushTip) -> BrushTip {
        match self.action(attributes) {
            Action::Eraser => BrushTip::Eraser,
            Action::CurrentBrush => current,
            _ => default,
        }
    }
}
struct Contact {
    raw: [f64; 2],
    position: [f64; 2],
    last: Event,
    clicked: bool,
    blocked: bool,
}
#[derive(Default)]
pub struct Adapter {
    settings: Option<Settings>,
    contacts: HashMap<Source, Contact>,
    cursor: Option<[f64; 2]>,
}
impl Adapter {
    pub fn cursor(&self) -> Option<[f64; 2]> {
        self.cursor
    }
    pub fn reset(&mut self) {
        self.contacts.clear();
        self.cursor = None;
    }
    pub fn configure(&mut self, settings: Settings) -> Vec<Event> {
        if self.settings == Some(settings) {
            return vec![];
        }
        let cancelled = self
            .contacts
            .drain()
            .filter_map(|(_, mut c)| {
                if !c.clicked {
                    return None;
                }
                c.last.phase = Phase::Cancel;
                Some(c.last)
            })
            .collect();
        self.cursor = None;
        self.settings = Some(settings);
        cancelled
    }
    pub fn event(&mut self, mut e: Event, bounds: [[f64; 2]; 2], anchor: [f64; 2]) -> Vec<Event> {
        if e.source.kind != Kind::Pen {
            return vec![e];
        }
        let s = self.settings.unwrap_or_default();
        if !e.valid()
            || !s.valid()
            || bounds
                .iter()
                .flatten()
                .chain(anchor.iter())
                .any(|v| !v.is_finite() || v.abs() > 1e12)
            || (0..2).any(|n| bounds[0][n] >= bounds[1][n])
        {
            return self
                .contacts
                .remove(&e.source)
                .filter(|c| c.clicked)
                .map(|mut c| {
                    c.last.phase = Phase::Cancel;
                    c.last
                })
                .into_iter()
                .collect();
        }
        let source = e.source;
        if e.phase == Phase::Down {
            if let Some(c) = self.contacts.get_mut(&source) {
                c.blocked = true;
                let clicked = std::mem::replace(&mut c.clicked, false);
                return if clicked {
                    let mut cancel = c.last;
                    cancel.phase = Phase::Cancel;
                    vec![cancel]
                } else {
                    vec![]
                };
            }
            if self.contacts.len() >= 32 {
                return vec![];
            }
            let position = if s.mode == Mode::Relative {
                let p = self.cursor.unwrap_or(anchor);
                std::array::from_fn(|n| p[n].clamp(bounds[0][n], bounds[1][n]))
            } else {
                e.position
            };
            self.contacts.insert(
                source,
                Contact {
                    raw: e.position,
                    position,
                    last: e,
                    clicked: false,
                    blocked: false,
                },
            );
        }
        let Some(c) = self.contacts.get_mut(&source) else {
            return vec![];
        };
        let raw = e.position;
        if s.mode == Mode::Relative {
            c.position = std::array::from_fn(|n| {
                (c.position[n] + (raw[n] - c.raw[n]) * s.speed).clamp(bounds[0][n], bounds[1][n])
            });
            e.position = c.position;
            self.cursor = Some(c.position);
        }
        c.raw = raw;
        let phase = e.phase;
        if matches!(phase, Phase::Up | Phase::Cancel) {
            let clicked = c.clicked;
            self.contacts.remove(&source);
            return if clicked { vec![e] } else { vec![] };
        }
        if c.blocked {
            return vec![];
        }
        let down = s.click_pressure.is_none_or(|min| e.pressure_value() >= min);
        if down && !c.clicked {
            if s.action(e.attributes) == Action::Ignore {
                c.blocked = true;
                return vec![];
            }
            e.phase = Phase::Down;
            c.clicked = true;
        } else if !down && c.clicked {
            e.phase = Phase::Up;
            c.clicked = false;
        } else if down {
            e.phase = Phase::Move;
        } else {
            c.last = e;
            return vec![];
        }
        c.last = e;
        vec![e]
    }
}
