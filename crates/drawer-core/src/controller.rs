//! Named OS wheel actions. No raw HID reports or inferred gestures.
use serde::{Deserialize, Serialize};

pub const DEVICE: &str = "windows-radial";
pub const ACTIONS: [&str; 3] = ["clockwise", "counterclockwise", "click"];
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub degrees_per_step: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            degrees_per_step: 15.0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.degrees_per_step.is_finite() && (1.0..=180.0).contains(&self.degrees_per_step)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Acquired,
    Lost,
    Rotation(f64),
    Click,
}
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub serial: u64,
    pub kind: Kind,
}
#[derive(Default)]
pub struct Controller {
    settings: Settings,
    last: u64,
    acquired: bool,
    remainder: f64,
}
impl Controller {
    /// Focus/configuration changes discard fractions; loss requires acquisition.
    /// At most 32 actions per drain, with no deferred burst/backlog.
    pub fn actions(
        &mut self,
        settings: Settings,
        events: &[Event],
        allowed: bool,
    ) -> Vec<&'static str> {
        if settings != self.settings || !allowed || !settings.enabled || !settings.valid() {
            self.remainder = 0.0;
        }
        self.settings = settings;
        let mut actions = Vec::new();
        for event in events {
            if event.serial <= self.last {
                continue;
            }
            self.last = event.serial;
            match event.kind {
                Kind::Acquired => {
                    self.acquired = true;
                    self.remainder = 0.0;
                }
                Kind::Lost => {
                    self.acquired = false;
                    self.remainder = 0.0;
                }
                _ if !allowed || !settings.enabled || !settings.valid() || !self.acquired => {}
                Kind::Click => {
                    if actions.len() < 32 {
                        actions.push("click");
                    }
                }
                Kind::Rotation(delta) => {
                    if !delta.is_finite() || delta.abs() > 36000.0 {
                        self.remainder = 0.0;
                        continue;
                    }
                    let sum = self.remainder + delta;
                    let steps = (sum / settings.degrees_per_step).trunc();
                    self.remainder = sum % settings.degrees_per_step;
                    let action = if steps > 0.0 {
                        "clockwise"
                    } else {
                        "counterclockwise"
                    };
                    actions.extend(std::iter::repeat_n(
                        action,
                        (steps.abs() as usize).min(32 - actions.len()),
                    ));
                }
            }
        }
        actions
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_validate_and_old_profiles_load_without_new_bindings() {
        let settings = crate::actions::Settings::default();
        let mut json = serde_json::to_value(&settings).unwrap();
        json.as_object_mut().unwrap().remove("controller");
        let restored: crate::actions::Settings = serde_json::from_value(json).unwrap();
        assert_eq!(restored, settings);
        assert!(restored.valid());
        for degrees in [0.0, 181.0, f64::NAN, f64::INFINITY] {
            assert!(
                !Settings {
                    degrees_per_step: degrees,
                    ..Default::default()
                }
                .valid()
            );
        }
    }
    fn e(serial: u64, kind: Kind) -> Event {
        Event { serial, kind }
    }
    #[test]
    fn fractions_direction_duplicates_loss_and_fresh_acquisition() {
        let mut c = Controller::default();
        let s = Settings::default();
        assert!(c.actions(s, &[e(1, Kind::Rotation(60.0))], true).is_empty());
        assert_eq!(
            c.actions(
                s,
                &[
                    e(2, Kind::Acquired),
                    e(3, Kind::Rotation(10.0)),
                    e(4, Kind::Rotation(20.0)),
                    e(4, Kind::Rotation(20.0)),
                    e(5, Kind::Click),
                    e(5, Kind::Click)
                ],
                true
            ),
            ["clockwise", "clockwise", "click"]
        );
        assert_eq!(
            c.actions(
                s,
                &[
                    e(6, Kind::Rotation(-16.0)),
                    e(7, Kind::Lost),
                    e(8, Kind::Click),
                    e(9, Kind::Rotation(360.0))
                ],
                true
            ),
            ["counterclockwise"]
        );
        assert!(
            c.actions(
                s,
                &[e(10, Kind::Acquired), e(11, Kind::Rotation(14.0))],
                true
            )
            .is_empty()
        );
        assert_eq!(
            c.actions(s, &[e(12, Kind::Rotation(1.0))], true),
            ["clockwise"]
        );
    }
    #[test]
    fn bounds_config_focus_disabled_invalid_and_no_delayed_burst() {
        let mut c = Controller::default();
        let s = Settings::default();
        assert_eq!(
            c.actions(
                s,
                &[e(1, Kind::Acquired), e(2, Kind::Rotation(36000.0))],
                true
            )
            .len(),
            32
        );
        assert!(c.actions(s, &[], true).is_empty());
        assert!(
            c.actions(
                s,
                &[
                    e(3, Kind::Rotation(f64::NAN)),
                    e(4, Kind::Rotation(f64::INFINITY)),
                    e(5, Kind::Rotation(f64::MAX)),
                    e(6, Kind::Rotation(14.0))
                ],
                true
            )
            .is_empty()
        );
        c.actions(s, &[], false);
        assert!(c.actions(s, &[e(7, Kind::Rotation(1.0))], true).is_empty());
        let changed = Settings {
            degrees_per_step: 2.0,
            ..s
        };
        assert!(
            c.actions(changed, &[e(8, Kind::Rotation(1.0))], true)
                .is_empty()
        );
        assert_eq!(
            c.actions(changed, &[e(9, Kind::Rotation(1.0))], true),
            ["clockwise"]
        );
        assert!(
            c.actions(
                Settings {
                    enabled: false,
                    ..changed
                },
                &[e(10, Kind::Click)],
                true
            )
            .is_empty()
        );
        assert!(
            c.actions(
                Settings {
                    degrees_per_step: 0.0,
                    ..s
                },
                &[e(11, Kind::Click)],
                true
            )
            .is_empty()
        );
        c.reset();
        assert!(c.actions(s, &[e(1, Kind::Click)], true).is_empty());
        assert_eq!(
            c.actions(s, &[e(2, Kind::Acquired), e(3, Kind::Click)], true),
            ["click"]
        );
    }
}
