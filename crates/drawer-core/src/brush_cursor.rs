use crate::{scene::Point, view::View};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    Outline,
    Crosshair,
    Icon,
    System,
}
impl Style {
    pub const ALL: [Self; 4] = [Self::Outline, Self::Crosshair, Self::Icon, Self::System];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Raw,
    Corrected,
    Both,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Raw, Self::Corrected, Self::Both];
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub style: Style,
    pub mode: Mode,
    pub minimum_pixels: f64,
    pub prediction: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            style: Style::Outline,
            mode: Mode::Raw,
            minimum_pixels: 0.0,
            prediction: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub tools: [Appearance; 7],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            tools: [Appearance::default(); 7],
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
            && self.tools.iter().all(|s| {
                (0.0..=32.0).contains(&s.minimum_pixels)
                    && (s.style != Style::System || (s.mode == Mode::Raw && !s.prediction))
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Raw,
    Corrected,
    Prediction,
}
/// Filtering receives real samples only. Display mode never enters this computation.
#[derive(Default, Clone)]
pub struct Trace {
    pub raw: Option<Point>,
    pub corrected: Option<Point>,
    pub prediction: Option<Point>,
}
impl Trace {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn receive(&mut self, point: Point, strength: f64) -> Option<(Point, Point)> {
        if !point.iter().all(|v| v.is_finite() && v.abs() <= 1e12)
            || !(0.0..=0.95).contains(&strength)
        {
            return None;
        }
        if self.raw == Some(point) {
            return None;
        }
        let from = self.corrected.unwrap_or(point);
        let next = [
            point[0] + (from[0] - point[0]) * strength,
            point[1] + (from[1] - point[1]) * strength,
        ];
        let delta = [next[0] - from[0], next[1] - from[1]];
        let scale = (64.0 / delta[0].hypot(delta[1]).max(64.0)).min(1.0);
        self.prediction = self
            .raw
            .map(|_| [next[0] + delta[0] * scale, next[1] + delta[1] * scale]);
        self.raw = Some(point);
        self.corrected = Some(next);
        Some((from, next))
    }
    pub fn marks(&self, appearance: Appearance) -> Vec<(Mark, Point)> {
        let mut marks = Vec::new();
        if appearance.mode != Mode::Corrected
            && let Some(p) = self.raw
        {
            marks.push((Mark::Raw, p));
        }
        if appearance.mode != Mode::Raw
            && let Some(p) = self.corrected
        {
            marks.push((Mark::Corrected, p));
        }
        if appearance.prediction
            && let Some(p) = self.prediction
        {
            marks.push((Mark::Prediction, p));
        }
        marks
    }
    pub fn paint(
        &mut self,
        pad: &mut crate::workspaces::Scratchpad,
        point: Point,
        strength: f64,
        size: f64,
        tip: crate::assets::BrushTip,
        color: [u8; 4],
    ) {
        self.paint_tip(pad, point, strength, size, (tip, Default::default()), color);
    }
    pub fn paint_tip(
        &mut self,
        pad: &mut crate::workspaces::Scratchpad,
        point: Point,
        strength: f64,
        size: f64,
        tip: (crate::assets::BrushTip, crate::tip_profile::Settings),
        color: [u8; 4],
    ) {
        if let Some((from, to)) = self.receive(point, strength) {
            pad.stroke_tip(from, to, size, tip, color);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub size: f64,
    pub aspect: f64,
    pub angle: f64,
}
impl Default for Shape {
    fn default() -> Self {
        Self {
            size: 12.0,
            aspect: 1.0,
            angle: 0.0,
        }
    }
}
impl Shape {
    pub fn valid(self) -> bool {
        self.size.is_finite()
            && self.size > 0.0
            && self.size <= 4096.0
            && (0.05..=1.0).contains(&self.aspect)
            && self.angle.is_finite()
    }
    pub fn outline(
        self,
        view: View,
        point: Point,
        size: Point,
        center: Point,
        ppp: f64,
    ) -> Vec<Point> {
        if !self.valid() || !ppp.is_finite() || ppp <= 0.0 || point.iter().any(|n| !n.is_finite()) {
            return Vec::new();
        }
        let (sin, cos) = self.angle.to_radians().sin_cos();
        (0..64)
            .map(|n| {
                let angle = n as f64 * std::f64::consts::TAU / 64.0;
                let x = angle.cos() * self.size / 2.0;
                let y = angle.sin() * self.size * self.aspect / 2.0;
                view.document_to_screen(
                    [point[0] + x * cos - y * sin, point[1] + x * sin + y * cos],
                    size,
                    center,
                    ppp,
                )
            })
            .collect()
    }
    pub fn displayed_outline(
        self,
        view: View,
        point: Point,
        size: Point,
        center: Point,
        ppp: f64,
        minimum: f64,
    ) -> Vec<Point> {
        let mut outline = self.outline(view, point, size, center, ppp);
        if minimum.is_finite() && minimum > 0.0 && self.size * view.zoom < minimum {
            let center = view.document_to_screen(point, size, center, ppp);
            let scale = minimum / (self.size * view.zoom);
            for p in &mut outline {
                p[0] = center[0] + (p[0] - center[0]) * scale;
                p[1] = center[1] + (p[1] - center[1]) * scale;
            }
        }
        outline
    }
    /// Two icon axes retain the mirror determinant and tip/view rotation.
    pub fn axes(self, view: View) -> [Point; 2] {
        let view = View {
            pan: [0.0; 2],
            ..view
        };
        let (sin, cos) = self.angle.to_radians().sin_cos();
        [[cos, sin], [-sin, cos]].map(|p| {
            let out = view.document_to_screen(p, [0.0; 2], [0.0; 2], 1.0);
            let len = out[0].hypot(out[1]);
            [out[0] / len, out[1] / len]
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_corrected_both_and_prediction_have_identical_confirmed_pixels_and_clear() {
        let samples = [
            [8.0, 8.0],
            [70.0, 8.0],
            [70.0, 80.0],
            [15.0, 80.0],
            [15.0, 20.0],
        ];
        let mut results = Vec::new();
        for mode in Mode::ALL {
            let mut trace = Trace::default();
            let mut pad = crate::workspaces::Scratchpad::default();
            let a = Appearance {
                mode,
                prediction: true,
                ..Default::default()
            };
            for point in samples {
                trace.paint(
                    &mut pad,
                    point,
                    0.6,
                    6.0,
                    crate::assets::BrushTip::Pen,
                    [30, 100, 180, 255],
                );
                let marks = trace.marks(a);
                assert!(marks.iter().any(|(k, _)| *k != Mark::Prediction));
            }
            assert_ne!(trace.raw, trace.corrected);
            results.push(pad.pixels);
            trace.clear();
            assert!(trace.marks(a).is_empty());
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[1], results[2]);
    }
    #[test]
    fn minimum_outline_center_rotated_tilt_axes_mirror_and_dpi() {
        let shape = Shape {
            size: 1.0,
            aspect: 0.3,
            angle: 29.0,
        };
        for ppp in [1.0, 1.5, 2.0] {
            for flipped in [false, true] {
                let view = View {
                    zoom: 0.01,
                    angle: 37.0,
                    flipped,
                    ..Default::default()
                };
                let point = [50.0, 60.0];
                let center = view.document_to_screen(point, [100.0; 2], [400.0; 2], ppp);
                let o = shape.displayed_outline(view, point, [100.0; 2], [400.0; 2], ppp, 4.0);
                assert!(((o[0][0] - o[32][0]).hypot(o[0][1] - o[32][1]) * ppp - 4.0).abs() < 1e-8);
                for n in 0..32 {
                    assert!(((o[n][0] + o[n + 32][0]) / 2.0 - center[0]).abs() < 1e-8);
                }
                let axes = shape.axes(view);
                let det = axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0];
                assert!((det - if flipped { -1.0 } else { 1.0 }).abs() < 1e-8);
                let direction = [o[0][0] - center[0], o[0][1] - center[1]];
                assert!((direction[0] * axes[0][1] - direction[1] * axes[0][0]).abs() < 1e-8);
            }
        }
    }
    #[test]
    fn minimum_large_rotated_elliptic_footprints_match_first_input_across_mirror_dpi_zoom() {
        let size = [800.0, 600.0];
        let center = [550.0, 400.0];
        let screen = [387.0, 322.0];
        for diameter in [1.0, 4096.0] {
            for ppp in [1.0, 1.5, 2.0, 4.0] {
                for zoom in [0.01, 1.0, 64.0] {
                    for flipped in [false, true] {
                        let view = View {
                            zoom,
                            angle: 37.0,
                            flipped,
                            pan: [10.0, -20.0],
                        };
                        let point = view.screen_to_document(screen, size, center, ppp);
                        let shape = Shape {
                            size: diameter,
                            aspect: 0.3,
                            angle: 29.0,
                        };
                        let outline = shape.outline(view, point, size, center, ppp);
                        for n in 0..32 {
                            let midpoint = [
                                (outline[n][0] + outline[n + 32][0]) / 2.0,
                                (outline[n][1] + outline[n + 32][1]) / 2.0,
                            ];
                            assert!(
                                (midpoint[0] - screen[0]).abs() < 1e-6
                                    && (midpoint[1] - screen[1]).abs() < 1e-6
                            );
                        }
                        let a = view.screen_to_document(outline[0], size, center, ppp);
                        let b = view.screen_to_document(outline[16], size, center, ppp);
                        assert!(
                            ((a[0] - point[0]).hypot(a[1] - point[1]) - diameter / 2.0).abs()
                                < 1e-6
                        );
                        assert!(
                            ((b[0] - point[0]).hypot(b[1] - point[1]) - diameter * 0.3 / 2.0).abs()
                                < 1e-6
                        );
                    }
                }
            }
        }
    }
}
