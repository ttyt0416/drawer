//! Geometry taper is independent of device pressure and position stabilization.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub start: f64,
    pub end: f64,
}
impl Settings {
    pub fn valid(self) -> bool {
        [self.start, self.end]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1e6).contains(v))
    }
    pub fn active(self) -> bool {
        self.start > 0.0 || self.end > 0.0
    }
    pub fn scale(self, at: f64, total: f64, finalizing: bool) -> f64 {
        if total <= 1e-9 {
            return 1.0;
        }
        let start = if self.start > 0.0 {
            (at / self.start).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let end = if finalizing && self.end > 0.0 {
            ((total - at) / self.end).clamp(0.0, 1.0)
        } else {
            1.0
        };
        start * end
    }
}

/// Check pressure independence and one-step publication in the installed binary.
pub fn verify() -> Result<(), String> {
    use crate::{
        assets::BrushTip,
        brush_cursor::Shape,
        document::{Background, CanvasSpec, Document},
        mouse_pressure::{Replay, Sample},
        painting::Style,
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 80,
            height: 32,
            dpi: 300.0,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let style = Style {
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: Shape {
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [40, 80, 120, 200],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    };
    let ends = Settings {
        start: 12.0,
        end: 12.0,
    };
    let mut r = Replay::begin(&d, 0, style, Default::default())?.configure_input(ends, true)?;
    for point in [[5.5, 16.5], [65.5, 16.5]] {
        r.push_input(
            Sample {
                point,
                micros: None,
            },
            0.5,
            0.0,
        )?;
    }
    let preview = r.render(&d, false)?;
    if d.snapshot() != before || preview.style().pressure != style.pressure {
        return Err("Geometry preview changed the source pressure/document".into());
    }
    let final_stroke = r.render(&d, true)?;
    if !final_stroke.commit_mouse(&mut d, r.metadata())? {
        return Err("Geometry stroke produced no pixels".into());
    }
    if d.composite_pixel(35, 16)[3] != 100 || d.composite_pixel(11, 16)[3] != 100 {
        return Err("Geometry taper altered pressure opacity".into());
    }
    let width = |x| (0..32).filter(|y| d.composite_pixel(x, *y)[3] > 0).count();
    if width(11) >= width(35) || width(59) >= width(35) || d.composite_pixel(67, 16)[3] != 0 {
        return Err("Geometry endpoints/tail are incorrect".into());
    }
    if d.scene.mouse_strokes[0].ends != ends || !d.scene.mouse_strokes[0].native_pressure {
        return Err("Frozen geometry settings missing".into());
    }
    let after = d.snapshot();
    if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
        return Err("Geometry history diverged".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn start_end_geometry_is_independent_from_native_pressure_and_history() {
        verify().unwrap();
    }
    #[test]
    fn independent_end_switches_overlap_dot_bounds_and_missing_legacy_fields() {
        let e = Settings {
            start: 10.0,
            end: 20.0,
        };
        assert_eq!(e.scale(5.0, 100.0, true), 0.5);
        assert_eq!(e.scale(90.0, 100.0, true), 0.5);
        assert_eq!(e.scale(90.0, 100.0, false), 1.0);
        assert_eq!(e.scale(0.0, 0.0, true), 1.0);
        assert_eq!(e.scale(5.0, 10.0, true), 0.125);
        assert_eq!(
            Settings {
                start: 0.0,
                end: 10.0
            }
            .scale(0.0, 20.0, true),
            1.0
        );
        assert_eq!(
            Settings {
                start: 10.0,
                end: 0.0
            }
            .scale(20.0, 20.0, true),
            1.0
        );
        for bad in [f64::NAN, f64::INFINITY, -1.0, 1e6 + 1.0] {
            assert!(
                !Settings {
                    start: bad,
                    end: 0.0
                }
                .valid()
            );
        }
        let mut old = serde_json::to_value(crate::tool_input::Settings::default()).unwrap();
        old.as_object_mut().unwrap().remove("ends");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .ends,
            [Settings::default(); 7]
        );
    }
}
