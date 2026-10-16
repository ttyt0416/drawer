//! Tool-specific pixel writing, alpha retention and watercolor supply boundaries.
use crate::{
    assets::BrushTip,
    brush_material::{Mode, Settings},
    document::{Background, CanvasSpec, Document, PlacedRaster},
    painting::{Stroke, Style},
    selection::{Mask, Run},
};
fn style(tip: BrushTip) -> Style {
    Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: crate::tip_profile::Settings {
            pixel: crate::tip_profile::Pixel {
                pressure: false,
                overwrite: tip == BrushTip::PixelPen,
                ..Default::default()
            },
            ..Default::default()
        },
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 1.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip,
        color: [220, 70, 30, 64],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.25,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
fn document(bg: [u8; 4]) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 4,
            dpi: 300.0,
            background: Background::Transparent,
        },
        16384,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill(bg);
    Ok(d)
}
fn select(d: &mut Document, alpha: u8) {
    d.scene.selection_mask = Some(Mask {
        runs: if alpha == 0 {
            vec![]
        } else {
            vec![Run {
                x: 1,
                y: 1,
                length: 1,
                alpha,
            }]
        },
    });
}
fn dab(
    d: &mut Document,
    s: Style,
    pressure: f64,
    passes: usize,
    point: [f64; 2],
) -> Result<bool, String> {
    let before = d.snapshot();
    let count = d.history_counts().0;
    let mut stroke = Stroke::begin(d, 0, s)?;
    for _ in 0..passes {
        stroke.direct_dab(d, point, point, pressure)?;
    }
    if d.snapshot() != before {
        return Err("Tool-mode preview changed original".into());
    }
    let changed = stroke.commit_mouse(
        d,
        crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: s,
            count: passes as u32,
            ends: Default::default(),
            native_pressure: true,
        },
    )?;
    let after = d.snapshot();
    if changed
        && (!d.undo()
            || d.snapshot() != before
            || !d.redo()
            || d.snapshot() != after
            || d.history_counts().0 != count + 1)
    {
        return Err("Tool-mode stroke history diverged".into());
    }
    Ok(changed)
}
/// Shared numerical/pixel checks for tests and installed executable verification.
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for alpha in [0, 128, 255] {
        for mask in [0, 128, 255] {
            for repeats in [1, 2, 10] {
                let bg = [20, 40, 80, alpha];
                let mut d = document(bg)?;
                select(&mut d, mask);
                let mut s = style(BrushTip::PixelPen);
                s.opacity = 0.5;
                let expected = if mask == 0 {
                    bg
                } else {
                    [
                        220,
                        70,
                        30,
                        (64.0 * 0.5 * mask as f64 / 255.0).round() as u8,
                    ]
                };
                for _ in 0..repeats {
                    dab(&mut d, s, 1.0, repeats, [1.5; 2])?;
                    if d.layers[0].pixels[5] != expected {
                        return Err("Pixel overwrite accumulated or kept destination alpha".into());
                    }
                }
                if d.layers[0]
                    .pixels
                    .iter()
                    .enumerate()
                    .any(|(n, p)| n != 5 && *p != bg)
                    || d.history_counts().0 != usize::from(mask > 0)
                {
                    return Err(
                        "Pixel overwrite escaped selection or created empty histories".into(),
                    );
                }
            }
        }
    }
    for (alpha, expected) in [
        (0, [220, 70, 30, 64]),
        (128, [100, 52, 60, 160]),
        (255, [70, 48, 67, 255]),
    ] {
        let mut d = document([20, 40, 80, alpha])?;
        let s = style(BrushTip::Pen);
        dab(&mut d, s, 1.0, 1, [1.5; 2])?;
        if d.layers[0].pixels[5] != expected {
            return Err("Normal brush no longer uses source-over".into());
        }
    }
    for (repeats, expected_alpha) in [(1, 64), (2, 112), (10, 241)] {
        let mut d = document([0; 4])?;
        for _ in 0..repeats {
            dab(&mut d, style(BrushTip::Pen), 1.0, 1, [1.5; 2])?;
        }
        if d.layers[0].pixels[5][3] != expected_alpha {
            return Err(
                "Normal stroke accumulation differs from independent alpha recurrence".into(),
            );
        }
    }
    for (passes, expected_alpha) in [(1, 64), (2, 112), (10, 241)] {
        let mut single = document([0; 4])?;
        dab(&mut single, style(BrushTip::Pen), 1.0, passes, [1.5; 2])?;
        if single.layers[0].pixels[5][3] != expected_alpha || single.history_counts() != (1, 0) {
            return Err("Normal repeated dabs did not accumulate inside one stroke".into());
        }
    }
    let mut d = document([20, 40, 80, 255])?;
    let mut s = style(BrushTip::PixelPen);
    s.color = [220, 70, 30, 0];
    dab(&mut d, s, 1.0, 1, [1.5; 2])?;
    if d.layers[0].pixels[5] != s.color {
        return Err(
            "Transparent pixel overwrite lost hidden RGB or failed to replace alpha".into(),
        );
    }
    let state = d.snapshot();
    crate::files::save(&mut d, &root.join("pixel-overwrite.drawer")).map_err(|e| e.to_string())?;
    if crate::files::open(&root.join("pixel-overwrite.drawer"), 16384)
        .map_err(|e| e.to_string())?
        .snapshot()
        != state
    {
        return Err("Pixel overwrite profile/result lost on native reopen".into());
    }
    for alpha in [0, 128, 255] {
        for mask in [0, 128, 255] {
            for repeats in [1, 2, 10] {
                let bg = [20, 40, 80, alpha];
                let mut d = document(bg)?;
                select(&mut d, mask);
                let before = d.layers[0].pixels.clone();
                let alphas = before.iter().map(|p| p[3]).collect::<Vec<_>>();
                let mut s = style(BrushTip::Marker);
                s.material.alpha_lock = true;
                s.opacity = 0.5;
                for _ in 0..repeats {
                    dab(&mut d, s, 1.0, 2, [1.5; 2])?;
                }
                if d.layers[0].pixels.iter().map(|p| p[3]).collect::<Vec<_>>() != alphas
                    || ((alpha == 0 || mask == 0) && d.layers[0].pixels != before)
                {
                    return Err(
                        "Marker alpha lock changed an alpha or an empty/masked pixel".into(),
                    );
                }
                if alpha > 0 && mask > 0 && d.layers[0].pixels[5] == before[5] {
                    return Err("Marker alpha lock prevented color mixing".into());
                }
            }
        }
    }
    let mut water = style(BrushTip::Pen);
    water.color = [0, 255, 0, 255];
    water.material = Settings {
        mode: Mode::Water,
        supply: 1.0,
        mixing: 1.0,
        retention: 0.0,
        smudge_distance: 0.0,
        water: 0.0,
        ..Default::default()
    };
    for (p, expected) in [
        (0.1999, [0, 0, 255, 255]),
        (0.2, [0, 255, 0, 255]),
        (0.2001, [0, 255, 0, 255]),
    ] {
        let mut d = document([0, 0, 255, 255])?;
        dab(&mut d, water, p, 1, [1.5; 2])?;
        if d.layers[0].pixels[5] != expected {
            return Err("Watercolor supply threshold boundary is not strict-less-than".into());
        }
    }
    for colorless in [false, true] {
        let mut d = document([0; 4])?;
        water.material.colorless = colorless;
        let p = if colorless { 1.0 } else { 0.1999 };
        if dab(&mut d, water, p, 1, [1.5; 2])? || d.layers[0].pixels.iter().any(|p| *p != [0; 4]) {
            return Err(
                "Colorless/low-pressure smudge supplied foreground into empty pixels".into(),
            );
        }
    }
    // Canvas/storage clipping preserves all off-canvas stored values.
    let mut clipped = document([20, 40, 80, 255])?;
    clipped.layers[0].pixels = vec![[20, 40, 80, 255]; 4];
    clipped.layers[0].placement = Some(PlacedRaster {
        width: 2,
        height: 2,
        offset: [-1, -1],
    });
    let before = clipped.layers[0].pixels.clone();
    let mut s = style(BrushTip::PixelPen);
    s.shape.size = 4.0;
    dab(&mut clipped, s, 1.0, 1, [0.5; 2])?;
    if clipped.layers[0].pixels[..3] != before[..3] || clipped.layers[0].pixels[3] == before[3] {
        return Err("Pixel overwrite escaped canvas/layer storage clipping".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_modes_repeats_alpha_masks_water_boundaries_and_native() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn overwrite_bypasses_pigment_and_special_composition_keeps_its_rule() {
        let mut s = style(BrushTip::PixelPen);
        s.material = Settings {
            mode: Mode::Water,
            colorless: true,
            supply: 0.0,
            water: 1.0,
            ..Default::default()
        };
        let mut d = document([20, 40, 80, 255]).unwrap();
        dab(&mut d, s, 1.0, 10, [1.5; 2]).unwrap();
        assert_eq!(d.layers[0].pixels[5], s.color);
        s.color[3] = 255;
        s.material.composition.mode = crate::brush_composite::Mode::Erase;
        let mut d = document([20, 40, 80, 255]).unwrap();
        dab(&mut d, s, 1.0, 1, [1.5; 2]).unwrap();
        assert_eq!(d.layers[0].pixels[5], [20, 40, 80, 0]);
    }
    #[test]
    fn legacy_supply_and_pixel_mode_defaults_and_invalid_threshold_preserve_state() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("supply_threshold");
        let old: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(old.supply_threshold, 0.0);
        assert_eq!(Settings::default().supply_threshold, 0.2);
        let pixel: crate::tip_profile::Pixel =
            serde_json::from_str(r#"{"pressure":true,"grid":true}"#).unwrap();
        assert!(!pixel.overwrite);
        let d = document([0; 4]).unwrap();
        let before = d.snapshot();
        for threshold in [-0.1, 1.1, f64::NAN] {
            let mut s = style(BrushTip::Pen);
            s.material.supply_threshold = threshold;
            assert!(!s.valid());
            assert!(Stroke::begin(&d, 0, s).is_err());
            assert!(d.snapshot() == before);
        }
    }
}
