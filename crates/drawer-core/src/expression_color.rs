//! Explicit sRGB expression thresholds, independent from raw ink and other effects.
use crate::{
    appearance::Style,
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    layer_search::Expression,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub brightness: u8,
    pub alpha: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 128,
            alpha: 128,
        }
    }
}
/// Linear sRGB luminance, encoded back to sRGB before the threshold comparison.
pub fn luminance(p: [u8; 4]) -> f64 {
    let decode = |v: u8| {
        let v = v as f64 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    if p[0] == p[1] && p[1] == p[2] {
        return p[0] as f64;
    }
    let y = 0.2126 * decode(p[0]) + 0.7152 * decode(p[1]) + 0.0722 * decode(p[2]);
    (if y <= 0.0031308 {
        12.92 * y
    } else {
        1.055 * y.powf(1. / 2.4) - 0.055
    }) * 255.
}
pub fn pixel(mut p: [u8; 4], mode: Expression, settings: Settings) -> [u8; 4] {
    if p[3] == 0 {
        return [0; 4];
    }
    match mode {
        Expression::Color => p,
        Expression::Gray => {
            let value = luminance(p).clamp(0., 255.).round() as u8;
            p[..3].fill(value);
            p
        }
        Expression::Mono => {
            if p[3] < settings.alpha {
                return [0; 4];
            }
            let v = if luminance(p) >= settings.brightness as f64 {
                255
            } else {
                0
            };
            [v, v, v, 255]
        }
    }
}
pub fn prepare_display(
    d: &Document,
    t: Target,
    mut style: Style,
    mode: Expression,
    settings: Settings,
) -> Result<DocumentState, String> {
    style.expression_settings = Some(settings);
    crate::appearance::prepare_expression(d, t, style, Some(mode))
}
/// Convert stored raster samples or vector stroke/fill colors. Geometry and masks stay editable.
pub fn prepare_pixels(
    d: &Document,
    t: Target,
    style: Style,
    mode: Expression,
    settings: Settings,
) -> Result<DocumentState, String> {
    let flags = p::inherited(d, t);
    if flags.hidden || flags.locked || flags.pixels_locked {
        return Err("Show and unlock the source ink before converting expression pixels".into());
    }
    let mut state = prepare_display(d, t, style, mode, settings)?;
    match t {
        Target::Raster(id) => {
            let n = crate::layers::index(d, id).ok_or("Missing raster")?;
            if state.layers[n].pixels.len() > 4 * 1024 * 1024 {
                return Err("Expression conversion exceeds 4Mi samples".into());
            }
            if d.color != crate::color_storage::Config::legacy() {
                let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
                crate::color_storage::map_layer(
                    &mut state.layers[n],
                    &d.color,
                    d.width,
                    d.height,
                    |p| {
                        space.from_srgb(space.to_srgb(p)?.expression(
                            mode,
                            settings.brightness as f64 / 255.,
                            settings.alpha as f64 / 255.,
                        )?)
                    },
                )?;
            } else {
                for p in &mut state.layers[n].pixels {
                    *p = pixel(*p, mode, settings);
                }
            }
        }
        Target::Vector(id) => {
            let l = state
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing vector")?;
            for path in &mut l.paths {
                path.color = pixel(path.color, mode, settings);
                if let Some(fill) = &mut path.fill {
                    *fill = pixel(*fill, mode, settings);
                }
            }
        }
        _ => return Err("Rasterize this layer before converting expression pixels".into()),
    }
    let key = crate::layer_catalog::key(d, t);
    let mut info = state.scene.catalog.info(key).cloned().unwrap_or_default();
    info.expression = Expression::Color;
    state.scene.catalog.set(key, info);
    let mut style = state.scene.appearance.get(t).cloned().unwrap_or_default();
    style.expression_settings = None;
    state.scene.appearance.set(t, style)?;
    if !state.scene.valid()
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Expression conversion exceeds snapshot limits".into());
    }
    crate::clipping::validate_state(d, &state)?;
    p::validate_edit(d, &state)?;
    Ok(state)
}
pub fn requires_v60(scene: &crate::scene::Scene) -> bool {
    scene
        .appearance
        .rasters
        .values()
        .chain(scene.appearance.vectors.values())
        .any(|s| s.expression_settings.is_some())
}
