//! Owned pigment model: carried color, supply, pickup, dilution and alpha constraints.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Paint,
    Blend,
    Smudge,
    Water,
}
impl Mode {
    pub const ALL: [Self; 4] = [Self::Paint, Self::Blend, Self::Smudge, Self::Water];
    pub fn label(self) -> &'static str {
        match self {
            Self::Paint => "Paint",
            Self::Blend => "Blend",
            Self::Smudge => "Smudge",
            Self::Water => "Watercolor",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub supply: f64,
    pub mixing: f64,
    pub retention: f64,
    pub smudge_distance: f64,
    pub water: f64,
    pub dilution: f64,
    pub keep_density: bool,
    pub alpha_lock: bool,
    pub colorless: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Paint,
            supply: 1.0,
            mixing: 0.5,
            retention: 0.5,
            smudge_distance: 12.0,
            water: 0.5,
            dilution: 0.0,
            keep_density: false,
            alpha_lock: false,
            colorless: false,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        [
            self.supply,
            self.mixing,
            self.retention,
            self.water,
            self.dilution,
        ]
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.smudge_distance.is_finite()
            && (0.0..=4096.0).contains(&self.smudge_distance)
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn active(self) -> bool {
        !self.is_default()
    }
    pub fn pigment(self) -> bool {
        self.mode != Mode::Paint || self.supply != 1.0 || self.colorless
    }
}

fn probe(
    material: Settings,
    opacity: f64,
) -> Result<(crate::document::Document, crate::sizing::DocumentState), String> {
    probe_in(material, opacity, false)
}
fn probe_in(
    material: Settings,
    opacity: f64,
    hole: bool,
) -> Result<(crate::document::Document, crate::sizing::DocumentState), String> {
    use crate::{
        assets::BrushTip,
        brush_cursor::Shape,
        document::{Background, CanvasSpec, Document},
        painting::{Stroke, Style},
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 32,
            dpi: 300.0,
            background: Background::Transparent,
        },
        131072,
    )
    .map_err(|e| e.to_string())?;
    for y in 0..32 {
        for x in 0..32 {
            d.layers[0].pixels[y * 64 + x] = if x < 16 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 128]
            };
        }
    }
    if hole {
        d.layers[0].pixels[16 * 64 + 8] = [0; 4];
    }
    d.mark_saved("material-baseline.drawer".into());
    let before = d.snapshot();
    let style = Style {
        texture: Default::default(),
        material,
        opacity,
        hardness: 1.0,
        spacing: 0.15,
        shape: Shape {
            size: 3.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [0, 255, 0, 255],
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    };
    let mut stroke = Stroke::begin(&d, 0, style)?;
    for point in [[8.5, 16.5], [24.5, 16.5], [48.5, 16.5]] {
        stroke.direct_dab(&d, point, point, 1.0)?;
    }
    if d.snapshot() != before {
        return Err("Pigment preview mutated the original layer".into());
    }
    stroke.commit(&mut d)?;
    Ok((d, before))
}
/// Same pigment/alpha checks run in unit tests and the installed native process.
pub fn verify() -> Result<(), String> {
    let base = Settings {
        mode: Mode::Water,
        supply: 0.0,
        mixing: 1.0,
        retention: 0.0,
        water: 0.0,
        ..Default::default()
    };
    let (plain, _) = probe(base, 1.0)?;
    let (mut keep, before) = probe(
        Settings {
            keep_density: true,
            ..base
        },
        1.0,
    )?;
    if plain.composite_pixel(48, 16)[3] != 0
        || keep.composite_pixel(48, 16)[3] == 0
        || keep.composite_pixel(48, 16)[2] == 0
    {
        return Err(
            "Density retention did not carry existing blue pigment into empty pixels".into(),
        );
    }
    let after = keep.snapshot();
    if keep.history_counts() != (1, 0)
        || !keep.undo()
        || keep.snapshot() != before
        || !keep.redo()
        || keep.snapshot() != after
    {
        return Err("Watercolor did not undo/redo as one stroke".into());
    }
    let (retained, _) = probe(
        Settings {
            retention: 1.0,
            ..base
        },
        1.0,
    )?;
    let (no_pickup, _) = probe(
        Settings {
            mixing: 0.0,
            ..base
        },
        1.0,
    )?;
    if retained.composite_pixel(24, 16)[0] <= plain.composite_pixel(24, 16)[0]
        || no_pickup.composite_pixel(24, 16)[0] <= plain.composite_pixel(24, 16)[0]
    {
        return Err("Retention and layer pickup did not remain independent".into());
    }
    let (foreground, _) = probe(
        Settings {
            supply: 1.0,
            ..base
        },
        1.0,
    )?;
    if foreground.composite_pixel(8, 16)[1] != 255 || plain.composite_pixel(8, 16)[1] != 0 {
        return Err("Foreground supply was not independent".into());
    }
    let (replenished, _) = probe(
        Settings {
            supply: 0.2,
            keep_density: true,
            ..base
        },
        1.0,
    )?;
    if replenished.composite_pixel(48, 16)[2] == 0 || replenished.composite_pixel(48, 16)[1] == 0 {
        return Err("Density retention replaced carried hue instead of replenishing it".into());
    }
    let (smudge, _) = probe(
        Settings {
            mode: Mode::Smudge,
            retention: 1.0,
            keep_density: true,
            ..base
        },
        1.0,
    )?;
    if smudge.layers[0].pixels.iter().any(|p| p[1] != 0) || smudge.composite_pixel(48, 16)[0] == 0 {
        return Err("Colorless smudge injected foreground or lost carried pigment".into());
    }
    let (locked, original) = probe(
        Settings {
            supply: 1.0,
            water: 1.0,
            keep_density: true,
            alpha_lock: true,
            ..base
        },
        1.0,
    )?;
    if locked.layers[0]
        .pixels
        .iter()
        .zip(&original.layers[0].pixels)
        .any(|(a, b)| a[3] != b[3])
        || locked.composite_pixel(8, 16)[1] == 0
    {
        return Err(
            "Alpha lock failed to preserve opaque/partial/empty alpha while changing color".into(),
        );
    }
    let (diluted, original) = probe(
        Settings {
            supply: 1.0,
            water: 1.0,
            dilution: 1.0,
            ..base
        },
        1.0,
    )?;
    if diluted.snapshot() != original {
        return Err("Full dilution injected paint".into());
    }
    let (wet, original) = probe_in(
        Settings {
            supply: 1.0,
            water: 1.0,
            ..base
        },
        0.25,
        true,
    )?;
    if wet.layers[0]
        .pixels
        .iter()
        .zip(&original.layers[0].pixels)
        .any(|(a, b)| b[3] == 0 && a[3] > 64)
    {
        return Err("Water diffusion exceeded stroke opacity".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pigment_pickup_supply_retention_density_alpha_and_dilution_are_distinct() {
        verify().unwrap();
    }
    #[test]
    fn alpha_lock_precedes_density_and_no_supply_never_injects_foreground_in_boundary_combinations()
    {
        for retention in [0.0, 1.0] {
            for supply in [0.0, 1.0] {
                for keep_density in [false, true] {
                    for alpha_lock in [false, true] {
                        let settings = Settings {
                            mode: Mode::Water,
                            retention,
                            supply,
                            keep_density,
                            alpha_lock,
                            mixing: 1.0,
                            water: 1.0,
                            ..Default::default()
                        };
                        let (d, before) = probe(settings, 1.0).unwrap();
                        if alpha_lock {
                            assert!(
                                d.layers[0]
                                    .pixels
                                    .iter()
                                    .zip(&before.layers[0].pixels)
                                    .all(|(a, b)| a[3] == b[3])
                            );
                        }
                        if supply == 0.0 {
                            assert!(d.layers[0].pixels.iter().all(|p| p[1] == 0));
                        }
                    }
                }
            }
        }
        let (a, _) = probe(
            Settings {
                mode: Mode::Smudge,
                retention: 1.0,
                smudge_distance: 0.0,
                water: 0.0,
                ..Default::default()
            },
            1.0,
        )
        .unwrap();
        let (b, _) = probe(
            Settings {
                mode: Mode::Smudge,
                retention: 1.0,
                smudge_distance: 4096.0,
                water: 0.0,
                ..Default::default()
            },
            1.0,
        )
        .unwrap();
        assert_ne!(a.composite_pixel(48, 16), b.composite_pixel(48, 16));
    }
    #[test]
    fn wet_diffusion_applies_global_opacity_once_even_inside_an_opaque_surrounding() {
        let (d, before) = probe_in(
            Settings {
                mode: Mode::Water,
                water: 1.0,
                supply: 1.0,
                ..Default::default()
            },
            0.25,
            true,
        )
        .unwrap();
        assert_eq!(before.layers[0].pixels[16 * 64 + 8][3], 0);
        assert!(d.composite_pixel(8, 16)[3] > 0 && d.composite_pixel(8, 16)[3] <= 64);
    }
    #[test]
    fn invalid_material_limits_and_legacy_style_defaults_reject_without_changes() {
        for bad in [-0.1, 1.01, f64::NAN, f64::INFINITY] {
            for field in 0..5 {
                let mut s = Settings::default();
                match field {
                    0 => s.supply = bad,
                    1 => s.mixing = bad,
                    2 => s.retention = bad,
                    3 => s.water = bad,
                    _ => s.dilution = bad,
                };
                assert!(!s.valid());
            }
        }
        for distance in [0.0, 4096.0] {
            assert!(
                Settings {
                    smudge_distance: distance,
                    ..Default::default()
                }
                .valid()
            );
        }
        assert!(
            !Settings {
                smudge_distance: 4096.1,
                ..Default::default()
            }
            .valid()
        );
        let mut old = serde_json::to_value(crate::input_replay::Sequence::default()).unwrap();
        old.as_object_mut().unwrap().remove("materials");
        let sequence: crate::input_replay::Sequence = serde_json::from_value(old).unwrap();
        assert!(sequence.materials.iter().all(Settings::is_default));
    }
}
