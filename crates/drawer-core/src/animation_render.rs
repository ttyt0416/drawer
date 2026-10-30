//! Captured animation output geometry, include policy and coordinate-stable watermark.
use crate::{
    animation_export::{Format, Options, Snapshot},
    document::Document,
    scene::{Point, Rect},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Area {
    #[default]
    Canvas,
    OutputFrame,
    Overflow,
    Selection,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct View {
    pub area: Area,
    pub camera: bool,
    pub drafts: bool,
    pub frame_lines: bool,
    pub transparent: bool,
    pub shadows: bool,
    pub icc: bool,
    pub matte: [u8; 3],
    pub watermark_opacity: u8,
    pub watermark_origin: [i32; 2],
    pub noise: u8,
    pub seed: u64,
}
impl Default for View {
    fn default() -> Self {
        Self {
            area: Area::Canvas,
            camera: true,
            drafts: false,
            frame_lines: true,
            transparent: true,
            shadows: true,
            icc: true,
            matte: [255; 3],
            watermark_opacity: 128,
            watermark_origin: [0; 2],
            noise: 0,
            seed: 1,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub origin: [i64; 2],
    pub width: u32,
    pub height: u32,
}
impl Layout {
    pub fn pixels(self) -> usize {
        self.width as usize * self.height as usize
    }
}
#[derive(Clone)]
pub struct Watermark {
    pub hash: [u8; 32],
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}
impl Watermark {
    pub fn read(path: &Path, budget: usize) -> Result<Self, String> {
        if !crate::font_library::local(path)
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            return Err("Choose a local PNG watermark".into());
        }
        let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.file_type().is_symlink()
            || m.len() > 8 * 1024 * 1024
            || m.len() > budget as u64 / 16
        {
            return Err("Watermark must be a regular PNG within8MiB and the output budget".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() != m.len() as usize {
            return Err("Watermark changed while being captured".into());
        }
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        decoder.set_limits(png::Limits { bytes: budget / 16 });
        let mut r = decoder.read_info().map_err(|e| e.to_string())?;
        if r.info().animation_control.is_some() {
            return Err("Use a static PNG watermark".into());
        }
        let n = r
            .output_buffer_size()
            .filter(|n| *n <= 16 * 1024 * 1024 && *n <= budget / 16)
            .ok_or("Watermark decode budget")?;
        let mut data = vec![0; n];
        let out = r.next_frame(&mut data).map_err(|e| e.to_string())?;
        let channels = out.color_type.samples();
        let pixels = data[..out.buffer_size()]
            .chunks_exact(channels)
            .map(|p| match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            })
            .collect::<Vec<_>>();
        // Watermark PNG pixels are explicitly interpreted as sRGB; embedded profiles must agree.
        if let Some(icc) = &r.info().icc_profile {
            let space = crate::color_profile::WorkingSpace::new(
                &crate::color_profile::RgbProfile::Embedded {
                    name: "Watermark".into(),
                    bytes: icc.to_vec(),
                },
            )?;
            let pixels = pixels
                .into_iter()
                .map(|p| {
                    space
                        .to_srgb(crate::color_precision::Sample::from8(p))
                        .map(|s| s.rgba8())
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Self {
                hash: Sha256::digest(&bytes).into(),
                width: out.width,
                height: out.height,
                pixels,
            });
        }
        Ok(Self {
            hash: Sha256::digest(&bytes).into(),
            width: out.width,
            height: out.height,
            pixels,
        })
    }
}
pub(crate) fn apply(d: &mut Document, v: View, top: bool) -> Result<(), String> {
    d.scene.include_drafts_in_output = v.drafts;
    if top && !v.camera {
        d.scene.keyframes.camera_output = false;
    }
    if v.transparent
        && let Some(id) = d.scene.paper_background
    {
        let n = crate::layers::index(d, id).ok_or("Paper layer missing")?;
        d.scene.paper.layers.entry(n).or_default().hidden = true;
        d.scene
            .layer_properties
            .rasters
            .entry(id)
            .or_default()
            .hidden = true;
    }
    if !v.frame_lines {
        d.scene.comics.page.include_guides = false;
        for o in &mut d.scene.objects {
            if let crate::scene::PageObject::Comic(c) = o
                && let crate::comics::Kind::Frame(f) = &mut c.kind
            {
                f.ink[3] = 0;
            }
        }
    }
    if !v.shadows {
        for s in d.scene.surface_effects.rasters.values_mut() {
            for e in &mut s.effects {
                if matches!(
                    e.operation,
                    crate::surface_effects::Operation::Shadow { .. }
                ) {
                    e.enabled = false;
                }
            }
        }
    }
    for p in &mut d.scene.cels.definitions {
        p.output_policy(v)?;
    }
    for p in &mut d.scene.page_parts.entries {
        p.output_policy(v)?;
    }
    // Nested float render plans consume their export document directly. Resolve draft
    // visibility here too, rather than relying on the top-level PNG output wrapper.
    if let Some(hidden) = crate::layer_roles::output_view(d)? {
        hidden.snapshot().restore(d);
    }
    Ok(())
}
fn extend(lo: &mut Point, hi: &mut Point, points: impl IntoIterator<Item = Point>) {
    for p in points {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
}
pub(crate) fn layout(d: &Document, o: Options, budget: usize) -> Result<Layout, String> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    match o.view.area {
        Area::Canvas => extend(
            &mut lo,
            &mut hi,
            [[0.; 2], [d.width as f64, d.height as f64]],
        ),
        Area::Selection => {
            if d.scene.selection_empty() {
                return Err("Create a nonempty selection before selection output".into());
            }
            let mut points = Vec::new();
            if let Some(mask) = &d.scene.selection_mask {
                for r in &mask.runs {
                    points.extend(crate::transform_handles::corners(Rect {
                        min: [r.x as f64, r.y as f64],
                        max: [r.x as f64 + r.length as f64, r.y as f64 + 1.],
                    }));
                }
            } else if let Some(r) = d.scene.selection {
                points.extend(crate::transform_handles::corners(r));
            }
            extend(
                &mut lo,
                &mut hi,
                points.into_iter().map(|p| d.scene.selection_point(p)),
            );
        }
        Area::OutputFrame | Area::Overflow => {
            if o.view.area == Area::Overflow {
                extend(
                    &mut lo,
                    &mut hi,
                    [[0.; 2], [d.width as f64, d.height as f64]],
                );
            }
            let mut view = crate::layer_merge::preview(&d.snapshot(), o.first)?;
            for frame in o.first..=o.last {
                view.current_frame = frame;
                if o.view.area == Area::OutputFrame {
                    let f = view
                        .scene
                        .animation_output
                        .at(frame)
                        .ok_or("Set an animation output frame before frame output")?;
                    extend(&mut lo, &mut hi, f.corners(view.scene.transform));
                } else {
                    for row in crate::layer_catalog::rows(&view)? {
                        if !crate::layer_catalog::properties(&view, row.key).hidden
                            && let Some(r) = crate::layer_catalog::bounds(&view, row.key)
                        {
                            extend(&mut lo, &mut hi, crate::transform_handles::corners(r));
                        }
                    }
                    // Surface caches carry the actual blur/glow/shadow extent in page space.
                    for (id, stack) in &view.scene.surface_effects.rasters {
                        if stack.active() {
                            let n =
                                crate::layers::index(&view, *id).ok_or("Surface source missing")?;
                            let e = crate::surface_effects::extent(
                                view.layers[n].extent(view.width, view.height),
                                stack,
                            )?;
                            let r = Rect {
                                min: e.offset.map(|n| n as f64),
                                max: [
                                    e.offset[0] as f64 + e.width as f64,
                                    e.offset[1] as f64 + e.height as f64,
                                ],
                            };
                            let key = crate::layer_search::Key::Layer(
                                crate::layer_properties::Target::Raster(*id),
                            );
                            let m = crate::camera2d::display_matrix(&view, key, true).0;
                            extend(
                                &mut lo,
                                &mut hi,
                                crate::transform_handles::corners(r).map(|p| m.map(p)),
                            );
                        }
                    }
                }
            }
        }
    }
    lo = lo.map(f64::floor);
    hi = hi.map(f64::ceil);
    if lo
        .iter()
        .chain(&hi)
        .any(|x| !x.is_finite() || x.abs() > 1e9)
    {
        return Err("Invalid output bounds".into());
    }
    let size = [hi[0] - lo[0], hi[1] - lo[1]];
    if size.iter().any(|v| !(1. ..=32768.).contains(v)) || size[0] * size[1] * 32. > budget as f64 {
        return Err("Output geometry exceeds dimensions / working memory budget".into());
    }
    Ok(Layout {
        origin: lo.map(|v| v as i64),
        width: size[0] as u32,
        height: size[1] as u32,
    })
}
fn noise(seed: u64, x: u32, y: u32) -> u8 {
    let mut z = seed
        .wrapping_add((x as u64) << 32 | y as u64)
        .wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    (z ^ (z >> 31)) as u8
}
/// Full-resolution output pixels; callers use the same captured options for preview and files.
pub fn frame(s: &Snapshot, frame: u32, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if !(s.options.first..=s.options.last).contains(&frame) {
        return Err("Frame outside captured range".into());
    }
    let mut d = crate::layer_merge::preview(&s.document().snapshot(), frame)?;
    apply(&mut d, s.options.view, true)?;
    let owned = crate::layer_roles::output_view(&d)?;
    let d = owned.as_deref().unwrap_or(&d);
    let renderer = crate::float_render::Renderer::for_export(d)?;
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let preserve = s.options.view.icc && !matches!(s.options.format, Format::Gif | Format::Movie);
    let mut bytes = Vec::with_capacity(s.layout.pixels() * 4);
    for y in 0..s.layout.height {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..s.layout.width {
            let point = [
                s.layout.origin[0] as f64 + x as f64 + 0.5,
                s.layout.origin[1] as f64 + y as f64 + 0.5,
            ];
            let mut pixel = renderer.output_sample(point)?;
            let guide = d.scene.comics.page.output_pixel(d, point);
            if guide[3] != 0 {
                pixel = pixel.over(
                    space.from_srgb(crate::color_precision::Sample::from8(guide))?,
                    crate::layer_blend::Mode::Normal,
                    1.,
                )?;
            }
            if s.options.view.area == Area::Selection {
                pixel = pixel.coverage(d.scene.selection_coverage(point) as f64 / 255.)?;
            }
            if !preserve {
                pixel = space.to_srgb(pixel)?;
            }
            let mut p = pixel
                .store(
                    crate::color_precision::Depth::Eight,
                    d.color.dither,
                    [x as i64, y as i64],
                )
                .map(|v| v as u8);
            let overlay = |base: [u8; 4], p: [u8; 4]| -> Result<[u8; 4], String> {
                let top = crate::color_precision::Sample::from8(p);
                let top = if preserve { space.from_srgb(top)? } else { top };
                Ok(crate::color_precision::Sample::from8(base)
                    .over(top, crate::layer_blend::Mode::Normal, 1.)?
                    .rgba8())
            };
            if let Some(w) = &s.watermark {
                let xx = x as i64 - s.options.view.watermark_origin[0] as i64;
                let yy = y as i64 - s.options.view.watermark_origin[1] as i64;
                if xx >= 0 && yy >= 0 && xx < w.width as i64 && yy < w.height as i64 {
                    let mut w = w.pixels[yy as usize * w.width as usize + xx as usize];
                    w[3] =
                        ((w[3] as u16 * s.options.view.watermark_opacity as u16 + 127) / 255) as u8;
                    p = overlay(p, w)?;
                }
            }
            if s.options.view.noise > 0 {
                let n = noise(s.options.view.seed, x, y);
                p = overlay(p, [n, n, n, s.options.view.noise])?;
            }
            if !s.options.view.transparent
                || s.options.format == Format::Movie
                    && s.options.codec == crate::movie_export::Codec::H264
            {
                let m = s.options.view.matte;
                let base = crate::color_precision::Sample::from8([m[0], m[1], m[2], 255]);
                let base = if preserve {
                    space.from_srgb(base)?
                } else {
                    base
                };
                p = base
                    .over(
                        crate::color_precision::Sample::from8(p),
                        crate::layer_blend::Mode::Normal,
                        1.,
                    )?
                    .rgba8();
            }
            bytes.extend_from_slice(&p);
        }
    }
    Ok(bytes)
}
pub(crate) fn icc(s: &Snapshot) -> Result<Option<Vec<u8>>, String> {
    if !s.options.view.icc
        || matches!(s.options.format, Format::Gif | Format::Movie)
        || s.document().color == crate::color_storage::Config::legacy()
    {
        return Ok(None);
    }
    if let crate::color_profile::RgbProfile::Embedded { bytes, .. } = &s.document().color.profile {
        return Ok(Some(bytes.clone()));
    }
    let mut icc = moxcms::ColorProfile::new_srgb()
        .encode()
        .map_err(|e| e.to_string())?;
    for (b, n) in icc[24..36]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([2000u16, 1, 1, 0, 0, 0])
    {
        b.copy_from_slice(&n.to_be_bytes());
    }
    Ok(Some(icc))
}
