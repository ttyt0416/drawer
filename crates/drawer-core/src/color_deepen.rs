//! Independent color-deepen controls share the exact correction-layer color pipeline.
use crate::{
    color_precision::{Depth, Sample},
    document::Document,
    layer_effects::{Adjustment, ColorModel},
    references::LocalImage,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub brightness: f64,
    pub saturation: f64,
    pub contrast: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 0.,
            saturation: 0.25,
            contrast: 0.25,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (-1. ..=1.).contains(&self.brightness)
            && (0. ..=1.).contains(&self.saturation)
            && (0. ..=1.).contains(&self.contrast)
    }
    pub fn identity(self) -> bool {
        self.brightness == 0. && self.saturation == 0. && self.contrast == 0.
    }
    pub fn base_adjustment(self) -> Adjustment {
        Adjustment {
            brightness: self.brightness,
            saturation: self.saturation,
            contrast: self.contrast * 0.5,
            color_model: ColorModel::Hsl,
            ..Default::default()
        }
    }
    pub fn adjustment(self) -> Adjustment {
        Adjustment {
            local: Some(crate::local_color::Correction::Deepen(self)),
            ..Default::default()
        }
    }
    pub fn clipped(self, rgb: [f64; 3]) -> bool {
        if self.identity() {
            return false;
        }
        let c = 1. / (1. - self.contrast * 0.5 * 0.99);
        let raw = rgb.map(|v| (v - 0.5) * c + 0.5 + self.brightness);
        if raw.iter().any(|v| *v < -1e-12 || *v > 1. + 1e-12) {
            return true;
        }
        let rgb = raw.map(|v| v.clamp(0., 1.));
        let max = rgb[0].max(rgb[1]).max(rgb[2]);
        let min = rgb[0].min(rgb[1]).min(rgb[2]);
        let delta = max - min;
        let scale = 1. - (max + min - 1.).abs();
        delta > 0. && scale > 0. && delta / scale * (1. + self.saturation) > 1. + 1e-12
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Destination {
    Pixels(u64),
    Layer,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub settings: Settings,
    pub destination: Destination,
    pub selection: bool,
}
impl Record {
    pub fn valid(&self) -> bool {
        self.settings.valid()
            && match self.destination {
                Destination::Pixels(id) => id > 0,
                Destination::Layer => true,
            }
    }
}
pub struct Prepared {
    pub state: DocumentState,
    pub warning: LocalImage,
    pub checked: usize,
    pub clipped: usize,
    pub sampled: bool,
}
fn weight(d: &Document, selection: bool, point: [f64; 2]) -> f64 {
    if selection {
        d.scene.selection_coverage(point) as f64 / 255.
    } else {
        1.
    }
}
pub fn prepare(
    d: &Document,
    destination: Destination,
    settings: Settings,
    selection: bool,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Prepared, String> {
    if !settings.valid()
        || !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || !d.color.valid_layers(&d.layers)
        || d.memory_bytes() > 32 * 1024 * 1024
    {
        return Err("Invalid color-deepen values/document or snapshot memory limit".into());
    }
    let layer = if let Destination::Pixels(id) = destination {
        let n = crate::layers::index(d, id).ok_or("Choose a live raster layer")?;
        crate::layer_properties::paintable(d, n)?;
        if d.layers[n].pixels.len() > 4 * 1024 * 1024 {
            return Err("Color-deepen source exceeds 4M pixels".into());
        }
        Some(n)
    } else {
        None
    };
    tick(0, 100)?;
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let value = settings.adjustment();
    let empty = selection
        && d.scene
            .selection_mask
            .as_ref()
            .is_some_and(|m| m.runs.is_empty());
    let mut state = d.snapshot();
    let (mut checked, mut clipped) = (0, 0);
    if !value.identity() && !empty {
        if let Some(n) = layer {
            let source = &d.layers[n];
            let extent = source.extent(d.width, d.height);
            let out = &mut state.layers[n];
            for i in 0..source.pixels.len() {
                if i.is_multiple_of(128) {
                    tick(i * 80 / source.pixels.len().max(1), 100)?;
                }
                let before = source.sample_at(i);
                if before.alpha() == 0. {
                    continue;
                }
                let pos = [
                    extent.offset[0] + (i % extent.width as usize) as i64,
                    extent.offset[1] + (i / extent.width as usize) as i64,
                ];
                let point = pos.map(|v| v as f64 + 0.5);
                let amount = weight(d, selection, point);
                if amount == 0. {
                    continue;
                }
                checked += 1;
                if settings.clipped(space.to_srgb(before)?.rgb()) {
                    clipped += 1;
                }
                let corrected = space.adjusted(before, value)?;
                let result = space.mix(
                    before,
                    corrected,
                    amount,
                    crate::color_mix::Space::Standard,
                    0.,
                )?;
                let mut stored = result.store(d.color.depth, d.color.dither, pos);
                stored[3] = if d.color.depth == Depth::Sixteen {
                    before.rgba16()[3]
                } else {
                    source.pixels[i][3] as u16
                };
                if d.color.depth == Depth::Sixteen {
                    if stored == before.rgba16() {
                        continue;
                    }
                    if out.precise.is_none() {
                        out.precise = Some(
                            (0..source.pixels.len())
                                .map(|i| source.sample_at(i).rgba16())
                                .collect(),
                        );
                    }
                    out.precise.as_mut().unwrap()[i] = stored;
                    out.pixels[i] = Sample::from16(stored).rgba8();
                } else {
                    out.pixels[i] = stored.map(|v| v as u8);
                }
            }
        } else {
            let result = crate::adjustments::prepare(
                d,
                crate::adjustments::Destination::New {
                    name: "Color deepen".into(),
                    parent: None,
                },
                value,
            )?;
            state = result.state;
            if selection && (d.scene.selection.is_some() || d.scene.selection_mask.is_some()) {
                let after = crate::layer_merge::preview(&state, d.current_frame)?;
                state = crate::layer_masks::prepare(
                    &after,
                    crate::layer_properties::Target::Raster(result.layer),
                    crate::layer_masks::Command::Selection {
                        hide_inside: false,
                        policy: crate::layer_masks::Policy::Create,
                    },
                )?
                .state;
            }
        }
    }
    if state != d.snapshot() {
        state.scene.last_color_deepen = Some(Box::new(Record {
            settings,
            destination,
            selection,
        }));
    }
    tick(80, 100)?;
    crate::layer_groups::finish(d, &state)?;
    let renderer = if layer.is_none() {
        Some(crate::float_render::Renderer::with_effect_progress(
            d,
            None,
            &mut |_, _| tick(80, 100),
        )?)
    } else {
        None
    };
    let nodes = d.scene.paper.root_nodes(d.layers.len());
    let size = [d.width.min(64) as usize, d.height.min(64) as usize];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        tick(80 + y * 19 / size[1].max(1), 100)?;
        for x in 0..size[0] {
            let point = [
                (x as f64 + 0.5) * d.width as f64 / size[0] as f64,
                (y as f64 + 0.5) * d.height as f64 / size[1] as f64,
            ];
            let before = if let Some(n) = layer {
                d.layers[n].sample(
                    d.width,
                    d.height,
                    point[0].floor() as i64,
                    point[1].floor() as i64,
                )
            } else {
                renderer
                    .as_ref()
                    .unwrap()
                    .nodes_sample(&nodes, point.map(|v| v.floor() as i64))?
            };
            let display = space.to_srgb(before)?;
            let eligible = before.alpha() > 0. && weight(d, selection, point) > 0.;
            let limited = eligible && !empty && settings.clipped(display.rgb());
            if layer.is_none() && eligible {
                checked += 1;
                if limited {
                    clipped += 1;
                }
            }
            pixels.push(if limited {
                [255, 32, 32, 255]
            } else {
                display.rgba8()
            });
        }
    }
    tick(100, 100)?;
    Ok(Prepared {
        state,
        warning: LocalImage {
            width: size[0] as u32,
            height: size[1] as u32,
            pixels,
        },
        checked,
        clipped,
        sampled: layer.is_none(),
    })
}
