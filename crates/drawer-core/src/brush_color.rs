//! Independent numerical and pixel checks for the owned two-ink brush contract.
use crate::{
    assets::BrushTip,
    brush_variation::{Blend, Controls, Dual},
    document::{Background, CanvasSpec, Document},
    painting::{Stroke, Style},
};
fn document() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 32,
            dpi: 300.0,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())
}
fn style() -> Style {
    Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [255, 0, 0, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
fn paint(s: Style) -> Result<Document, String> {
    let mut d = document()?;
    let mut st = Stroke::begin(&d, 0, s)?;
    st.direct_dab(&d, [32.5, 16.5], [32.5, 16.5], 1.0)?;
    st.commit(&mut d)?;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let magenta = [255, 0, 255, 255];
    let red = [255, 0, 0, 255];
    let black = [0, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    let cases = [
        (Blend::Normal, blue, 0.8),
        (Blend::Multiply, black, 0.16),
        (Blend::Add, magenta, 1.0),
        (Blend::Subtract, [0; 4], 0.0),
        (Blend::Minimum, black, 0.2),
        (Blend::Maximum, magenta, 0.8),
        (Blend::Screen, magenta, 0.84),
        (Blend::Overlay, red, 0.32),
        (Blend::ColorDodge, red, 1.0),
        (Blend::ColorBurn, red, 0.0),
        (Blend::LinearBurn, black, 0.0),
        (Blend::HardMix, magenta, 1.0),
        (Blend::HeightLinear, [128, 0, 128, 255], 0.5),
        (Blend::ApplyRgb, [236, 0, 19, 19], 0.8),
        (Blend::Difference, [0; 4], 0.6),
    ];
    for (mode, expected, channel) in cases {
        if (mode.channel(0.2, 0.8) - channel).abs() > 1e-12 {
            return Err(format!("Dual channel reference {mode:?}"));
        }
        for rgb in [false, true] {
            let mut s = style();
            s.variation.dual = Dual {
                enabled: true,
                blend: mode,
                mask: crate::brush_variation::Mask::Square,
                scale: 1.0,
                controls: Controls {
                    rgb,
                    color: blue,
                    ..Default::default()
                },
                ..Default::default()
            };
            let d = paint(s)?;
            let actual = d.layers[0].pixels[16 * 64 + 32];
            let expected = if rgb || expected[3] == 0 {
                expected
            } else {
                [255, 0, 0, expected[3]]
            };
            if actual != expected {
                return Err(format!(
                    "Dual actual pixels {mode:?}, rgb={rgb}: {actual:?} != {expected:?}"
                ));
            }
        }
    }
    let corners = [
        (Blend::Normal, [0.0, 1.0, 0.0, 1.0]),
        (Blend::Multiply, [0.0, 0.0, 0.0, 1.0]),
        (Blend::Add, [0.0, 1.0, 1.0, 1.0]),
        (Blend::Subtract, [0.0, 0.0, 1.0, 0.0]),
        (Blend::Minimum, [0.0, 0.0, 0.0, 1.0]),
        (Blend::Maximum, [0.0, 1.0, 1.0, 1.0]),
        (Blend::Screen, [0.0, 1.0, 1.0, 1.0]),
        (Blend::Overlay, [0.0, 0.0, 1.0, 1.0]),
        (Blend::ColorDodge, [0.0, 0.0, 1.0, 1.0]),
        (Blend::ColorBurn, [0.0, 0.0, 1.0, 1.0]),
        (Blend::LinearBurn, [0.0, 0.0, 0.0, 1.0]),
        (Blend::HardMix, [0.0, 1.0, 1.0, 1.0]),
        (Blend::HeightLinear, [0.0, 0.5, 0.5, 1.0]),
        (Blend::ApplyRgb, [0.0, 1.0, 0.0, 1.0]),
        (Blend::Difference, [0.0, 1.0, 1.0, 0.0]),
    ];
    for (mode, expected) in corners {
        for ((a, b), value) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)]
            .into_iter()
            .zip(expected)
        {
            if mode.channel(a, b) != value {
                return Err(format!("Dual black/white {mode:?}"));
            }
        }
    }
    let mut s = style();
    s.color[3] = 128;
    s.variation.dual = Dual {
        enabled: true,
        blend: Blend::Normal,
        scale: 1.0,
        controls: Controls {
            rgb: true,
            color: [0, 0, 255, 64],
            ..Default::default()
        },
        ..Default::default()
    };
    if paint(s)?.layers[0].pixels[16 * 64 + 32] != [153, 0, 102, 160] {
        return Err("Independent ink alpha / premultiplied RGB mismatch".into());
    }
    s.color = [200, 40, 90, 0];
    if paint(s)?.layers[0].pixels[16 * 64 + 32] != [0, 0, 255, 64] {
        return Err("Hidden primary RGB polluted secondary ink".into());
    }
    s.variation.dual.controls.color[3] = 0;
    if paint(s)?.layers[0].pixels.iter().any(|p| p[3] != 0) {
        return Err("Two transparent inks manufactured pigment".into());
    }
    let mut widths = vec![];
    for (size, linked) in [(8.0, true), (16.0, true), (8.0, false), (16.0, false)] {
        let mut s = style();
        s.shape.size = size;
        s.variation.dual = Dual {
            enabled: true,
            blend: Blend::Multiply,
            mask: crate::brush_variation::Mask::Square,
            scale: 0.5,
            controls: Controls {
                linked_size: linked,
                size: 6.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let d = paint(s)?;
        widths.push(
            (0..64)
                .filter(|x| d.layers[0].pixels[16 * 64 + *x][3] > 0)
                .count(),
        );
    }
    if widths[1] <= widths[0] || widths[2] != widths[3] {
        return Err(format!("Linked/independent actual diameters: {widths:?}"));
    }
    let mut d = document()?;
    let before = d.snapshot();
    let mut s = style();
    s.variation.dual = Dual {
        enabled: true,
        blend: Blend::Normal,
        controls: Controls {
            rgb: true,
            color: blue,
            ..Default::default()
        },
        ..Default::default()
    };
    s.material.space = crate::color_mix::Space::LabD65;
    s.material.brightness = 0.6;
    let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, s, Default::default())?;
    replay.push(crate::mouse_pressure::Sample {
        point: [32.5, 16.5],
        micros: Some(0),
    })?;
    replay
        .render(&d, true)?
        .commit_mouse(&mut d, replay.metadata())?;
    let after = d.snapshot();
    if d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Dual stroke history was split or reevaluated".into());
    }
    crate::files::save(&mut d, &root.join("dual-color.drawer")).map_err(|e| e.to_string())?;
    let restored =
        crate::files::open(&root.join("dual-color.drawer"), 65536).map_err(|e| e.to_string())?;
    if restored.snapshot() != d.snapshot() || restored.scene.mouse_strokes[0].style != s {
        return Err("Dual RGB/Lab settings or pixels lost in native file".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn dual_modes_numeric_alpha_size_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
    #[test]
    fn legacy_controls_and_invalid_values() {
        use crate::brush_variation::{Controls, Dual};
        let mut value = serde_json::to_value(Dual::default()).unwrap();
        value.as_object_mut().unwrap().remove("controls");
        assert_eq!(
            serde_json::from_value::<Dual>(value).unwrap().controls,
            Controls::default()
        );
        for size in [0.0, 4096.1, f64::NAN] {
            assert!(
                !Dual {
                    controls: Controls {
                        size,
                        ..Default::default()
                    },
                    ..Default::default()
                }
                .valid()
            );
        }
        for brightness in [-0.1, 1.1, f64::NAN] {
            assert!(
                !crate::brush_material::Settings {
                    brightness,
                    ..Default::default()
                }
                .valid()
            );
        }
    }
}
