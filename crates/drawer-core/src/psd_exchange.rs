//! Bounded PSD/PSB interchange from Adobe's public specification, without private application data.
use crate::{
    document::{Background, CanvasSpec, Document, PlacedRaster, RasterLayer},
    layer_blend::Mode,
    layer_properties::{self as props, Target},
    paper::{Folder, Node},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Psd,
    Psb,
}
#[derive(Clone, Debug)]
pub struct Mask {
    pub bounds: [i32; 4],
    pub default: u8,
    pub flags: u8,
    pub pixels: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub bounds: [i32; 4],
    pub pixels: Vec<[u8; 4]>,
    pub mask: Option<Mask>,
    pub blend: [u8; 4],
    pub opacity: u8,
    pub hidden: bool,
    pub clipped: bool,
    pub alpha_lock: bool,
    pub section: u32,
}
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub layers: Vec<Layer>,
    pub merged: Vec<[u8; 4]>,
}
pub struct Plan {
    pub partial: Option<Image>,
    pub flattened: Image,
    pub losses: Vec<String>,
    pub ranges: Vec<Vec<String>>,
    pub whole_names: Vec<String>,
    pub reason: Option<String>,
    pub delta: u8,
    source: u64,
    revision: u64,
    frame: u32,
}
impl Plan {
    pub fn check(&self, d: &Document) -> Result<(), String> {
        if (self.source, self.revision, self.frame) != (d.id, d.revision, d.current_frame) {
            Err("Document / frame changed; prepare PSD exchange again".into())
        } else {
            Ok(())
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        d: &Document,
        path: &Path,
        format: Format,
        flatten: bool,
        confirmed: bool,
        replace: bool,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.write_with_preview(
            d, path, format, flatten, true, confirmed, replace, budget, cancel,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write_with_preview(
        &self,
        d: &Document,
        path: &Path,
        format: Format,
        flatten: bool,
        merged_preview: bool,
        confirmed: bool,
        replace: bool,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.check(d)?;
        if !confirmed {
            return Err("Review and confirm the conversion/loss list before exporting".into());
        }
        let image = if flatten {
            &self.flattened
        } else {
            self.partial.as_ref().ok_or(
                "Partial conversion could not preserve appearance; choose the whole composite",
            )?
        };
        io::write(
            image,
            d,
            path,
            format,
            merged_preview,
            replace,
            budget,
            cancel,
        )
    }
}
pub(crate) fn canceled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("PSD exchange canceled; source and destination unchanged".into())
    } else {
        Ok(())
    }
}
pub(crate) fn area(b: [i32; 4]) -> Result<(usize, usize), String> {
    let w = b[3] as i64 - b[1] as i64;
    let h = b[2] as i64 - b[0] as i64;
    if !(0..=32768).contains(&w)
        || !(0..=32768).contains(&h)
        || w.saturating_mul(h) > 4 * 1024 * 1024
    {
        return Err("PSD layer/mask extent exceeds work limits".into());
    }
    Ok((w as usize, h as usize))
}
pub(crate) fn blend_key(m: Mode) -> Option<[u8; 4]> {
    Some(*match m {
        Mode::Normal => b"norm",
        Mode::Darken => b"dark",
        Mode::Multiply => b"mul ",
        Mode::ColorBurn => b"idiv",
        Mode::LinearBurn => b"lbrn",
        Mode::Subtract => b"fsub",
        Mode::DarkerColor => b"dkCl",
        Mode::Lighten => b"lite",
        Mode::Screen => b"scrn",
        Mode::ColorDodge => b"div ",
        Mode::Add => b"lddg",
        Mode::LighterColor => b"lgCl",
        Mode::Overlay => b"over",
        Mode::HardLight => b"hLit",
        Mode::VividLight => b"vLit",
        Mode::LinearLight => b"lLit",
        Mode::PinLight => b"pLit",
        Mode::HardMix => b"hMix",
        Mode::Difference => b"diff",
        Mode::Exclusion => b"smud",
        Mode::Hue => b"hue ",
        Mode::Saturation => b"sat ",
        Mode::Color => b"colr",
        Mode::Brightness => b"lum ",
        Mode::Divide => b"fdiv",
        _ => return None,
    })
}
fn blend_mode(k: [u8; 4]) -> Result<Mode, String> {
    Mode::ALL
        .into_iter()
        .find(|m| blend_key(*m) == Some(k))
        .ok_or_else(|| format!("Unsupported PSD blend {}", String::from_utf8_lossy(&k)))
}
pub(crate) fn render(
    d: &Document,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<[u8; 4]>, String> {
    let count = d.width as usize * d.height as usize;
    if count > 4 * 1024 * 1024 || count.saturating_mul(32) > budget {
        return Err("PSD preview exceeds pixel/memory limit".into());
    }
    let r = crate::render::Renderer::for_export(d)?;
    let mut pixels = Vec::with_capacity(count);
    for y in 0..d.height {
        canceled(cancel)?;
        for x in 0..d.width {
            pixels.push(r.display_sample([x as f64 + 0.5, y as f64 + 0.5])?.rgba8());
        }
    }
    Ok(pixels)
}
pub fn difference(a: &[[u8; 4]], b: &[[u8; 4]]) -> u8 {
    if a.len() != b.len() {
        return 255;
    }
    a.iter()
        .zip(b)
        .flat_map(|(a, b)| {
            let a = crate::color_precision::Sample::from8(*a);
            let b = crate::color_precision::Sample::from8(*b);
            let aa = a.rgba16();
            let bb = b.rgba16();
            std::array::from_fn::<_, 4, _>(|n| {
                if n == 3 {
                    (aa[n].abs_diff(bb[n]) as u32 / 257) as u8
                } else {
                    let ac = aa[n] as u32 * aa[3] as u32 / 65535;
                    let bc = bb[n] as u32 * bb[3] as u32 / 65535;
                    (ac.abs_diff(bc) / 257) as u8
                }
            })
        })
        .max()
        .unwrap_or(0)
}
impl Image {
    pub fn document(&self, budget: usize) -> Result<Document, String> {
        if self.layers.len() > 768
            || self.merged.len() != self.width as usize * self.height as usize
        {
            return Err("Invalid PSD image dimensions or record count".into());
        }
        let mut d = Document::create(
            CanvasSpec {
                width: self.width,
                height: self.height,
                dpi: self.dpi,
                background: Background::Transparent,
            },
            budget,
        )
        .map_err(|e| e.to_string())?;
        d.layers.clear();
        let mut stack: Vec<usize> = vec![];
        let mut clipping: Vec<(Target, bool)> = vec![];
        for layer in self.layers.iter().rev() {
            if layer.section == 3 {
                let n = d.scene.paper.folders.len();
                if n >= 256 || stack.len() >= 16 {
                    return Err("PSD folder nesting/count limit".into());
                }
                d.scene.paper.folders.push(Folder {
                    name: "Group".into(),
                    through: false,
                    children: vec![],
                    paper: Default::default(),
                });
                attach(&mut d, &stack, Node::Folder(n));
                stack.push(n);
                continue;
            }
            let t = if matches!(layer.section, 1 | 2) {
                let n = stack.pop().ok_or("Unbalanced PSD group end")?;
                d.scene.paper.folders[n].name = layer.name.clone();
                d.scene.paper.folders[n].through = layer.blend == *b"pass";
                d.scene.paper.folders[n].paper.hidden = layer.hidden;
                Target::Folder(n as u16)
            } else {
                let (w, h) = area(layer.bounds)?;
                if w == 0 || h == 0 || layer.pixels.len() != w * h {
                    return Err(format!(
                        "{}: no usable cached raster; a merged substitute is required",
                        layer.name
                    ));
                }
                if d.layers.len() >= 256 {
                    return Err("PSD raster layer limit (256)".into());
                }
                let n = d.layers.len();
                d.layers.push(RasterLayer {
                    tiled: None,
                    precise: None,
                    locked: false,
                    name: layer.name.clone(),
                    pixels: layer.pixels.clone(),
                    placement: Some(PlacedRaster {
                        width: w as u32,
                        height: h as u32,
                        offset: [layer.bounds[1] as i64, layer.bounds[0] as i64],
                    }),
                });
                attach(&mut d, &stack, Node::Raster(n));
                d.scene.paper.layers.entry(n).or_default().hidden = layer.hidden;
                Target::Raster(n as u64 + 1)
            };
            let mode = if layer.blend == *b"pass" {
                Mode::Normal
            } else {
                blend_mode(layer.blend)?
            };
            d.scene.layer_properties.set(
                t,
                props::Properties {
                    opacity: ((layer.opacity as u32 * 100 + 127) / 255) as u8,
                    blend: mode,
                    ..Default::default()
                },
            );
            if layer.alpha_lock {
                d.scene.clipping.alpha.insert(t);
            }
            clipping.push((t, layer.clipped));
            if let Some(mask) = &layer.mask {
                let (w, h) = area(mask.bounds)?;
                if mask.pixels.len() != w * h || mask.flags & !7 != 0 {
                    return Err("Unsupported PSD mask parameters / raster size".into());
                }
                let mut runs = vec![];
                for y in 0..self.height {
                    let mut x = 0;
                    while x < self.width {
                        let sample = |x: u32| {
                            let xx = x as i64 - mask.bounds[1] as i64;
                            let yy = y as i64 - mask.bounds[0] as i64;
                            if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                                mask.pixels[yy as usize * w + xx as usize]
                            } else {
                                mask.default
                            }
                        };
                        let a = sample(x);
                        let start = x;
                        x += 1;
                        while x < self.width && sample(x) == a {
                            x += 1;
                        }
                        if a > 0 {
                            runs.push(crate::selection::Run {
                                x: start as i32,
                                y: y as i32,
                                length: x - start,
                                alpha: a,
                            });
                            if runs.len() > 32768 {
                                return Err("PSD mask run limit; use merged substitute".into());
                            }
                        }
                    }
                }
                let m = crate::layer_effects::Effect {
                    mask: Some(crate::layer_effects::Mask {
                        space: Default::default(),
                        coverage: crate::selection::Mask { runs },
                        offset: [0.; 2],
                        enabled: mask.flags & 2 == 0,
                        linked: true,
                        inverted: mask.flags & 4 != 0,
                    }),
                    adjustment: None,
                };
                match t {
                    Target::Raster(id) => {
                        d.scene.layer_effects.rasters.insert(id, m);
                    }
                    Target::Folder(n) => {
                        d.scene.layer_effects.folders.insert(n, m);
                    }
                    _ => {}
                }
            }
        }
        if !stack.is_empty() || d.layers.is_empty() {
            return Err("Unbalanced PSD groups or no raster content".into());
        }
        d.scene.layer_ids.initialize(d.layers.len())?;
        fn bind(d: &mut Document, list: &[Node], flags: &[(Target, bool)]) -> Result<(), String> {
            let mut base = None;
            for node in list {
                let t = crate::clipping::node_key(d, *node);
                if flags.iter().any(|(id, c)| *id == t && *c) {
                    let b = base.ok_or("Clipped PSD layer has no sibling base")?;
                    match t {
                        Target::Raster(id) => {
                            d.scene.clipping.rasters.insert(id, b);
                        }
                        Target::Folder(n) => {
                            d.scene.clipping.folders.insert(n, b);
                        }
                        _ => {}
                    }
                } else {
                    base = Some(t);
                }
                if let Node::Folder(n) = node {
                    bind(d, &d.scene.paper.folders[*n].children.clone(), flags)?;
                }
            }
            Ok(())
        }
        let roots = d.scene.paper.roots.clone();
        bind(&mut d, &roots, &clipping)?;
        if !d.layers.iter().all(|l| l.valid(d.width, d.height))
            || !d.color.valid_layers(&d.layers)
            || !d.scene.valid()
            || !d.scene.paper.valid_for(d.layers.len())
            || d.memory_bytes() > budget
        {
            return Err("Imported PSD structure or memory limit".into());
        }
        Ok(d)
    }
}
fn attach(d: &mut Document, stack: &[usize], node: Node) {
    if let Some(n) = stack.last() {
        d.scene.paper.folders[*n].children.push(node);
    } else {
        d.scene.paper.roots.push(node);
    }
}
fn capture(d: &Document, merged: Vec<[u8; 4]>, cancel: &AtomicBool) -> Result<Image, String> {
    let mut image = Image {
        width: d.width,
        height: d.height,
        dpi: d.dpi,
        layers: vec![],
        merged,
    };
    fn visit(
        d: &Document,
        node: Node,
        out: &mut Vec<Layer>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        canceled(cancel)?;
        let t = crate::clipping::node_key(d, node);
        let p = props::properties(d, t);
        let through = matches!(node,Node::Folder(n) if d.scene.paper.folders[n].through);
        let mut layer = Layer {
            name: match node {
                Node::Raster(n) => d.layers[n].name.clone(),
                Node::Folder(n) => d.scene.paper.folders[n].name.clone(),
            },
            bounds: [0; 4],
            pixels: vec![],
            mask: None,
            blend: if through {
                *b"pass"
            } else {
                blend_key(p.blend).ok_or("Unmapped blend requires backdrop rasterization")?
            },
            opacity: ((p.opacity as u32 * 255 + 50) / 100) as u8,
            hidden: p.hidden,
            clipped: crate::clipping::active(d, t),
            alpha_lock: d.scene.clipping.alpha.contains(&t),
            section: 0,
        };
        if let Some(mask) = crate::layer_masks::get(d, t) {
            let mut pixels = Vec::with_capacity(d.width as usize * d.height as usize);
            for y in 0..d.height {
                canceled(cancel)?;
                for x in 0..d.width {
                    let point = mask.space.inverse().map([x as f64 + 0.5, y as f64 + 0.5]);
                    pixels.push(
                        mask.coverage
                            .coverage([point[0] - mask.offset[0], point[1] - mask.offset[1]]),
                    );
                }
            }
            layer.mask = Some(Mask {
                bounds: [0, 0, d.height as i32, d.width as i32],
                default: 0,
                flags: ((!mask.enabled as u8) << 1) | ((mask.inverted as u8) << 2),
                pixels,
            });
        }
        match node {
            Node::Raster(n) => {
                let raster = &d.layers[n];
                let e = raster.extent(d.width, d.height);
                layer.bounds = [
                    e.offset[1] as i32,
                    e.offset[0] as i32,
                    (e.offset[1] + e.height as i64) as i32,
                    (e.offset[0] + e.width as i64) as i32,
                ];
                let (w, h) = area(layer.bounds)?;
                layer.pixels.reserve(w * h);
                let space = crate::color_profile::Transform::new(
                    &d.color.profile,
                    &crate::color_profile::RgbProfile::Srgb,
                )?;
                for y in 0..h {
                    canceled(cancel)?;
                    for x in 0..w {
                        let point = [e.offset[0] + x as i64, e.offset[1] + y as i64];
                        let sample = raster.sample(d.width, d.height, point[0], point[1]);
                        layer.pixels.push(space.map(sample)?.rgba8());
                    }
                }
                out.push(layer);
            }
            Node::Folder(n) => {
                layer.section = 1;
                out.push(layer.clone());
                for child in d.scene.paper.folders[n].children.iter().rev() {
                    visit(d, *child, out, cancel)?;
                }
                layer.section = 3;
                layer.name = "</Group>".into();
                layer.mask = None;
                layer.blend = *b"norm";
                layer.opacity = 255;
                layer.hidden = false;
                layer.clipped = false;
                out.push(layer);
            }
        }
        Ok(())
    }
    let mut roots = d.scene.paper.root_nodes(d.layers.len());
    roots.retain(
        |n| !matches!(n,Node::Raster(i) if d.scene.render_order.deferred(d.scene.layer_ids.id(*i))),
    );
    for entry in crate::render_order::normalized(&d.scene) {
        if let crate::layer_search::Key::Layer(Target::Raster(id)) = entry.key
            && let Some(n) = crate::layers::index(d, id)
        {
            roots.push(Node::Raster(n));
        }
    }
    for node in roots.into_iter().rev() {
        visit(d, node, &mut image.layers, cancel)?;
    }
    Ok(image)
}
#[path = "psd_plan.rs"]
mod planner;
pub use planner::prepare;
#[path = "psd_exchange_io.rs"]
mod io;
pub use io::{open_detailed, open_report};
#[path = "psd_report.rs"]
pub mod report;

#[path = "psd_exchange_verify.rs"]
pub mod verify;
