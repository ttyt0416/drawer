//! Touch manipulation never edits pixels. Only explicitly enabled single-finger
//! drawing is forwarded to the same contact/undo pipeline as other devices.
use crate::{
    pointer_input::{Event, Kind, Origin, Phase, Source},
    view::View,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub gestures: bool,
    pub finger_drawing: bool,
    pub reject_palms: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            gestures: true,
            finger_drawing: false,
            reject_palms: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub view: u64,
    pub from: [[f64; 2]; 2],
    pub to: [[f64; 2]; 2],
}
impl Motion {
    pub fn apply(
        self,
        view: &mut View,
        size: [f64; 2],
        center: [f64; 2],
        ppp: f64,
    ) -> Result<(), String> {
        if self
            .from
            .iter()
            .chain(self.to.iter())
            .flatten()
            .chain(size.iter())
            .chain(center.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e12)
            || size.iter().any(|v| *v <= 0.0)
            || !ppp.is_finite()
            || ppp <= 0.0
            || !(0.01..=64.0).contains(&view.zoom)
            || !view.angle.is_finite()
            || view.pan.iter().any(|v| !v.is_finite())
        {
            return Err("Invalid touch manipulation".into());
        }
        let middle = |p: [[f64; 2]; 2]| [(p[0][0] + p[1][0]) * 0.5, (p[0][1] + p[1][1]) * 0.5];
        let a = middle(self.from);
        let b = middle(self.to);
        let doc = view.screen_to_document(a, size, center, ppp);
        let vector = |p: [[f64; 2]; 2]| [p[1][0] - p[0][0], p[1][1] - p[0][1]];
        let u = vector(self.from);
        let v = vector(self.to);
        let before = u[0].hypot(u[1]);
        let after = v[0].hypot(v[1]);
        if before * ppp >= 2.0 && after * ppp >= 2.0 {
            view.zoom = (view.zoom * (after / before).clamp(1.0 / 16.0, 16.0)).clamp(0.01, 64.0);
            view.rotate(
                view.angle
                    + (u[0] * v[1] - u[1] * v[0])
                        .atan2(u[0] * v[0] + u[1] * v[1])
                        .to_degrees(),
            );
        }
        let actual = view.document_to_screen(doc, size, center, ppp);
        view.pan_by([b[0] - actual[0], b[1] - actual[1]]);
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Pointer(Event),
    Motion(Motion),
}
struct Contact {
    event: Event,
    blocked: bool,
}
#[derive(Default)]
pub struct Controller {
    settings: Settings,
    contacts: Vec<Contact>,
    drawing: Option<Event>,
    pens: HashSet<Source>,
    palms: HashSet<Source>,
    mouse: bool,
    overflow: bool,
    view: Option<u64>,
}
impl Controller {
    pub fn gui_drag_blocked(&self) -> bool {
        (!self.contacts.is_empty() || !self.palms.is_empty()) && !self.mouse && self.pens.is_empty()
    }
    pub fn clear_touches(&mut self) -> Vec<Effect> {
        for c in &mut self.contacts {
            c.blocked = true;
        }
        self.view = None;
        self.drawing
            .take()
            .map(|mut e| {
                e.phase = Phase::Cancel;
                Effect::Pointer(e)
            })
            .into_iter()
            .collect()
    }
    pub fn reset(&mut self) -> Vec<Effect> {
        self.pens.clear();
        self.palms.clear();
        self.mouse = false;
        self.overflow = false;
        let events = self.clear_touches();
        self.contacts.clear();
        events
    }
    pub fn configure(&mut self, s: Settings) -> Vec<Effect> {
        if self.settings == s {
            return vec![];
        }
        self.settings = s;
        if !s.reject_palms {
            self.palms.clear();
        }
        self.clear_touches()
    }
    fn pair(&self) -> Option<[(Source, [f64; 2]); 2]> {
        if self.contacts.len() != 2 || self.contacts.iter().any(|c| c.blocked) {
            return None;
        }
        let mut p = std::array::from_fn(|n| {
            (
                self.contacts[n].event.source,
                self.contacts[n].event.position,
            )
        });
        p.sort_by_key(|(s, _)| (s.device, s.contact));
        Some(p)
    }
    fn cancel_drawing(&mut self, out: &mut Vec<Effect>) {
        if let Some(mut e) = self.drawing.take() {
            e.phase = Phase::Cancel;
            out.push(Effect::Pointer(e));
        }
    }
    fn contact(&mut self, e: Event, view: u64, allowed: bool, out: &mut Vec<Effect>) -> bool {
        if e.origin == Origin::TouchCompatibility {
            return false;
        } // native batch is authoritative
        let found = self
            .contacts
            .iter()
            .position(|c| c.event.source == e.source);
        let palm = self.settings.reject_palms
            && (e.attributes.palm == Some(true) || self.palms.contains(&e.source));
        if palm {
            if matches!(e.phase, Phase::Up | Phase::Cancel) {
                self.palms.remove(&e.source);
            } else if self.palms.len() < 32 {
                self.palms.insert(e.source);
            } else {
                self.overflow = true;
            }
        }
        if !e.valid() || palm {
            if let Some(n) = found {
                self.contacts.remove(n);
                if self.drawing.is_some_and(|d| d.source == e.source) {
                    self.cancel_drawing(out);
                }
                return true;
            }
            return false;
        }
        let blocked = self.mouse || !self.pens.is_empty() || self.overflow;
        match e.phase {
            Phase::Down => {
                if let Some(n) = found {
                    self.contacts[n].blocked = true;
                    self.cancel_drawing(out);
                    return true;
                }
                if self.contacts.len() >= 16
                    || self
                        .contacts
                        .first()
                        .is_some_and(|c| c.event.source.device != e.source.device)
                {
                    return false;
                }
                let blocked = blocked
                    || !allowed
                    || (!self.settings.gestures && !self.settings.finger_drawing)
                    || self.contacts.iter().any(|c| c.blocked);
                if self.contacts.is_empty() && !blocked {
                    self.view = Some(view);
                }
                self.contacts.push(Contact { event: e, blocked });
                if self.contacts.len() == 1 && self.settings.finger_drawing && !blocked {
                    self.drawing = Some(e);
                    out.push(Effect::Pointer(e));
                } else {
                    self.cancel_drawing(out);
                }
                true
            }
            Phase::Move => {
                let Some(n) = found else {
                    return false;
                };
                self.contacts[n].event = e;
                if blocked || self.contacts[n].blocked {
                    return false;
                }
                if self.contacts.len() == 1 && self.drawing.is_some_and(|d| d.source == e.source) {
                    self.drawing = Some(e);
                    out.push(Effect::Pointer(e));
                }
                false
            }
            Phase::Up | Phase::Cancel => {
                let Some(n) = found else {
                    return false;
                };
                self.contacts.remove(n);
                if self.drawing.is_some_and(|d| d.source == e.source) {
                    self.drawing = None;
                    out.push(Effect::Pointer(e));
                }
                if self.contacts.is_empty() {
                    self.view = None;
                }
                true
            }
        }
    }
    pub fn process(
        &mut self,
        events: &[Event],
        view: u64,
        mut allowed: impl FnMut(Event) -> bool,
    ) -> Vec<Effect> {
        let mut out = Vec::new();
        let mut n = 0;
        while n < events.len() {
            let e = events[n];
            if e.source.kind != Kind::Touch {
                if e.source.kind == Kind::Pen {
                    match e.phase {
                        Phase::Down | Phase::Move => {
                            if self.pens.len() < 32 {
                                self.pens.insert(e.source);
                            } else if !self.pens.contains(&e.source) {
                                self.overflow = true;
                            }
                            out.extend(self.clear_touches());
                        }
                        Phase::Up | Phase::Cancel => {
                            self.pens.remove(&e.source);
                        }
                    }
                } else if e.source.kind == Kind::Mouse {
                    match e.phase {
                        Phase::Down => {
                            self.mouse = true;
                            out.extend(self.clear_touches());
                        }
                        Phase::Up | Phase::Cancel => self.mouse = false,
                        _ => {}
                    }
                }
                out.push(Effect::Pointer(e));
                n += 1;
                continue;
            }
            let before = self.pair();
            let mut changed = false;
            let frame = e.attributes.touch_frame;
            loop {
                let e = events[n];
                changed |= self.contact(e, view, allowed(e), &mut out);
                n += 1;
                if frame.is_none()
                    || n == events.len()
                    || events[n].source.kind != Kind::Touch
                    || events[n].attributes.touch_frame != frame
                {
                    break;
                }
            }
            if !changed
                && self.settings.gestures
                && let (Some(a), Some(b), Some(view)) = (before, self.pair(), self.view)
                && a[0].0 == b[0].0
                && a[1].0 == b[1].0
                && a != b
            {
                out.push(Effect::Motion(Motion {
                    view,
                    from: a.map(|p| p.1),
                    to: b.map(|p| p.1),
                }));
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer_input::Attributes;
    fn e(id: u64, phase: Phase, p: [f64; 2], frame: Option<u64>) -> Event {
        Event {
            source: Source {
                kind: Kind::Touch,
                device: 9,
                contact: id,
            },
            phase,
            position: p,
            pressure: None,
            attributes: Attributes {
                touch_frame: frame,
                ..Default::default()
            },
            origin: Origin::Unmarked,
        }
    }
    fn feed(c: &mut Controller, events: &[Event]) -> Vec<Effect> {
        c.process(events, 77, |_| true)
    }
    #[test]
    fn native_batch_manipulation_preserves_centroid_across_dpi_rotation_and_mirror() {
        let mut c = Controller::default();
        assert!(
            feed(
                &mut c,
                &[
                    e(1, Phase::Down, [0.0; 2], Some(1)),
                    e(2, Phase::Down, [10.0, 0.0], Some(1))
                ]
            )
            .is_empty()
        );
        let effects = feed(
            &mut c,
            &[
                e(1, Phase::Move, [5.0, 5.0], Some(2)),
                e(2, Phase::Move, [5.0, 25.0], Some(2)),
            ],
        );
        assert_eq!(effects.len(), 1);
        let Effect::Motion(m) = effects[0] else {
            panic!()
        };
        assert_eq!(m.view, 77);
        for ppp in [1.0, 1.5, 2.0] {
            for flipped in [false, true] {
                let mut view = View {
                    angle: 37.0,
                    flipped,
                    ..Default::default()
                };
                let doc = view.screen_to_document([5.0, 0.0], [64.0, 48.0], [100.0; 2], ppp);
                m.apply(&mut view, [64.0, 48.0], [100.0; 2], ppp).unwrap();
                assert_eq!(view.zoom, 2.0);
                assert!((view.angle - 127.0).abs() < 1e-9);
                assert_eq!(view.flipped, flipped);
                let point = view.document_to_screen(doc, [64.0, 48.0], [100.0; 2], ppp);
                assert!((point[0] - 5.0).abs() < 1e-9 && (point[1] - 15.0).abs() < 1e-9);
            }
        }
        assert!(
            feed(
                &mut c,
                &[
                    e(3, Phase::Down, [20.0; 2], None),
                    e(1, Phase::Move, [6.0; 2], None),
                    e(3, Phase::Up, [20.0; 2], None)
                ]
            )
            .is_empty()
        );
        assert!(matches!(
            feed(&mut c, &[e(1, Phase::Move, [7.0; 2], None)]).as_slice(),
            [Effect::Motion(_)]
        ));
        let mut coincident = View::default();
        Motion {
            view: 0,
            from: [[0.0; 2]; 2],
            to: [[5.0; 2]; 2],
        }
        .apply(&mut coincident, [10.0; 2], [0.0; 2], 2.0)
        .unwrap();
        assert_eq!(coincident.zoom, 1.0);
        assert_eq!(coincident.pan, [5.0; 2]);
    }
    #[test]
    fn pen_priority_palms_count_change_and_focus_never_restart_held_finger_ink() {
        let mut c = Controller::default();
        c.configure(Settings {
            finger_drawing: true,
            ..Default::default()
        });
        assert!(matches!(
            feed(&mut c, &[e(1, Phase::Down, [0.0; 2], None)]).as_slice(),
            [Effect::Pointer(Event {
                phase: Phase::Down,
                ..
            })]
        ));
        let mut pen = e(9, Phase::Down, [0.0; 2], None);
        pen.source.kind = Kind::Pen;
        assert!(matches!(
            feed(&mut c, &[pen]).as_slice(),
            [
                Effect::Pointer(Event {
                    phase: Phase::Cancel,
                    ..
                }),
                Effect::Pointer(Event {
                    phase: Phase::Down,
                    ..
                })
            ]
        ));
        assert!(feed(&mut c, &[e(2, Phase::Down, [10.0; 2], None)]).is_empty());
        pen.phase = Phase::Up;
        feed(&mut c, &[pen]);
        assert!(
            feed(
                &mut c,
                &[
                    e(1, Phase::Move, [1.0; 2], None),
                    e(2, Phase::Move, [11.0; 2], None),
                    e(1, Phase::Up, [1.0; 2], None),
                    e(2, Phase::Up, [11.0; 2], None)
                ]
            )
            .is_empty()
        );
        feed(&mut c, &[e(1, Phase::Down, [0.0; 2], None)]);
        let second = feed(&mut c, &[e(2, Phase::Down, [10.0; 2], None)]);
        assert!(matches!(
            second.as_slice(),
            [Effect::Pointer(Event {
                phase: Phase::Cancel,
                ..
            })]
        ));
        feed(&mut c, &[e(2, Phase::Up, [10.0; 2], None)]);
        assert!(feed(&mut c, &[e(1, Phase::Move, [2.0; 2], None)]).is_empty());
        c.reset();
        assert!(feed(&mut c, &[e(1, Phase::Move, [3.0; 2], None)]).is_empty());
        let mut palm = e(3, Phase::Down, [0.0; 2], None);
        palm.attributes.palm = Some(true);
        assert!(feed(&mut c, &[palm]).is_empty());
        feed(&mut c, &[e(1, Phase::Down, [0.0; 2], None)]);
        let mut bad = e(1, Phase::Move, [f64::NAN, 0.0], None);
        assert!(matches!(
            feed(&mut c, &[bad]).as_slice(),
            [Effect::Pointer(Event {
                phase: Phase::Cancel,
                ..
            })]
        ));
        bad.phase = Phase::Move;
        bad.position = [0.0; 2];
        assert!(feed(&mut c, &[bad]).is_empty());
    }
    #[test]
    fn unrelated_devices_promotions_settings_and_rejected_ui_contacts_are_isolated() {
        let mut c = Controller::default();
        let a = e(1, Phase::Down, [0.0; 2], None);
        feed(&mut c, &[a]);
        let mut foreign = e(2, Phase::Down, [10.0; 2], None);
        foreign.source.device = 10;
        assert!(feed(&mut c, &[foreign]).is_empty());
        let mut promotion = e(7, Phase::Down, [0.0; 2], None);
        promotion.origin = Origin::TouchCompatibility;
        assert!(feed(&mut c, &[promotion]).is_empty());
        c.reset();
        c.configure(Settings {
            finger_drawing: true,
            ..Default::default()
        });
        assert!(c.process(&[a], 77, |_| false).is_empty());
        assert!(
            feed(
                &mut c,
                &[
                    e(2, Phase::Down, [10.0; 2], None),
                    e(1, Phase::Up, [0.0; 2], None),
                    e(2, Phase::Move, [11.0; 2], None)
                ]
            )
            .is_empty()
        );
        c.reset();
        feed(&mut c, &[a]);
        assert!(matches!(
            c.configure(Settings::default()).as_slice(),
            [Effect::Pointer(Event {
                phase: Phase::Cancel,
                ..
            })]
        ));
        assert!(feed(&mut c, &[e(1, Phase::Move, [1.0; 2], None)]).is_empty());
    }
}
