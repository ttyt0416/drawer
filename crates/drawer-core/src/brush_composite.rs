//! Independently defined brush/child-layer composition; all random decisions are coordinate seeded.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Normal,
    Erase,
    Dissolve,
    Background,
    ReplaceAlpha,
    BlackBurn,
    WhiteBurn,
    CompareDensity,
    EraseCompare,
}
impl Mode {
    pub const ALL: [Self; 9] = [
        Self::Normal,
        Self::Erase,
        Self::Dissolve,
        Self::Background,
        Self::ReplaceAlpha,
        Self::BlackBurn,
        Self::WhiteBurn,
        Self::CompareDensity,
        Self::EraseCompare,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Erase => "Erase",
            Self::Dissolve => "Dissolve",
            Self::Background => "Background",
            Self::ReplaceAlpha => "Replace alpha",
            Self::BlackBurn => "Black burn",
            Self::WhiteBurn => "White burn",
            Self::CompareDensity => "Compare density",
            Self::EraseCompare => "Erase (compare)",
        }
    }
    pub fn compose(
        self,
        base: [u8; 4],
        mut ink: [u8; 4],
        amount: f64,
        point: [i64; 2],
        seed: u32,
    ) -> [u8; 4] {
        let amount = amount.clamp(0.0, 1.0);
        if amount <= 0.0 {
            return base;
        }
        let sa = ink[3] as f64 / 255.0 * amount;
        let ba = base[3] as f64 / 255.0;
        match self {
            Self::Erase => [
                base[0],
                base[1],
                base[2],
                (base[3] as f64 * (1.0 - sa)).round() as u8,
            ],
            Self::ReplaceAlpha => [
                base[0],
                base[1],
                base[2],
                (base[3] as f64 * (1.0 - amount) + ink[3] as f64 * amount).round() as u8,
            ],
            Self::Background => composite(base, ink, ba + sa * (1.0 - ba), ba, sa * (1.0 - ba)),
            Self::CompareDensity => {
                if sa > ba {
                    [ink[0], ink[1], ink[2], (sa * 255.0).round() as u8]
                } else {
                    base
                }
            }
            Self::EraseCompare => {
                if sa >= ba && sa > 0.0 {
                    [base[0], base[1], base[2], 0]
                } else {
                    base
                }
            }
            Self::Dissolve => {
                if random(point, seed) < sa {
                    ink[3] = 255;
                    ink
                } else {
                    base
                }
            }
            Self::BlackBurn | Self::WhiteBurn => {
                if ba > 0.0 {
                    for c in 0..3 {
                        let b = base[c] as f64 / 255.0;
                        let s = ink[c] as f64 / 255.0;
                        let blend = if self == Self::BlackBurn {
                            if s == 0.0 {
                                0.0
                            } else {
                                (1.0 - (1.0 - b) / s).max(0.0)
                            }
                        } else if s == 1.0 {
                            1.0
                        } else {
                            (b / (1.0 - s)).min(1.0)
                        };
                        ink[c] = (blend * 255.0).round() as u8;
                    }
                }
                composite(base, ink, sa + ba * (1.0 - sa), ba * (1.0 - sa), sa)
            }
            Self::Normal => composite(base, ink, sa + ba * (1.0 - sa), ba * (1.0 - sa), sa),
        }
    }
}
fn composite(base: [u8; 4], ink: [u8; 4], alpha: f64, bw: f64, iw: f64) -> [u8; 4] {
    if alpha <= 0.0 {
        return base;
    }
    let mut result = [0; 4];
    for c in 0..3 {
        result[c] = ((base[c] as f64 * bw + ink[c] as f64 * iw) / alpha).round() as u8;
    }
    result[3] = (alpha * 255.0).round() as u8;
    result
}
pub(crate) fn random([x, y]: [i64; 2], seed: u32) -> f64 {
    let mut hash = (x as u64).wrapping_mul(0x9e3779b185ebca87)
        ^ (y as u64).wrapping_mul(0xc2b2ae3d27d4eb4f)
        ^ (seed as u64).wrapping_mul(0x165667b19e3779f9);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0x94d049bb133111eb);
    hash ^= hash >> 31;
    (hash >> 11) as f64 / (1u64 << 53) as f64
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub render_quality: crate::color_mix::Quality,
    pub mode: Mode,
    pub seed: u32,
    pub mixing_pressure: crate::brush_dynamics::Curve,
}
impl Settings {
    pub fn valid(self) -> bool {
        self.mixing_pressure.valid(1.0)
    }
}
fn style(composition: Settings) -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: crate::tip_profile::Settings {
            minimum_opacity: 1.0,
            ..Default::default()
        },
        texture: Default::default(),
        material: crate::brush_material::Settings {
            composition,
            ..Default::default()
        },
        shape: crate::brush_cursor::Shape {
            size: 12.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [220, 90, 40, 192],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    }
}
fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())
}
fn paint(
    mut d: crate::document::Document,
    s: crate::painting::Style,
    pressure: f64,
) -> Result<crate::document::Document, String> {
    let before = d.snapshot();
    let mut stroke = crate::painting::Stroke::begin(&d, 0, s)?;
    stroke.sample_pressure(&d, [8.5, 8.5], pressure)?;
    stroke.sample_pressure(&d, [24.5, 8.5], pressure)?;
    if d.snapshot() != before {
        return Err("Composition preview mutated original".into());
    }
    stroke.commit_mouse(
        &mut d,
        crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: s,
            count: 2,
            ends: Default::default(),
            native_pressure: true,
        },
    )?;
    let after = d.snapshot();
    if after != before
        && (!d.undo()
            || d.snapshot() != before
            || !d.redo()
            || d.snapshot() != after
            || d.history_counts() != (1, 0))
    {
        return Err("Composition history diverged".into());
    }
    Ok(d)
}
/// Actual raster, folder, metadata and history checks, also used by the installed app.
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for mode in Mode::ALL {
        let base = [80, 150, 210, 128];
        let mut d = document()?;
        d.layers[0].pixels.fill(base);
        let s = style(Settings {
            mode,
            seed: 71,
            ..Default::default()
        });
        let mut d = paint(d, s, 1.0)?;
        let expected = mode.compose(base, s.color, 1.0, [16, 8], 71);
        if d.layers[0].pixels[8 * 32 + 16] != expected {
            return Err(format!("Actual raster mode {mode:?} differs from rule"));
        }
        let path = root.join(format!("composition-{mode:?}.drawer"));
        let saved = d.snapshot();
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        if crate::files::open(&path, 65536)
            .map_err(|e| e.to_string())?
            .snapshot()
            != saved
        {
            return Err("Composition native pixels/seed/style lost".into());
        }
    }
    let mut d = document()?;
    d.layers[0].pixels.fill([50, 100, 200, 255]);
    d.layers.push(crate::document::RasterLayer {
        precise: None,
        locked: false,
        name: "Eraser child".into(),
        pixels: vec![[0, 0, 0, 128]; 512],
        placement: None,
    });
    let g = &mut d.scene.paper;
    g.layers.insert(
        1,
        crate::paper::Paper {
            blend: Mode::Erase,
            ..Default::default()
        },
    );
    let inner = g.group_all(2, "Inner")?;
    g.folders[inner].children = vec![crate::paper::Node::Raster(1)];
    g.roots = vec![
        crate::paper::Node::Raster(0),
        crate::paper::Node::Folder(inner),
    ];
    let outer = g.group_all(2, "Outer")?;
    let before = d.snapshot();
    let isolated = d.composite_pixel(1, 1);
    let mut scene = d.scene.clone();
    scene.paper.folders[inner].through = true;
    d.edit_scene(scene)?;
    let through = d.composite_pixel(1, 1);
    if isolated != [50, 100, 200, 255]
        || through != [50, 100, 200, 127]
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.composite_pixel(1, 1) != through
    {
        return Err("Nested Through/isolated folder composition or history failed".into());
    }
    let state = d.snapshot();
    if crate::paper::bake(&mut d, crate::paper::Node::Folder(outer)).is_ok()
        || d.snapshot() != state
        || crate::psd::layers_supported(&d)
    {
        return Err("External composition was silently baked/lost".into());
    }
    let path = root.join("through-folders.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, 65536)
        .map_err(|e| e.to_string())?
        .composite_pixel(1, 1)
        != through
    {
        return Err("Through folder not preserved in native file".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometric_quality_changes_edges_and_preserves_pixel_grid_and_tool_roles() {
        let mut s = style(Settings::default());
        s.shape.size = 3.5;
        s.shape.aspect = 0.6;
        s.shape.angle = 37.0;
        let a = paint(document().unwrap(), s, 1.0).unwrap().layers[0]
            .pixels
            .clone();
        s.material.composition.render_quality = crate::color_mix::Quality::High;
        let b = paint(document().unwrap(), s, 1.0).unwrap().layers[0]
            .pixels
            .clone();
        assert_ne!(a, b);
        assert!(b.iter().all(|p| p[3] <= 192));
        for tip in [
            crate::assets::BrushTip::Eraser,
            crate::assets::BrushTip::Pen,
        ] {
            s.tip = tip;
            assert_eq!(
                s.resolved().material.composition.render_quality,
                crate::color_mix::Quality::High
            );
        }
        s.clone_stamp = crate::clone_stamp::Stamp {
            enabled: true,
            offset: [0, 0],
        };
        assert_eq!(
            s.resolved().material.composition.render_quality,
            crate::color_mix::Quality::High
        );
        s.clone_stamp = Default::default();
        s.tip = crate::assets::BrushTip::PixelPen;
        let high = paint(document().unwrap(), s, 1.0).unwrap().layers[0]
            .pixels
            .clone();
        s.material.composition.render_quality = crate::color_mix::Quality::Normal;
        assert_eq!(
            high,
            paint(document().unwrap(), s, 1.0).unwrap().layers[0].pixels
        );
    }
    #[test]
    fn actual_alpha_grid_and_final_opacity_precede_density_comparison() {
        for mode in Mode::ALL {
            for base_alpha in [0, 128, 255] {
                for ink_alpha in [0, 128, 255] {
                    let base = [70, 140, 210, base_alpha];
                    let mut d = document().unwrap();
                    d.layers[0].pixels.fill(base);
                    let mut s = style(Settings {
                        mode,
                        seed: 31,
                        ..Default::default()
                    });
                    s.color[3] = ink_alpha;
                    let d = paint(d, s, 1.0).unwrap();
                    assert_eq!(
                        d.layers[0].pixels[8 * 32 + 16],
                        mode.compose(base, s.color, 1.0, [16, 8], 31),
                        "{mode:?}/{base_alpha}/{ink_alpha}"
                    );
                }
            }
            if mode != Mode::Normal {
                let base = [70, 140, 210, 128];
                let mut d = document().unwrap();
                d.layers[0].pixels.fill(base);
                let mut s = style(Settings {
                    mode,
                    seed: 31,
                    ..Default::default()
                });
                s.opacity = 0.25;
                let d = paint(d, s, 1.0).unwrap();
                assert_eq!(
                    d.layers[0].pixels[8 * 32 + 16],
                    mode.compose(base, s.color, 0.25, [16, 8], 31)
                );
            }
        }
        let legacy = serde_json::to_value(crate::brush_material::Settings::default()).unwrap();
        let mut legacy = legacy.as_object().unwrap().clone();
        legacy.remove("composition");
        let decoded: crate::brush_material::Settings =
            serde_json::from_value(legacy.into()).unwrap();
        assert_eq!(decoded.composition, Settings::default());
        let old_folder:crate::paper::Folder=serde_json::from_str(r#"{"name":"Old","children":[],"paper":{"pattern":{"source":0,"seed":0,"scale":1.0,"strength":0.0,"offset":[0.0,0.0],"angle":0.0},"origin":[0.0,0.0]}}"#).unwrap();
        assert!(!old_folder.through);
        assert_eq!(old_folder.paper.blend, Mode::Normal);
    }
    #[test]
    fn actual_pixels_folders_history_and_native_reopen() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn pressure_pickup_is_independent_of_opacity_and_quality() {
        let mut s = style(Settings {
            mixing_pressure: crate::brush_dynamics::Curve {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        });
        s.material.mode = crate::brush_material::Mode::RunningColor;
        s.material.mixing = 1.0;
        let render = |s, p| {
            let mut d = document().unwrap();
            d.layers[0].pixels.fill([0, 0, 255, 255]);
            paint(d, s, p).unwrap().layers[0].pixels.clone()
        };
        let light = render(s, 0.0);
        let hard = render(s, 1.0);
        assert_ne!(light, hard);
        assert_eq!(light[8 * 32 + 16], s.color);
        assert_eq!(hard[8 * 32 + 16], [0, 0, 255, 255]);
        s.material.composition.mixing_pressure.enabled = false;
        assert_eq!(render(s, 0.0), render(s, 1.0));
    }
    #[test]
    fn dissolve_masks_opacity_alpha_lock_water_and_repeat() {
        let mut s = style(Settings {
            mode: Mode::Dissolve,
            seed: 73,
            ..Default::default()
        });
        s.opacity = 0.5;
        s.color[3] = 255;
        let render = |s| {
            paint(document().unwrap(), s, 1.0).unwrap().layers[0]
                .pixels
                .clone()
        };
        let a = render(s);
        assert_eq!(a, render(s));
        assert!(a.iter().all(|p| p[3] == 0 || p[3] == 255));
        assert!(a.iter().any(|p| p[3] > 0));
        s.material.composition.seed = 74;
        assert_ne!(a, render(s));
        s.material.alpha_lock = true;
        assert!(render(s).iter().all(|p| p[3] == 0));
        s.material.alpha_lock = false;
        s.material.mode = crate::brush_material::Mode::Water;
        s.material.supply = 1.0;
        s.material.mixing = 0.0;
        assert!(render(s).iter().all(|p| p[3] == 0 || p[3] == 255));
        let mut d = document().unwrap();
        d.layers[0].pixels.fill([10, 20, 30, 128]);
        s.material.mode = crate::brush_material::Mode::Paint;
        s.material.composition.mode = Mode::ReplaceAlpha;
        s.opacity = 1.0;
        s.color[3] = 0;
        let replaced = paint(d, s, 1.0).unwrap();
        assert_eq!(replaced.layers[0].pixels[8 * 32 + 16], [10, 20, 30, 0]);
    }
    #[test]
    fn selection_texture_and_global_opacity_apply_once_to_special_modes() {
        for mode in Mode::ALL.into_iter().filter(|m| *m != Mode::Normal) {
            let base = [70, 140, 210, 128];
            let mut d = document().unwrap();
            d.layers[0].pixels.fill(base);
            d.scene.selection_mask = Some(crate::selection::Mask {
                runs: vec![crate::selection::Run {
                    x: 16,
                    y: 8,
                    length: 1,
                    alpha: 128,
                }],
            });
            let mut s = style(Settings {
                mode,
                seed: 73,
                ..Default::default()
            });
            s.opacity = 0.5;
            s.texture = crate::texture::Pattern {
                source: 1,
                strength: 0.7,
                ..Default::default()
            };
            let factor = 128.0 / 255.0 * 0.5 * s.texture.factor([16.5, 8.5]);
            let d = paint(d, s, 1.0).unwrap();
            assert_eq!(
                d.layers[0].pixels[8 * 32 + 16],
                mode.compose(base, s.color, factor, [16, 8], 73),
                "{mode:?}"
            );
            assert_eq!(d.layers[0].pixels[8 * 32 + 15], base);
        }
    }
    #[test]
    fn independent_alpha_rgb_rules_and_endpoints() {
        let p = [0, 0];
        let b = [100, 150, 200, 128];
        let s = [200, 100, 50, 128];
        assert_eq!(Mode::Erase.compose(b, s, 1.0, p, 0), [100, 150, 200, 64]);
        assert_eq!(
            Mode::Background.compose(b, s, 1.0, p, 0),
            [133, 133, 150, 192]
        );
        assert_eq!(
            Mode::ReplaceAlpha.compose(b, [1, 2, 3, 0], 0.5, p, 0),
            [100, 150, 200, 64]
        );
        assert_eq!(Mode::CompareDensity.compose(b, s, 1.0, p, 0), b);
        assert_eq!(
            Mode::CompareDensity.compose(b, [200, 100, 50, 255], 1.0, p, 0),
            [200, 100, 50, 255]
        );
        assert_eq!(
            Mode::EraseCompare.compose(b, s, 1.0, p, 0),
            [100, 150, 200, 0]
        );
        assert_eq!(Mode::EraseCompare.compose(b, s, 0.5, p, 0), b);
        assert_eq!(
            Mode::BlackBurn.compose([128; 4], [128, 128, 128, 255], 1.0, p, 0),
            [2, 2, 2, 255]
        );
        assert_eq!(
            Mode::WhiteBurn.compose([128; 4], [128, 128, 128, 255], 1.0, p, 0),
            [255; 4]
        );
        for mode in Mode::ALL {
            assert_eq!(mode.compose(b, s, 0.0, p, 0), b);
            if mode != Mode::ReplaceAlpha {
                assert_eq!(mode.compose(b, [200, 100, 50, 0], 1.0, p, 0), b);
            }
            assert_eq!(
                mode.compose([1, 2, 3, 0], [200, 100, 50, 0], 1.0, p, 0),
                [1, 2, 3, 0]
            );
        }
        assert_eq!(
            Mode::Background.compose([10, 20, 30, 255], [250; 4], 1.0, p, 0),
            [10, 20, 30, 255]
        );
    }
    #[test]
    fn dissolve_binary_coverage_frequency_repeat_and_seed() {
        let draw = |seed| {
            (0..10000)
                .map(|n| {
                    Mode::Dissolve.compose(
                        [0; 4],
                        [10, 20, 30, 255],
                        0.25,
                        [n % 100, n / 100],
                        seed,
                    )
                })
                .collect::<Vec<_>>()
        };
        let a = draw(7);
        assert_eq!(a, draw(7));
        assert_ne!(a, draw(8));
        let painted = a.iter().filter(|p| p[3] == 255).count();
        assert!((2300..2700).contains(&painted));
        assert!(a.iter().all(|p| p[3] == 0 || p[3] == 255));
        assert!(Settings::default().valid());
        let bad = Settings {
            mixing_pressure: crate::brush_dynamics::Curve {
                maximum: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!bad.valid());
    }
}
