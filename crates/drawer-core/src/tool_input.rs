use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub hold_millis: u32,
    pub drag_pixels: f64,
    #[serde(default)]
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub smoothing: [f64; 7],
    #[serde(default)]
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub ends: [crate::stroke_ends::Settings; 7],
    #[serde(default = "crate::brush_slots::materials")]
    #[serde(deserialize_with = "crate::brush_slots::read_materials")]
    pub materials: [crate::brush_material::Settings; 7],
    #[serde(default)]
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub textures: [crate::texture::Pattern; 7],
    #[serde(default, deserialize_with = "crate::brush_slots::read")]
    pub profiles: [crate::tip_profile::Settings; 7],
    /// Ctrl, Shift, Alt. Modifiers always restore on release.
    #[serde(default)]
    pub modifiers: [Option<BrushTip>; 3],
    #[serde(default)]
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub mouse_pressure: [crate::mouse_pressure::Settings; 7],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            hold_millis: 300,
            drag_pixels: 4.0,
            smoothing: [0.0; 7],
            ends: [Default::default(); 7],
            materials: crate::brush_slots::materials(),
            textures: [Default::default(); 7],
            profiles: [Default::default(); 7],
            modifiers: [None; 3],
            mouse_pressure: [Default::default(); 7],
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
            && self.ends.iter().all(|s| s.valid())
            && self.materials.iter().all(|s| s.valid())
            && self.profiles.iter().all(|s| s.valid())
            && self.textures.iter().all(|s| s.valid())
            && self.mouse_pressure.iter().all(|s| s.valid())
            && self.hold_millis <= 5000
            && (0.0..=64.0).contains(&self.drag_pixels)
            && self.smoothing.iter().all(|s| (0.0..=0.95).contains(s))
    }
}
#[derive(Clone)]
struct Held {
    key: String,
    tool: BrushTip,
    time: f64,
    sequence: u64,
    restore: bool,
}
/// Releases are resolved independently; short choices commit in press order.
#[derive(Default)]
pub struct Controller {
    held: Vec<Held>,
    base: Option<(BrushTip, Option<u64>)>,
    sequence: u64,
    committed: u64,
}
impl Controller {
    pub fn press(
        &mut self,
        key: &str,
        tool: BrushTip,
        current: BrushTip,
        document: Option<u64>,
        time: f64,
        repeat: bool,
    ) -> Option<BrushTip> {
        if repeat || !time.is_finite() || self.held.iter().any(|h| h.key == key) {
            return None;
        }
        if self.held.is_empty() {
            self.base = Some((current, document));
            self.committed = 0;
            self.sequence = 0;
        }
        self.sequence += 1;
        self.held.push(Held {
            key: key.into(),
            tool,
            time,
            sequence: self.sequence,
            restore: false,
        });
        Some(tool)
    }
    pub fn press_modifier(
        &mut self,
        key: &str,
        tool: BrushTip,
        current: BrushTip,
        document: Option<u64>,
        time: f64,
    ) -> Option<BrushTip> {
        let result = self.press(key, tool, current, document, time, false)?;
        self.held.last_mut()?.restore = true;
        Some(result)
    }
    pub fn release(
        &mut self,
        key: &str,
        time: f64,
        settings: Settings,
    ) -> Option<(BrushTip, Option<u64>)> {
        let index = self.held.iter().position(|h| h.key == key)?;
        let h = self.held.remove(index);
        let (base, document) = self.base.as_mut()?;
        if !h.restore
            && time.is_finite()
            && time >= h.time
            && (time - h.time) * 1000.0 + 1e-7 < settings.hold_millis as f64
            && h.sequence > self.committed
        {
            *base = h.tool;
            self.committed = h.sequence;
        }
        let tool = self.held.last().map_or(*base, |h| h.tool);
        let doc = *document;
        if self.held.is_empty() {
            self.base = None;
        }
        Some((tool, doc))
    }
    pub fn cancel(&mut self) -> Option<(BrushTip, Option<u64>)> {
        self.held.clear();
        self.base.take()
    }
    pub fn persisted_tool(&self) -> Option<BrushTip> {
        self.base.map(|b| b.0)
    }
    pub fn document(&self) -> Option<Option<u64>> {
        self.base.map(|b| b.1)
    }
    pub fn temporary(&self, time: f64, settings: Settings) -> bool {
        self.held.last().is_some_and(|h| {
            time.is_finite() && (time - h.time) * 1000.0 + 1e-7 >= settings.hold_millis as f64
        })
    }
}
#[derive(Clone, Copy)]
pub struct Drag {
    origin: [f64; 2],
    last: [f64; 2],
    started: bool,
}
impl Drag {
    pub fn new(point: [f64; 2]) -> Self {
        Self {
            origin: point,
            last: point,
            started: false,
        }
    }
    /// Physical-pixel threshold; logical delta preserves view behavior across DPI.
    pub fn update(&mut self, point: [f64; 2], pixels: f64, ppp: f64) -> Option<[f64; 2]> {
        if !point
            .iter()
            .chain(self.origin.iter())
            .all(|n| n.is_finite())
            || !ppp.is_finite()
            || ppp <= 0.0
        {
            return None;
        }
        if !self.started
            && ((point[0] - self.origin[0]).hypot(point[1] - self.origin[1]) * ppp + 1e-9) < pixels
        {
            return None;
        }
        self.started = true;
        let delta = [point[0] - self.last[0], point[1] - self.last[1]];
        self.last = point;
        Some(delta)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifiers_always_restore_and_overlap_in_press_order() {
        for duration in [0.01, 0.3, 10.0] {
            let mut c = Controller::default();
            c.press_modifier("@Ctrl", BrushTip::Eraser, BrushTip::Pen, Some(1), 0.0);
            c.press_modifier("@Shift", BrushTip::Wash, BrushTip::Eraser, Some(1), 0.001);
            assert_eq!(c.persisted_tool(), Some(BrushTip::Pen));
            assert_eq!(
                c.release("@Ctrl", duration, Settings::default()),
                Some((BrushTip::Wash, Some(1)))
            );
            assert_eq!(
                c.release("@Shift", duration, Settings::default()),
                Some((BrushTip::Pen, Some(1)))
            );
        }
        let legacy: Settings =
            serde_json::from_str(r#"{"version":1,"hold_millis":300,"drag_pixels":4}"#).unwrap();
        assert_eq!(legacy.modifiers, [None; 3]);
        assert!(serde_json::from_str::<Settings>(r#"{"version":1,"hold_millis":300,"drag_pixels":4,"modifiers":["Missing",null,null]}"#).is_err());
    }
    #[test]
    fn short_long_boundary_repeat_and_bad_time_release() {
        let mut c = Controller::default();
        let s = Settings::default();
        for (end, expected) in [
            (0.299, BrushTip::Wash),
            (0.3, BrushTip::Pen),
            (0.301, BrushTip::Pen),
            (f64::NAN, BrushTip::Pen),
        ] {
            assert_eq!(
                c.press("W", BrushTip::Wash, BrushTip::Pen, Some(7), 0.0, false),
                Some(BrushTip::Wash)
            );
            assert_eq!(
                c.press("W", BrushTip::Wash, BrushTip::Wash, Some(7), 0.29, true),
                None
            );
            assert_eq!(c.persisted_tool(), Some(BrushTip::Pen));
            assert!(c.temporary(0.3, s));
            assert_eq!(c.release("W", end, s), Some((expected, Some(7))));
            assert_eq!(c.persisted_tool(), None);
        }
    }
    #[test]
    fn overlapping_key_release_order_focus_cancel_and_document_context() {
        for reverse in [false, true] {
            let mut c = Controller::default();
            c.press("N", BrushTip::Pencil, BrushTip::Pen, Some(1), 1.0, false);
            c.press(
                "E",
                BrushTip::Eraser,
                BrushTip::Pencil,
                Some(1),
                1.01,
                false,
            );
            let keys = if reverse { ["E", "N"] } else { ["N", "E"] };
            c.release(keys[0], 1.1, Settings::default());
            assert_eq!(
                c.release(keys[1], 1.11, Settings::default()).unwrap().0,
                BrushTip::Eraser
            );
            c.press("W", BrushTip::Wash, BrushTip::Eraser, Some(1), 2.0, false);
            assert_eq!(c.document(), Some(Some(1)));
            assert_eq!(c.cancel(), Some((BrushTip::Eraser, Some(1))));
            assert_eq!(c.release("W", 3.0, Settings::default()), None);
        }
    }
    #[test]
    fn drag_threshold_keeps_full_first_delta_at_mixed_dpi_and_rejects_bad_input() {
        for ppp in [1.0, 1.5, 2.0] {
            let mut d = Drag::new([10.0, 20.0]);
            assert_eq!(d.update([10.0 + 3.9 / ppp, 20.0], 4.0, ppp), None);
            assert!(
                (d.update([10.0 + 4.0 / ppp, 20.0], 4.0, ppp).unwrap()[0] - 4.0 / ppp).abs() < 1e-9
            );
            assert!(
                (d.update([10.0 + 6.0 / ppp, 20.0], 4.0, ppp).unwrap()[0] - 2.0 / ppp).abs() < 1e-9
            );
            assert_eq!(d.update([f64::NAN, 0.0], 4.0, ppp), None);
        }
    }
}
