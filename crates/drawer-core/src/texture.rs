//! Owned procedural texture recipes: deterministic document-space samples, no external assets.
use crate::scene::Point;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub source: u32,
    pub seed: u32,
    pub scale: f64,
    pub strength: f64,
    pub offset: Point,
    pub angle: f64,
}
impl Default for Pattern {
    fn default() -> Self {
        Self {
            source: 0,
            seed: 0,
            scale: 1.0,
            strength: 0.5,
            offset: [0.0; 2],
            angle: 0.0,
        }
    }
}
impl Pattern {
    pub fn valid(self) -> bool {
        (0.01..=1024.0).contains(&self.scale)
            && (0.0..=1.0).contains(&self.strength)
            && self.offset.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
            && self.angle.is_finite()
            && self.angle.abs() <= 360.0
    }
    pub fn missing(self) -> bool {
        self.source > 3
    }
    pub fn effective(self) -> bool {
        self.source > 0 && self.source <= 3 && self.strength > 0.0
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn is_none(&self) -> bool {
        self.source == 0
    }
    pub fn label(self) -> String {
        match self.source {
            0 => "No texture".into(),
            1 => "Owned fine grain".into(),
            2 => "Owned linen".into(),
            3 => "Owned flecks".into(),
            n => format!("Missing texture #{n} — no texture fallback"),
        }
    }
    pub fn factor(self, point: Point) -> f64 {
        if !self.effective() || !self.valid() {
            return 1.0;
        }
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let q = [
            (point[0] - self.offset[0]) / self.scale,
            (point[1] - self.offset[1]) / self.scale,
        ];
        let q = [q[0] * cos + q[1] * sin, -q[0] * sin + q[1] * cos];
        let x = q[0].rem_euclid(64.0).floor() as u32;
        let y = q[1].rem_euclid(64.0).floor() as u32;
        let mut hash = x.wrapping_mul(0x9e3779b9) ^ y.wrapping_mul(0x85ebca6b) ^ self.seed;
        hash = (hash ^ (hash >> 16)).wrapping_mul(0x7feb352d);
        hash = (hash ^ (hash >> 15)).wrapping_mul(0x846ca68b);
        hash ^= hash >> 16;
        let sample = match self.source {
            1 => 0.3 + 0.7 * (hash % 256) as f64 / 255.0,
            2 => {
                0.6 + 0.4
                    * ((q[0] * std::f64::consts::TAU / 8.0).cos()
                        * (q[1] * std::f64::consts::TAU / 8.0).cos()
                        + 1.0)
                    * 0.5
            }
            3 if hash.is_multiple_of(11) => 0.0,
            _ => 1.0,
        };
        1.0 - self.strength + self.strength * sample
    }
    pub fn shade(self, mut color: [u8; 4], point: Point) -> [u8; 4] {
        if color[3] > 0 {
            let f = self.factor(point);
            for c in &mut color[..3] {
                *c = (*c as f64 * f).round() as u8;
            }
        }
        color
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recipes_have_stable_scale_strength_offsets_and_fallback() {
        let p = Pattern {
            source: 1,
            strength: 1.0,
            ..Default::default()
        };
        assert_eq!(p.factor([2.5, 3.5]), p.factor([66.5, 3.5]));
        let scaled = Pattern { scale: 2.0, ..p };
        assert_eq!(scaled.factor([5.0, 7.0]), p.factor([2.5, 3.5]));
        let shifted = Pattern {
            offset: [10.0, -4.0],
            ..p
        };
        assert_eq!(shifted.factor([12.5, -0.5]), p.factor([2.5, 3.5]));
        assert_eq!(Pattern { strength: 0.0, ..p }.factor([1.5, 2.5]), 1.0);
        assert_eq!(Pattern { source: 44, ..p }.factor([1.5, 2.5]), 1.0);
        assert!(
            !Pattern {
                scale: f64::NAN,
                ..p
            }
            .valid()
        );
        assert!(
            !Pattern {
                offset: [f64::INFINITY, 0.0],
                ..p
            }
            .valid()
        );
        assert_eq!(p.shade([4, 5, 6, 0], [0.0; 2]), [4, 5, 6, 0]);
        assert_eq!(p.shade([255, 255, 255, 77], [0.5; 2])[3], 77);
    }
    #[test]
    fn brush_pattern_is_applied_once_to_opacity_and_water_at_document_coordinates() {
        use crate::{
            assets::BrushTip,
            brush_cursor::Shape,
            document::{Background, CanvasSpec, Document},
            painting::{Stroke, Style},
        };
        for water in [false, true] {
            let mut d = Document::create(
                CanvasSpec {
                    width: 8,
                    height: 8,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                1024,
            )
            .unwrap();
            let p = Pattern {
                source: 1,
                strength: 1.0,
                ..Default::default()
            };
            let style = Style {
                profile: Default::default(),
                texture: p,
                material: crate::brush_material::Settings {
                    mode: if water {
                        crate::brush_material::Mode::Water
                    } else {
                        crate::brush_material::Mode::Paint
                    },
                    ..Default::default()
                },
                opacity: 0.5,
                hardness: 1.0,
                spacing: 0.15,
                shape: Shape {
                    size: 6.0,
                    aspect: 1.0,
                    angle: 0.0,
                },
                tip: BrushTip::Pen,
                color: [255; 4],
                smoothing: 0.0,
                pressure: Default::default(),
            };
            let mut stroke = Stroke::begin(&d, 0, style).unwrap();
            for _ in 0..8 {
                stroke.direct_dab(&d, [3.5; 2], [3.5; 2], 1.0).unwrap();
            }
            stroke.commit(&mut d).unwrap();
            let expected = (255.0 * 0.5 * p.factor([3.5; 2])).round() as u8;
            assert_eq!(d.composite_pixel(3, 3)[3], expected);
            assert!(d.undo());
            assert!(d.layers[0].pixels.iter().all(|p| p[3] == 0));
            assert!(d.redo());
        }
    }
}
