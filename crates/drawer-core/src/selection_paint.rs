//! Selection integration fixtures for the shared raster brush pipeline.
use crate::{
    assets::BrushTip,
    document::{Background, CanvasSpec, Document},
    painting::{Stroke, Style},
    selection::{Mask, Run},
};
pub fn style(tip: BrushTip) -> Result<Style, String> {
    let workspace = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    let mut s = crate::brush_presets::Preset::capture(&workspace, tip, "Own selection fixture")?
        .style([220, 40, 20, 128]);
    s.shape = crate::brush_cursor::Shape {
        size: 8.0,
        aspect: 1.0,
        angle: 0.0,
    };
    s.pressure.size = false;
    s.pressure.opacity = false;
    s.opacity = 1.0;
    s.hardness = 1.0;
    s.smoothing = 0.0;
    s.material = Default::default();
    s.profile.image = crate::tip_profile::Image::Square;
    s.profile.minimum_opacity = 1.0;
    s.profile.pixel.pressure = false;
    Ok(s)
}
fn doc(base: [u8; 4], mask: u8) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill(base);
    d.scene.selection_mask = Some(Mask {
        runs: if mask == 0 {
            vec![]
        } else {
            (0..16)
                .map(|y| Run {
                    x: 0,
                    y,
                    length: 16,
                    alpha: mask,
                })
                .collect()
        },
    });
    Ok(d)
}
fn draw(d: &mut Document, s: Style) -> Result<bool, String> {
    let mut stroke = Stroke::begin(d, 0, s)?;
    stroke.set_seed(19);
    stroke.direct_dab(d, [8.5, 8.5], [8.5, 8.5], 1.0)?;
    stroke.finish_material(d, || false)?;
    stroke.commit(d)
}
pub(crate) fn expected(a: [u8; 4], b: [u8; 4], coverage: u8) -> [u8; 4] {
    if coverage == 255 {
        return b;
    }
    let w = coverage as u64;
    let alpha = a[3] as u64 * (255 - w) + b[3] as u64 * w;
    if alpha == 0 {
        return a;
    }
    let mut c = a;
    for n in 0..3 {
        c[n] = ((a[n] as u64 * a[3] as u64 * (255 - w) + b[n] as u64 * b[3] as u64 * w + alpha / 2)
            / alpha) as u8;
    }
    c[3] = ((alpha + 127) / 255) as u8;
    c
}
pub fn verify_overwrite() -> Result<(), String> {
    let mut s = style(BrushTip::PixelPen)?;
    s.profile.pixel.overwrite = true;
    for ba in [128, 0, 255] {
        for ia in [128, 0, 255] {
            for opacity in [1.0, 0.5] {
                let base = [20, 80, 160, ba];
                s.color = [220, 40, 20, ia];
                s.opacity = opacity;
                let target = [220, 40, 20, (ia as f64 * opacity).round() as u8];
                for coverage in [0, 128, 255] {
                    let mut d = doc(base, coverage)?;
                    draw(&mut d, s)?;
                    let actual = d.layers[0].pixel(16, 16, 8, 8);
                    let want = expected(base, target, coverage);
                    if actual != want {
                        return Err(format!(
                            "RGBA overwrite selection {coverage}, base alpha {ba}, ink alpha {ia}, opacity {opacity}: got {actual:?}, expected {want:?}"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn verify_hidden_erase() -> Result<(), String> {
    for tip in BrushTip::ALL {
        for transparent in [false, true] {
            if !transparent && tip != BrushTip::Eraser {
                continue;
            }
            let mut s = style(tip)?;
            s.profile.transparent = transparent;
            for coverage in [0, 128, 255] {
                let base = [20, 80, 160, 0];
                let mut d = doc(base, coverage)?;
                let before = d.snapshot();
                if draw(&mut d, s)? || d.snapshot() != before || d.history_counts() != (0, 0) {
                    return Err(format!(
                        "{tip:?} hidden transparent RGB changed under selection {coverage}"
                    ));
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn rgba_overwrite_respects_soft_selection() {
        super::verify_overwrite().unwrap();
    }
    #[test]
    fn partial_erase_preserves_zero_alpha_hidden_rgb() {
        super::verify_hidden_erase().unwrap();
    }
}

pub fn verify_tips(root: &std::path::Path) -> Result<(), String> {
    for tip in BrushTip::ALL {
        let base = if tip == BrushTip::Eraser {
            [20, 80, 160, 255]
        } else if tip == BrushTip::Smudge {
            [20, 80, 160, 128]
        } else {
            [20, 80, 160, 0]
        };
        let s = style(tip)?;
        let mut full = doc(base, 255)?;
        draw(&mut full, s)?;
        let full_pixels = full.layers[0].pixels.clone();
        for coverage in [0, 128, 255] {
            let mut d = doc(base, coverage)?;
            let before = d.snapshot();
            let changed = draw(&mut d, s)?;
            for (actual, target) in d.layers[0].pixels.iter().zip(&full_pixels) {
                let want = expected(base, *target, coverage);
                if (0..4).any(|n| actual[n].abs_diff(want[n]) > 1) {
                    return Err(format!(
                        "{tip:?} selection {coverage} did not blend original/target once: {actual:?}/{want:?}"
                    ));
                }
            }
            let after = d.snapshot();
            if changed {
                if d.history_counts() != (1, 0)
                    || !d.undo()
                    || d.snapshot() != before
                    || !d.redo()
                    || d.snapshot() != after
                {
                    return Err("Masked tip undo/redo lost pixels or selection".into());
                }
            } else if d.snapshot() != before || d.history_counts() != (0, 0) {
                return Err("Zero-effect selection created history".into());
            }
            if tip == BrushTip::Pen && coverage == 128 {
                let path = root.join("selected-paint.drawer");
                crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
                if crate::files::open(&path, 65536)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    != after
                {
                    return Err("Mask/paint native roundtrip failed".into());
                }
            }
        }
    }
    Ok(())
}
fn path(d: &mut Document, s: Style) -> Result<bool, String> {
    let mut stroke = Stroke::begin(d, 0, s)?;
    stroke.set_seed(19);
    stroke.sample(d, [3.5, 8.5])?;
    stroke.sample(d, [13.5, 8.5])?;
    stroke.finish_material(d, || false)?;
    stroke.commit(d)
}
pub fn verify_paths() -> Result<(), String> {
    let pen = style(BrushTip::Pen)?;
    let mut cases = Vec::new();
    for tip in BrushTip::ALL {
        cases.push((format!("{tip:?}"), style(tip)?));
        let mut s = style(tip)?;
        s.profile.transparent = true;
        cases.push((format!("transparent {tip:?}"), s));
    }
    for mode in crate::brush_material::Mode::ALL {
        let mut s = pen;
        s.material.mode = mode;
        s.material.water = 1.0;
        s.material.edge = 0.8;
        s.material.retention = 0.8;
        cases.push((format!("material {mode:?}"), s));
    }
    for mode in crate::brush_composite::Mode::ALL {
        let mut s = pen;
        s.material.composition.mode = mode;
        cases.push((format!("compose {mode:?}"), s));
    }
    for mode in [crate::tonal::Mode::Dodge, crate::tonal::Mode::Burn] {
        let mut s = pen;
        s.material.tonal.mode = mode;
        s.material.tonal.band = crate::tonal::Band::Shadows;
        cases.push((format!("tonal {mode:?}"), s));
    }
    for mode in crate::brush_pattern::Mode::ALL {
        let mut s = pen;
        s.pattern.mode = mode;
        s.pattern.width = 8.0;
        cases.push((format!("pattern {mode:?}"), s));
    }
    let mut s = pen;
    s.clone_stamp = crate::clone_stamp::Stamp {
        enabled: true,
        offset: [-4, 0],
    };
    cases.push(("clone".into(), s));
    let mut s = pen;
    s.variation.dual.enabled = true;
    s.variation.dual.controls.rgb = true;
    cases.push(("dual RGB".into(), s));
    let mut s = pen;
    s.texture.source = 1;
    s.texture.strength = 0.8;
    cases.push(("owned texture".into(), s));
    let mut s = style(BrushTip::PixelPen)?;
    s.profile.pixel.overwrite = true;
    cases.push(("RGBA overwrite".into(), s));
    for (name, style) in cases {
        for empty in [false, true] {
            let mut d = doc([20, 80, 160, 128], 0)?;
            for y in 0..16 {
                for x in 0..16 {
                    d.layers[0].pixels[y * 16 + x] = [20 + x as u8 * 5, 80, 160 - x as u8 * 4, 128];
                }
            }
            if !empty {
                d.scene.selection_mask = Some(Mask {
                    runs: (0..16)
                        .flat_map(|y| {
                            [
                                Run {
                                    x: 8,
                                    y,
                                    length: 4,
                                    alpha: 128,
                                },
                                Run {
                                    x: 12,
                                    y,
                                    length: 4,
                                    alpha: 255,
                                },
                            ]
                        })
                        .collect(),
                });
            }
            let before = d.snapshot();
            let changed = path(&mut d, style)?;
            for y in 0..16 {
                for x in 0..16 {
                    if (empty || x < 8)
                        && d.layers[0].pixels[y * 16 + x] != before.layers[0].pixels[y * 16 + x]
                    {
                        return Err(format!("{name} modified zero selection at {x},{y}"));
                    }
                }
            }
            let after = d.snapshot();
            if changed {
                if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
                    return Err(format!("{name} masked path history mismatch"));
                }
            } else if d.snapshot() != before || d.history_counts() != (0, 0) {
                return Err(format!("{name} no-op created data/history"));
            }
        }
    }
    for coverage in [0, 128, 255] {
        let mut d = doc([20, 80, 160, 255], coverage)?;
        d.scene.lines.settings.erase = vec![crate::line_tools::Target::Raster(0)];
        let before = d.snapshot();
        crate::line_tools::erase(&mut d, [8.5, 8.5], 4.0)?;
        let p = d.layers[0].pixel(16, 16, 8, 8);
        if p != [20, 80, 160, 255 - coverage] {
            return Err("Multi-target raster erase ignored selection coverage".into());
        }
        if coverage == 0 && (d.snapshot() != before || d.history_counts() != (0, 0)) {
            return Err("Zero selected erase changed data".into());
        }
    }
    Ok(())
}
pub fn verify_placed() -> Result<(), String> {
    let mut d = doc([20, 80, 160, 128], 128)?;
    d.layers[0].placement = Some(crate::document::PlacedRaster {
        width: 24,
        height: 24,
        offset: [-4, -4],
    });
    d.layers[0].pixels = vec![[20, 80, 160, 128]; 24 * 24];
    let before = d.snapshot();
    let mut s = style(BrushTip::PixelPen)?;
    s.profile.pixel.overwrite = true;
    s.shape.size = 16.0;
    let mut stroke = Stroke::begin(&d, 0, s)?;
    stroke.direct_dab(&d, [0.5, 0.5], [0.5, 0.5], 1.0)?;
    stroke.commit(&mut d)?;
    if d.layers[0].pixel(16, 16, 0, 0) != [120, 60, 90, 128] {
        return Err("Placed raster soft-selection overwrite mismatch".into());
    }
    for y in 0..24 {
        for x in 0..24 {
            let p = [x as i32 - 4, y as i32 - 4];
            if (p[0] < 0 || p[1] < 0 || p[0] >= 16 || p[1] >= 16)
                && d.layers[0].pixels[y * 24 + x] != before.layers[0].pixels[y * 24 + x]
            {
                return Err("Selection changed off-canvas placed pixels".into());
            }
        }
    }
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    verify_overwrite()?;
    verify_hidden_erase()?;
    verify_tips(root)?;
    verify_paths()?;
    verify_placed()?;
    verify_material_strength()
}
#[cfg(test)]
mod integration_tests {
    #[test]
    fn seven_tips_coverage_undo_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify_tips(&root.0).unwrap();
    }
    #[test]
    fn pigment_composition_pattern_clone_dual_and_erase_stencils() {
        super::verify_paths().unwrap();
        super::verify_placed().unwrap();
    }
}

pub fn verify_material_strength() -> Result<(), String> {
    for mode in crate::brush_material::Mode::ALL {
        let mut s = style(BrushTip::Pen)?;
        s.material.mode = mode;
        s.material.water = 1.0;
        s.material.edge = 0.0;
        s.color[3] = 192;
        let base = [20, 80, 160, 128];
        let mut full = doc(base, 255)?;
        draw(&mut full, s)?;
        for coverage in [0, 128, 255] {
            let mut d = doc(base, coverage)?;
            draw(&mut d, s)?;
            for (a, b) in d.layers[0].pixels.iter().zip(&full.layers[0].pixels) {
                let target = expected(base, *b, coverage);
                if (0..4).any(|n| a[n].abs_diff(target[n]) > 1) {
                    return Err(format!(
                        "{mode:?} material selection strength {coverage}: {a:?}/{target:?}"
                    ));
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod material_tests {
    #[test]
    fn each_pigment_mode_zero_half_full_is_applied_once() {
        super::verify_material_strength().unwrap();
    }
}
