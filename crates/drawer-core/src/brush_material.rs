//! Owned pigment model: carried color, supply, pickup, dilution and alpha constraints.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Paint,
    Blend,
    RunningColor,
    Smudge,
    Water,
}
impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Paint,
        Self::Blend,
        Self::RunningColor,
        Self::Smudge,
        Self::Water,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Paint => "None",
            Self::Blend => "Blend",
            Self::RunningColor => "Running color",
            Self::Smudge => "Smear",
            Self::Water => "Watercolor",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub space: crate::color_mix::Space,
    #[serde(default)]
    pub quality: crate::color_mix::Quality,
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
            space: Default::default(),
            quality: Default::default(),
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
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
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
        space: Default::default(),
        quality: Default::default(),
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

fn verify_numeric_controls() -> Result<(), String> {
    let base = Settings {
        space: Default::default(),
        quality: Default::default(),
        mode: Mode::Water,
        supply: 0.3,
        mixing: 0.4,
        retention: 0.6,
        smudge_distance: 17.0,
        water: 0.7,
        dilution: 0.2,
        keep_density: true,
        alpha_lock: false,
        colorless: false,
    };
    for field in 0..6 {
        let mut rendered = None;
        for value in [0.0, 1.0] {
            let mut changed = base;
            match field {
                0 => changed.supply = value,
                1 => changed.mixing = value,
                2 => changed.retention = value,
                3 => changed.water = value,
                4 => changed.dilution = value,
                _ => changed.smudge_distance = value * 4096.0,
            };
            let (d, _) = probe(changed, 0.25)?;
            if d.layers[0]
                .pixels
                .iter()
                .enumerate()
                .any(|(n, p)| n % 64 >= 32 && p[3] > 64)
            {
                return Err("Pigment controls exceeded the stroke opacity cap".into());
            }
            let pixels = d.layers[0].pixels.clone();
            if rendered.as_ref() == Some(&pixels) {
                return Err(format!(
                    "Control {} had no effect with other values fixed",
                    [
                        "supply",
                        "pickup",
                        "retention",
                        "water",
                        "dilution",
                        "smear distance"
                    ][field]
                ));
            }
            rendered = Some(pixels);
        }
    }
    Ok(())
}
/// Boundary checks for independent controls, also run in the installed process.
pub fn verify_independent() -> Result<(), String> {
    verify_numeric_controls()?;
    let base = Settings {
        space: Default::default(),
        quality: Default::default(),
        mode: Mode::Water,
        supply: 0.0,
        mixing: 1.0,
        retention: 0.0,
        water: 0.0,
        ..Default::default()
    };
    let (dry, _) = probe(
        Settings {
            supply: 1.0,
            dilution: 0.0,
            ..base
        },
        1.0,
    )?;
    let (half, _) = probe(
        Settings {
            supply: 1.0,
            dilution: 0.5,
            ..base
        },
        1.0,
    )?;
    let (none, before) = probe(
        Settings {
            supply: 1.0,
            dilution: 1.0,
            ..base
        },
        1.0,
    )?;
    if dry.composite_pixel(48, 16)[3] != 255
        || half.composite_pixel(48, 16)[3] != 128
        || none.snapshot() != before
    {
        return Err("Dilution stopped working when water was zero".into());
    }
    let (local, _) = probe(
        Settings {
            smudge_distance: 0.0,
            ..base
        },
        1.0,
    )?;
    let (no_pickup, _) = probe(
        Settings {
            smudge_distance: 0.0,
            mixing: 0.0,
            ..base
        },
        1.0,
    )?;
    let (retained, _) = probe(
        Settings {
            smudge_distance: 0.0,
            retention: 1.0,
            ..base
        },
        1.0,
    )?;
    if local.composite_pixel(24, 16)[2] != 255
        || no_pickup.composite_pixel(24, 16)[0] != 255
        || retained.composite_pixel(24, 16)[0] != 255
    {
        return Err("Zero smear distance overrode pickup or carried color retention".into());
    }
    for water in [0.0, 1.0] {
        for dilution in [0.0, 1.0] {
            for keep_density in [false, true] {
                let (locked, original) = probe(
                    Settings {
                        supply: 0.5,
                        water,
                        dilution,
                        keep_density,
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
                {
                    return Err("Alpha lock changed with independent pigment controls".into());
                }
            }
        }
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
                retention: 0.5,
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
                retention: 0.5,
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
        assert!(sequence.materials[..4].iter().all(Settings::is_default));
        assert_eq!(sequence.materials[5].mode, Mode::Smudge);
    }
    #[test]
    fn zero_water_dilution_and_zero_smear_pickup_retention_are_independent() {
        verify_independent().unwrap();
    }

    #[test]
    fn each_numeric_control_changes_pixels_with_the_other_parameters_fixed() {
        verify_numeric_controls().unwrap();
    }
    #[test]
    fn eraser_ignores_pigment_only_options_and_alpha_lock_preserves_hidden_rgb() {
        use crate::{
            assets::BrushTip,
            brush_cursor::Shape,
            document::{Background, CanvasSpec, Document},
            painting::{Stroke, Style},
        };
        let mut results = Vec::new();
        for pigment in [false, true] {
            let mut d = Document::create(
                CanvasSpec {
                    width: 3,
                    height: 1,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                1024,
            )
            .unwrap();
            d.layers[0].pixels = vec![[15, 22, 33, 0], [80, 90, 100, 128], [120, 130, 140, 255]];
            let style = Style {
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: if pigment {
                    Settings {
                        mode: Mode::Water,
                        supply: 0.0,
                        mixing: 1.0,
                        retention: 0.0,
                        water: 1.0,
                        dilution: 1.0,
                        keep_density: true,
                        ..Default::default()
                    }
                } else {
                    Default::default()
                },
                shape: Shape {
                    size: 6.0,
                    aspect: 1.0,
                    angle: 0.0,
                },
                tip: BrushTip::Eraser,
                color: [0, 255, 0, 255],
                smoothing: 0.0,
                opacity: 1.0,
                hardness: 1.0,
                spacing: 0.15,
                pressure: Default::default(),
            };
            let before = d.snapshot();
            let mut locked = style;
            locked.material.alpha_lock = true;
            let mut stroke = Stroke::begin(&d, 0, locked).unwrap();
            stroke.sample(&d, [1.5, 0.5]).unwrap();
            assert!(!stroke.commit(&mut d).unwrap());
            assert!(d.snapshot() == before);
            let mut stroke = Stroke::begin(&d, 0, style).unwrap();
            stroke.sample(&d, [1.5, 0.5]).unwrap();
            stroke.commit(&mut d).unwrap();
            assert!(d.layers[0].pixels.iter().all(|p| p[3] == 0));
            results.push(d.layers.remove(0).pixels);
        }
        assert_eq!(results[0], results[1]);
    }
}

/// Independent stripe checks also run in the installed application.
pub fn verify_mix_modes() -> Result<(), String> {
    use crate::color_mix::{Quality, Space};
    let mut results = Vec::new();
    for mode in [Mode::Paint, Mode::Blend, Mode::RunningColor, Mode::Smudge] {
        let (d, _) = probe(
            Settings {
                mode,
                supply: 0.4,
                mixing: 0.7,
                retention: 0.8,
                smudge_distance: 16.0,
                ..Default::default()
            },
            1.0,
        )?;
        let pixels = d.layers[0].pixels.clone();
        if results.contains(&pixels) {
            return Err(format!("{} duplicated another mix mode", mode.label()));
        }
        results.push(pixels);
    }
    let base = Settings {
        mode: Mode::Blend,
        supply: 0.4,
        mixing: 0.7,
        ..Default::default()
    };
    let (standard, _) = probe(base, 1.0)?;
    let (perceptual, _) = probe(
        Settings {
            space: Space::Perceptual,
            ..base
        },
        1.0,
    )?;
    if standard.layers[0].pixels == perceptual.layers[0].pixels {
        return Err("Perceptual pigment mode had no effect".into());
    }
    use crate::{
        assets::BrushTip,
        brush_cursor::Shape,
        painting::{Stroke, Style},
    };
    let render = |quality, mode| -> Result<Vec<[u8; 4]>, String> {
        let (mut d, _) = probe(Settings::default(), 0.0)?;
        let s = Style {
            variation: Default::default(),
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Default::default(),
            texture: Default::default(),
            material: Settings {
                mode,
                quality,
                supply: 0.0,
                mixing: 1.0,
                ..Default::default()
            },
            shape: Shape {
                size: 12.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip: BrushTip::Pen,
            color: [0; 4],
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
        let mut st = Stroke::begin(&d, 0, s)?;
        st.direct_dab(&d, [16.5, 16.5], [16.5, 16.5], 1.0)?;
        st.commit(&mut d)?;
        Ok(d.layers.remove(0).pixels)
    };
    if render(Quality::Normal, Mode::RunningColor)? == render(Quality::High, Mode::RunningColor)? {
        return Err("Running color footprint quality was ignored".into());
    }
    if render(Quality::Normal, Mode::Blend)? != render(Quality::High, Mode::Blend)? {
        return Err("Running quality leaked into another mode".into());
    }
    crate::color_mix::verify()
}
#[cfg(test)]
mod mix_mode_tests {
    #[test]
    fn stripe_modes_perceptual_and_running_quality_are_independent() {
        super::verify_mix_modes().unwrap();
    }
}
