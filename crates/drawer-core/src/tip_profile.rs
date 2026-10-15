//! Owned tip images and minimum output response, independent of raw pressure calibration.
use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Image {
    #[default]
    Native,
    Round,
    Square,
}
impl Image {
    pub const ALL: [Self; 3] = [Self::Native, Self::Round, Self::Square];
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Tool's owned tip",
            Self::Round => "Owned round tip",
            Self::Square => "Owned square tip",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub minimum_size: f64,
    pub minimum_opacity: f64,
    pub image: Image,
}
impl Settings {
    pub fn valid(self) -> bool {
        (0.0..=1.0).contains(&self.minimum_size) && (0.0..=1.0).contains(&self.minimum_opacity)
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn size(self, p: f64) -> f64 {
        self.minimum_size + (1.0 - self.minimum_size) * p
    }
    pub fn opacity(self, p: f64) -> f64 {
        self.minimum_opacity + (1.0 - self.minimum_opacity) * p
    }
    pub fn square(self, tip: BrushTip) -> bool {
        match self.image {
            Image::Square => true,
            Image::Round => false,
            Image::Native => tip == BrushTip::PixelPen,
        }
    }
    /// An owned local preview image; selection does not depend on any external file.
    pub fn mask(self, tip: BrushTip, size: usize) -> Vec<u8> {
        assert!((2..=256).contains(&size));
        if self.image == Image::Native {
            return tip.mask(size);
        }
        (0..size * size)
            .map(|n| {
                let q = [
                    ((n % size) as f64 + 0.5) * 2.0 / size as f64 - 1.0,
                    ((n / size) as f64 + 0.5) * 2.0 / size as f64 - 1.0,
                ];
                if self.square(tip) || q[0].hypot(q[1]) < 1.0 {
                    255
                } else {
                    0
                }
            })
            .collect()
    }
}
/// Convert the 64-point ellipse outline into its matching rectangle in screen space.
pub fn rectify_outline(points: &mut [[f64; 2]], center: [f64; 2]) {
    let count = points.len();
    for (n, p) in points.iter_mut().enumerate() {
        let (sin, cos) = (n as f64 * std::f64::consts::TAU / count as f64).sin_cos();
        let scale = 1.0 / sin.abs().max(cos.abs());
        for a in 0..2 {
            p[a] = center[a] + (p[a] - center[a]) * scale;
        }
    }
}

fn probe(
    profile: Settings,
    pressure: f64,
    hardness: f64,
) -> Result<crate::document::Document, String> {
    use crate::{
        brush_cursor::Shape,
        document::{Background, CanvasSpec, Document},
        painting::{Stroke, Style},
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 24,
            height: 24,
            dpi: 300.0,
            background: Background::Transparent,
        },
        4096,
    )
    .map_err(|e| e.to_string())?;
    let style = Style {
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile,
        texture: Default::default(),
        material: Default::default(),
        shape: Shape {
            size: 10.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [40, 100, 180, 255],
        smoothing: 0.0,
        opacity: 1.0,
        hardness,
        spacing: 0.15,
        pressure: crate::pressure::Mapping {
            size: true,
            opacity: true,
            ..Default::default()
        },
    };
    let mut stroke = Stroke::begin(&d, 0, style)?;
    stroke.direct_dab(&d, [12.5; 2], [12.5; 2], pressure)?;
    stroke.commit(&mut d)?;
    Ok(d)
}
pub fn verify() -> Result<(), String> {
    let full = Settings {
        minimum_size: 1.0,
        minimum_opacity: 1.0,
        image: Image::Round,
    };
    let zero = probe(full, 0.0, 1.0)?;
    let maximum = probe(full, 1.0, 1.0)?;
    if zero.layers[0].pixels != maximum.layers[0].pixels
        || zero.composite_pixel(12, 12) != [40, 100, 180, 255]
    {
        return Err("Minimum equals maximum did not ignore zero pressure".into());
    }
    let floor = Settings {
        minimum_size: 0.5,
        minimum_opacity: 0.4,
        image: Image::Round,
    };
    let mut minimum = probe(floor, 0.0, 1.0)?;
    if minimum.composite_pixel(12, 12)[3] != 102
        || minimum.layers[0].pixels.iter().any(|p| p[3] > 102)
        || minimum.history_counts() != (1, 0)
        || !minimum.undo()
        || minimum.layers[0].pixels.iter().any(|p| p[3] != 0)
        || !minimum.redo()
    {
        return Err("Zero-pressure output floors or undo failed".into());
    }
    if probe(Settings::default(), 0.0, 1.0)?.history_counts() != (0, 0) {
        return Err("Zero default floors unexpectedly painted at zero pressure".into());
    }
    let round = probe(full, 1.0, 1.0)?;
    let square = probe(
        Settings {
            image: Image::Square,
            ..full
        },
        1.0,
        1.0,
    )?;
    let soft = probe(full, 1.0, 0.0)?;
    if round.composite_pixel(16, 16)[3] != 0
        || square.composite_pixel(16, 16)[3] != 255
        || soft.composite_pixel(16, 12)[3] >= round.composite_pixel(16, 12)[3]
    {
        return Err("Round/square tip image or hardness did not change its edge".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_pressure_floors_equal_maximum_owned_images_softness_and_one_undo() {
        verify().unwrap();
    }
    #[test]
    fn invalid_floors_legacy_defaults_and_pressure_axis_disable_are_explicit() {
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(
                !Settings {
                    minimum_size: value,
                    ..Default::default()
                }
                .valid()
            );
            assert!(
                !Settings {
                    minimum_opacity: value,
                    ..Default::default()
                }
                .valid()
            );
        }
        let a = Settings {
            minimum_size: 0.3,
            minimum_opacity: 0.6,
            ..Default::default()
        };
        assert_eq!(a.size(0.0), 0.3);
        assert_eq!(a.size(1.0), 1.0);
        assert_eq!(a.opacity(0.0), 0.6);
        assert_eq!(a.opacity(1.0), 1.0);
        let mut value = serde_json::to_value(crate::tool_input::Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("profiles");
        let legacy: crate::tool_input::Settings = serde_json::from_value(value).unwrap();
        assert!(legacy.profiles.iter().all(Settings::is_default));
        let zero = probe(Settings::default(), 0.0, 1.0).unwrap();
        let mut d = zero;
        let style = crate::painting::Style {
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Settings::default(),
            texture: Default::default(),
            material: Default::default(),
            shape: crate::brush_cursor::Shape {
                size: 4.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip: BrushTip::Pen,
            color: [255; 4],
            smoothing: 0.0,
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.15,
            pressure: crate::pressure::Mapping {
                size: false,
                opacity: false,
                ..Default::default()
            },
        };
        let mut stroke = crate::painting::Stroke::begin(&d, 0, style).unwrap();
        stroke.direct_dab(&d, [12.5; 2], [12.5; 2], 0.0).unwrap();
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(12, 12), [255; 4]);
        let mut empty = probe(Settings::default(), 0.0, 1.0).unwrap();
        let style = crate::painting::Style {
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Settings {
                minimum_size: 1.0,
                minimum_opacity: 1.0,
                ..Default::default()
            },
            ..style
        };
        let mut stroke = crate::painting::Stroke::begin(&empty, 0, style).unwrap();
        stroke.set_geometry_scale(0.0).unwrap();
        stroke
            .direct_dab(&empty, [12.5; 2], [12.5; 2], 0.0)
            .unwrap();
        assert!(!stroke.commit(&mut empty).unwrap());
    }
    #[test]
    fn square_corner_outside_canvas_survives_segment_clipping() {
        let mut d = probe(Settings::default(), 0.0, 1.0).unwrap();
        let style = crate::painting::Style {
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Settings {
                image: Image::Square,
                ..Default::default()
            },
            texture: Default::default(),
            material: Default::default(),
            shape: crate::brush_cursor::Shape {
                size: 10.0,
                aspect: 1.0,
                angle: 45.0,
            },
            tip: BrushTip::Pen,
            color: [255; 4],
            smoothing: 0.0,
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.1,
            pressure: Default::default(),
        };
        let mut stroke = crate::painting::Stroke::begin(&d, 0, style).unwrap();
        stroke.sample(&d, [-6.5, 6.5]).unwrap();
        stroke.sample(&d, [-6.5, 10.5]).unwrap();
        stroke.commit(&mut d).unwrap();
        assert!(d.composite_pixel(0, 8)[3] > 0);
    }
    #[test]
    fn profile_preset_frozen_native_style_and_legacy_fields_round_trip() {
        let root = crate::test_support::TempDir::new();
        let mut snapshot = crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let p = Settings {
            minimum_size: 0.4,
            minimum_opacity: 0.7,
            image: Image::Square,
        };
        snapshot.input.profiles[0] = p;
        let preset =
            crate::brush_presets::Preset::capture(&snapshot, BrushTip::Pen, "Square floor")
                .unwrap();
        let restored = crate::brush_presets::Preset::decode(&preset.encode().unwrap()).unwrap();
        assert_eq!(
            restored
                .apply(&crate::workspaces::Snapshot {
                    properties: Some(Default::default()),
                    ..Default::default()
                })
                .unwrap()
                .input
                .profiles[0],
            p
        );
        let mut old = serde_json::to_value(&preset).unwrap();
        old.as_object_mut().unwrap().remove("profile");
        assert!(
            serde_json::from_value::<crate::brush_presets::Preset>(old)
                .unwrap()
                .profile
                .is_default()
        );
        let mut d = probe(Settings::default(), 0.0, 1.0).unwrap();
        let style = crate::painting::Style {
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: p,
            texture: Default::default(),
            material: Default::default(),
            shape: crate::brush_cursor::Shape {
                size: 8.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip: BrushTip::Pen,
            color: [255; 4],
            smoothing: 0.0,
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.15,
            pressure: crate::pressure::Mapping {
                size: true,
                opacity: true,
                ..Default::default()
            },
        };
        let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, style, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        for point in [[4.5, 12.5], [20.5, 12.5]] {
            replay
                .push_input(
                    crate::mouse_pressure::Sample {
                        point,
                        micros: None,
                    },
                    0.0,
                    0.0,
                )
                .unwrap();
        }
        let metadata = replay.metadata();
        replay
            .render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, metadata)
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("profile.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let reopened = crate::files::open(&path, 1000000).unwrap();
        assert!(reopened.snapshot() == before);
        assert_eq!(reopened.scene.mouse_strokes[0].style.profile, p);
        let mut old = serde_json::to_value(style).unwrap();
        old.as_object_mut().unwrap().remove("profile");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .profile
                .is_default()
        );
    }
    #[test]
    fn owned_mask_and_transformed_rectangle_cursor_are_local_and_deterministic() {
        let round = Settings {
            image: Image::Round,
            ..Default::default()
        };
        let square = Settings {
            image: Image::Square,
            ..Default::default()
        };
        assert_eq!(round.mask(BrushTip::Pen, 16)[0], 0);
        assert!(square.mask(BrushTip::Pen, 16).iter().all(|p| *p == 255));
        use crate::{brush_cursor::Shape, view::View};
        for (angle, mirror) in [(0.0, false), (37.0, true), (90.0, false)] {
            let view = View {
                angle,
                flipped: mirror,
                ..Default::default()
            };
            let shape = Shape {
                size: 10.0,
                aspect: 0.5,
                angle: 23.0,
            };
            let center = view.document_to_screen([12.0; 2], [24.0; 2], [50.0; 2], 1.5);
            let mut points =
                shape.displayed_outline(view, [12.0; 2], [24.0; 2], [50.0; 2], 1.5, 0.0);
            let before = points.clone();
            rectify_outline(&mut points, center);
            assert_eq!(points.len(), 64);
            assert_eq!(points[0], before[0]);
            assert!(
                (points[8][0] - before[8][0]).abs() + (points[8][1] - before[8][1]).abs() > 0.1
            );
        }
    }
    #[test]
    fn equal_output_floors_are_pressure_independent_for_all_seven_tips_and_test_area() {
        use crate::{
            brush_cursor::Shape,
            document::{Background, CanvasSpec, Document},
            painting::{Stroke, Style},
        };
        for tip in BrushTip::ALL {
            let mut d = Document::create(
                CanvasSpec {
                    width: 24,
                    height: 24,
                    dpi: 300.0,
                    background: if tip == BrushTip::Eraser {
                        Background::Color([90, 140, 210])
                    } else {
                        Background::Transparent
                    },
                },
                4096,
            )
            .unwrap();
            if tip == BrushTip::Smudge {
                for y in 0..24 {
                    for x in 0..9 {
                        d.layers[0].pixels[y * 24 + x] = [220, 40, 10, 150];
                    }
                }
            }
            let style = Style {
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Settings {
                    minimum_size: 1.0,
                    minimum_opacity: 1.0,
                    image: Image::Square,
                },
                texture: Default::default(),
                material: crate::basic_brush::material(tip),
                shape: Shape {
                    size: 6.0,
                    aspect: 1.0,
                    angle: 0.0,
                },
                tip,
                color: [20, 80, 160, 180],
                smoothing: 0.0,
                opacity: 0.7,
                hardness: 0.7,
                spacing: 0.15,
                pressure: crate::pressure::Mapping {
                    size: true,
                    opacity: true,
                    ..Default::default()
                },
            };
            let mut result = None;
            for p in [0.0, 1.0] {
                let mut stroke = Stroke::begin(&d, 0, style).unwrap();
                stroke.sample_pressure(&d, [6.5, 12.5], p).unwrap();
                stroke.sample_pressure(&d, [20.5, 12.5], p).unwrap();
                assert!(stroke.commit(&mut d).unwrap());
                let pixels = d.layers[0].pixels.clone();
                if let Some(before) = &result {
                    assert_eq!(&pixels, before, "{tip:?}");
                } else {
                    result = Some(pixels);
                }
                assert!(d.undo());
            }
        }
        let mut round = crate::workspaces::Scratchpad::default();
        let mut square = round.clone();
        round.stroke_tip(
            [12.5; 2],
            [12.5; 2],
            10.0,
            (
                BrushTip::Pen,
                Settings {
                    image: Image::Round,
                    ..Default::default()
                },
            ),
            [255; 4],
        );
        square.stroke_tip(
            [12.5; 2],
            [12.5; 2],
            10.0,
            (
                BrushTip::Pen,
                Settings {
                    image: Image::Square,
                    ..Default::default()
                },
            ),
            [255; 4],
        );
        let i = 16 * crate::workspaces::PAD_SIZE[0] + 16;
        assert_eq!(round.pixels[i][3], 0);
        assert_eq!(square.pixels[i][3], 255);
    }
}
