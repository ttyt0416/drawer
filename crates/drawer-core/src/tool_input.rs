use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub hold_millis: u32,
    pub drag_pixels: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            hold_millis: 300,
            drag_pixels: 4.0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1 && self.hold_millis <= 5000 && (0.0..=64.0).contains(&self.drag_pixels)
    }
}
#[derive(Clone)]
struct Held {
    key: String,
    tool: BrushTip,
    time: f64,
    sequence: u64,
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
        });
        Some(tool)
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
        if time.is_finite()
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
