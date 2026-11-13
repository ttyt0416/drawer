//! Last confirmed glyph geometry, not a font file. Input-bound and size-limited.
use crate::{
    scene::{Affine, PageObject, Point, Scene},
    sizing::DocumentState,
    text_format::PaintRange,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::{Arc, OnceLock},
};
const MAX_RAW: usize = 20 * 1024 * 1024;
const MAX_PACKED: usize = 2 * 1024 * 1024;
const MAX_POINTS: usize = 1024 * 1024;
const MAX_PATHS: usize = 65536;
#[derive(Clone, Debug)]
pub(crate) struct Geometry {
    pub rich: bool,
    pub paths: Vec<Vec<Point>>,
    pub backgrounds: Vec<Vec<Point>>,
    pub paint: Vec<PaintRange>,
    pub background: [u8; 4],
    widths: Vec<f64>,
    glyphs: u32,
    lines: u32,
}
impl Geometry {
    fn heap(&self) -> usize {
        self.paths
            .iter()
            .chain(&self.backgrounds)
            .map(|p| p.len() * 16 + 24)
            .sum::<usize>()
            + self.paint.len() * std::mem::size_of::<PaintRange>()
            + self.widths.len() * 8
            + 256
    }
    pub(crate) fn points(&self) -> usize {
        self.paths
            .iter()
            .chain(&self.backgrounds)
            .map(Vec::len)
            .sum()
    }
    fn valid(&self) -> bool {
        self.paths.len() + self.backgrounds.len() <= MAX_PATHS
            && self.points() <= MAX_POINTS
            && self.glyphs <= 65536
            && self.lines <= 65536
            && self.widths.len() <= 65536
            && self.widths.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
            && self
                .paths
                .iter()
                .chain(&self.backgrounds)
                .flatten()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
            && self.paint.len() <= 8192
            && self.paint.iter().all(|p| {
                p.start <= p.end
                    && p.end <= self.paths.len()
                    && p.paint.border.is_none_or(|b| b.valid())
            })
    }
    fn validate_frame(&self, frame: Affine) -> Result<(), String> {
        if !frame.valid()
            || self
                .paths
                .iter()
                .chain(&self.backgrounds)
                .flatten()
                .any(|p| {
                    frame
                        .map(*p)
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > 1e9)
                })
        {
            return Err("Transformed cached text coordinate limit".into());
        }
        Ok(())
    }
    pub(crate) fn framed(&self, frame: Affine) -> Result<Self, String> {
        self.validate_frame(frame)?;
        let mut out = self.clone();
        if !frame.is_identity() {
            for path in out.paths.iter_mut().chain(&mut out.backgrounds) {
                for p in path {
                    *p = frame.map(*p);
                }
            }
        }
        Ok(out)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cached {
    origin: Point,
    input_sha256: String,
    raw_sha256: String,
    raw_bytes: usize,
    heap_bytes: usize,
    packed: String,
    #[serde(skip, default)]
    decoded: OnceLock<Result<Arc<Geometry>, String>>,
}
impl PartialEq for Cached {
    fn eq(&self, b: &Self) -> bool {
        self.origin == b.origin
            && self.input_sha256 == b.input_sha256
            && self.raw_sha256 == b.raw_sha256
            && self.raw_bytes == b.raw_bytes
            && self.heap_bytes == b.heap_bytes
            && self.packed == b.packed
    }
}
impl Cached {
    fn small_valid(&self) -> bool {
        self.origin.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
            && crate::asset_catalog::valid_hash(&self.input_sha256)
            && crate::asset_catalog::valid_hash(&self.raw_sha256)
            && self.raw_bytes <= MAX_RAW
            && self.heap_bytes <= 32 * 1024 * 1024
            && self.packed.len() <= MAX_PACKED
    }
    fn geometry(&self) -> Result<Arc<Geometry>, String> {
        self.decoded.get_or_init(|| self.decode()).clone()
    }
    fn decode(&self) -> Result<Arc<Geometry>, String> {
        if !self.small_valid() {
            return Err("Text appearance limits/hash metadata invalid".into());
        }
        let compressed = STANDARD.decode(&self.packed).map_err(|e| e.to_string())?;
        let mut decoder = ZlibDecoder::new(compressed.as_slice());
        let mut raw = Vec::new();
        raw.try_reserve_exact(self.raw_bytes.saturating_add(1))
            .map_err(|_| "Text appearance allocation refused")?;
        decoder
            .by_ref()
            .take(self.raw_bytes as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
        if raw.len() != self.raw_bytes
            || decoder.total_in() != compressed.len() as u64
            || crate::asset_catalog::hash(&raw) != self.raw_sha256
        {
            return Err("Text appearance length/stream/hash differs".into());
        }
        let geometry = decode(&raw)?;
        if !geometry.valid() || geometry.heap() != self.heap_bytes {
            return Err("Text appearance geometry/heap metadata differs".into());
        }
        Ok(Arc::new(geometry))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    entries: BTreeMap<u64, Cached>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.entries.len() <= 4096
            && self
                .entries
                .iter()
                .all(|(id, c)| *id > 0 && c.small_valid())
            && self.entries.values().map(|c| c.packed.len()).sum::<usize>() <= MAX_PACKED
            && self.entries.values().map(|c| c.raw_bytes).sum::<usize>() <= MAX_RAW
            && self.bytes() <= 32 * 1024 * 1024
    }
    pub fn bytes(&self) -> usize {
        self.entries
            .values()
            .map(|c| c.packed.len() + c.heap_bytes + 128)
            .sum()
    }
    pub fn has_snapshot(&self, id: u64) -> bool {
        self.entries.contains_key(&id)
    }
    pub(crate) fn display(&self, scene: &Scene, n: usize) -> Result<Option<Arc<Geometry>>, String> {
        let Some(cached) = self.entries.get(&scene.catalog.objects.id(n)) else {
            return Ok(None);
        };
        if cached.input_sha256 != signature(&scene.objects[n])? {
            return Ok(None);
        }
        let mut g = cached.geometry()?;
        let PageObject::Text { baseline, .. } = &scene.objects[n] else {
            return Err("Cached target is not text".into());
        };
        let delta = [
            baseline[0] - cached.origin[0],
            baseline[1] - cached.origin[1],
        ];
        if delta != [0.; 2] {
            let mut shifted = (*g).clone();
            for point in shifted
                .paths
                .iter_mut()
                .chain(&mut shifted.backgrounds)
                .flatten()
            {
                for axis in 0..2 {
                    point[axis] += delta[axis];
                }
            }
            if !shifted.valid() {
                return Err("Moved text appearance coordinate limit".into());
            }
            g = Arc::new(shifted);
        }
        g.validate_frame(frame(&scene.objects[n]))?;
        Ok(Some(g))
    }
}
pub(crate) fn contours(scene: &Scene, n: usize) -> Result<Vec<Vec<Point>>, String> {
    if let Some(g) = scene.text_appearance.display(scene, n)? {
        return Ok(g.framed(frame(&scene.objects[n]))?.paths);
    }
    let PageObject::Text {
        baseline,
        text,
        size_px,
        layout,
        ..
    } = &scene.objects[n]
    else {
        return Err("Not a text object".into());
    };
    if let Some(style) = layout {
        Ok(crate::text_layout::layout(*baseline, text, *size_px, style)?.paths)
    } else {
        Ok(crate::render::legacy_text_paths(*baseline, text, *size_px))
    }
}
pub(crate) fn bounds(scene: &Scene, n: usize) -> Option<crate::scene::Rect> {
    let PageObject::Text {
        baseline,
        size_px,
        layout: Some(style),
        ..
    } = &scene.objects[n]
    else {
        return None;
    };
    if !style.extended() && style.frame.is_identity() {
        return None;
    }
    let g = scene
        .text_appearance
        .display(scene, n)
        .ok()??
        .framed(style.frame)
        .ok()?;
    let mut bounds = crate::scene::Rect {
        min: [baseline[0] - size_px, baseline[1] - size_px],
        max: [baseline[0] + size_px, baseline[1] + size_px],
    };
    if !style.frame.is_identity()
        && let Some(p) = g.paths.iter().flatten().next()
    {
        bounds = crate::scene::Rect { min: *p, max: *p };
    }
    for p in g.paths.iter().chain(&g.backgrounds).flatten() {
        for (axis, v) in p.iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(*v);
            bounds.max[axis] = bounds.max[axis].max(*v);
        }
    }
    let stroke = g
        .paint
        .iter()
        .filter_map(|p| p.paint.border)
        .map(|b| b.width / 2.)
        .fold(0., f64::max);
    bounds.min = bounds.min.map(|v| v - stroke);
    bounds.max = bounds.max.map(|v| v + stroke);
    Some(bounds)
}
fn signature(object: &PageObject) -> Result<String, String> {
    let PageObject::Text {
        text,
        size_px,
        layout,
        ..
    } = object
    else {
        return Err("Text cache target is not text".into());
    };
    let mut style = layout.as_deref().cloned();
    if let Some(s) = &mut style {
        s.frame = Default::default();
    }
    let data = serde_json::to_vec(&(text, size_px, style)).map_err(|e| e.to_string())?;
    Ok(crate::asset_catalog::hash(&data))
}
fn frame(object: &PageObject) -> Affine {
    match object {
        PageObject::Text { layout, .. } => {
            layout.as_ref().map_or_else(Default::default, |s| s.frame)
        }
        _ => Default::default(),
    }
}
fn capture(object: &PageObject) -> Result<Cached, String> {
    let PageObject::Text {
        baseline,
        text,
        size_px,
        layout,
        ..
    } = object
    else {
        return Err("Text cache target is not text".into());
    };
    let mut style = layout.as_deref().cloned();
    if let Some(style) = &mut style {
        style.frame = Default::default();
        if !text.is_empty() {
            for font in style.applied_fonts(text) {
                if let crate::text_layout::Font::Local(reference) = font
                    && crate::font_library::data(reference).is_none()
                {
                    return Err(format!(
                        "Original local font {} is unavailable. Preserve the cached appearance; select and review an available local replacement before editing or capturing uncached text.",
                        font.label()
                    ));
                }
            }
        }
    }
    let rich = style.as_ref().is_some_and(|s| s.rich.is_some());
    let geometry = if let Some(style) = &style {
        let shape = crate::text_layout::layout(*baseline, text, *size_px, style)?;
        Geometry {
            rich,
            paths: shape.paths,
            backgrounds: if rich { shape.backgrounds } else { vec![] },
            paint: if rich { shape.paint } else { vec![] },
            background: style.typeset.as_ref().map_or([0; 4], |o| o.background),
            widths: shape.widths,
            glyphs: shape.glyphs as u32,
            lines: shape.lines as u32,
        }
    } else {
        Geometry {
            rich: false,
            paths: crate::render::legacy_text_paths(*baseline, text, *size_px),
            backgrounds: vec![],
            paint: vec![],
            background: [0; 4],
            widths: vec![],
            glyphs: text.chars().count() as u32,
            lines: text.lines().count() as u32,
        }
    };
    if !geometry.valid() {
        return Err("Text appearance outline/position budget exceeded".into());
    }
    let raw = encode(&geometry)?;
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&raw).map_err(|e| e.to_string())?;
    let packed = STANDARD.encode(z.finish().map_err(|e| e.to_string())?);
    let cached = Cached {
        origin: *baseline,
        input_sha256: signature(object)?,
        raw_sha256: crate::asset_catalog::hash(&raw),
        raw_bytes: raw.len(),
        heap_bytes: geometry.heap(),
        packed,
        decoded: OnceLock::new(),
    };
    if !cached.small_valid() {
        return Err(
            "Text appearance exceeds the local cache budget; split the text or reduce complexity"
                .into(),
        );
    }
    Ok(cached)
}
pub(crate) fn refresh(
    before: &crate::document::Document,
    after: &mut DocumentState,
) -> Result<(), String> {
    refresh_scene(Some(&before.scene), &mut after.scene, false)
}
pub(crate) fn freeze_all(scene: &mut Scene) -> Result<(), String> {
    refresh_scene(None, scene, true)
}
fn refresh_scene(before: Option<&Scene>, scene: &mut Scene, all: bool) -> Result<(), String> {
    let mut next = Data::default();
    for (n, object) in scene.objects.iter().enumerate() {
        if !matches!(object, PageObject::Text { .. }) {
            continue;
        }
        let id = scene.catalog.objects.id(n);
        let input = signature(object)?;
        let candidate = scene
            .text_appearance
            .entries
            .get(&id)
            .filter(|c| c.input_sha256 == input)
            .or_else(|| {
                scene
                    .text_appearance
                    .entries
                    .values()
                    .find(|c| c.input_sha256 == input)
            })
            .or_else(|| {
                before.and_then(|s| {
                    s.text_appearance
                        .entries
                        .values()
                        .find(|c| c.input_sha256 == input)
                })
            });
        if let Some(cached) = candidate {
            cached.geometry()?.validate_frame(frame(object))?;
            next.entries.insert(id, cached.clone());
            continue;
        }
        let unchanged = before
            .and_then(|s| {
                s.catalog
                    .objects
                    .index(id, s.objects.len())
                    .map(|i| &s.objects[i])
            })
            .is_some_and(|old| signature(old).is_ok_and(|h| h == input));
        if all || !unchanged {
            let cached = capture(object)?;
            cached.geometry()?.validate_frame(frame(object))?;
            next.entries.insert(id, cached);
        }
    }
    if !next.valid() {
        return Err("Combined text appearance exceeds the document cache budget".into());
    }
    scene.text_appearance = next;
    Ok(())
}
pub(crate) fn validate_native(scene: &Scene, available: usize) -> Result<(), String> {
    if !scene.text_appearance.valid() {
        return Err("Invalid text appearance metadata".into());
    }
    let working = scene
        .text_appearance
        .entries
        .values()
        .map(|c| {
            c.raw_bytes
                .saturating_mul(2)
                .saturating_add(2 * 1024 * 1024)
        })
        .max()
        .unwrap_or(0);
    if working > available {
        return Err("Insufficient working memory to validate text appearance".into());
    }
    for (id, cached) in &scene.text_appearance.entries {
        let n = scene
            .catalog
            .objects
            .index(*id, scene.objects.len())
            .ok_or("Missing cached text target")?;
        if signature(&scene.objects[n])? != cached.input_sha256 {
            return Err("Cached text input signature differs".into());
        }
        scene.text_appearance.display(scene, n)?;
    }
    Ok(())
}
fn paths(out: &mut Vec<u8>, value: &[Vec<Point>]) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    for path in value {
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        for point in path {
            for v in point {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
}
fn encode(g: &Geometry) -> Result<Vec<u8>, String> {
    let mut out = b"DTXT01\r\n".to_vec();
    out.push(u8::from(g.rich));
    out.extend_from_slice(&g.glyphs.to_le_bytes());
    out.extend_from_slice(&g.lines.to_le_bytes());
    paths(&mut out, &g.paths);
    paths(&mut out, &g.backgrounds);
    out.extend_from_slice(&(g.widths.len() as u32).to_le_bytes());
    for v in &g.widths {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let paint = serde_json::to_vec(&g.paint).map_err(|e| e.to_string())?;
    if paint.len() > 1024 * 1024 {
        return Err("Text paint cache limit".into());
    }
    out.extend_from_slice(&(paint.len() as u32).to_le_bytes());
    out.extend_from_slice(&paint);
    out.extend_from_slice(&g.background);
    if out.len() > MAX_RAW {
        return Err("Text appearance raw limit".into());
    }
    Ok(out)
}
struct Reader<'a> {
    bytes: &'a [u8],
    points: usize,
    paths: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let b = self.bytes.get(..n).ok_or("Truncated text appearance")?;
        self.bytes = &self.bytes[n..];
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| "Text integer")?,
        ))
    }
    fn f64(&mut self) -> Result<f64, String> {
        let v = f64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "Text coordinate")?);
        if !v.is_finite() || v.abs() > 1e9 {
            return Err("Invalid cached text coordinate".into());
        }
        Ok(v)
    }
    fn paths(&mut self) -> Result<Vec<Vec<Point>>, String> {
        let n = self.u32()? as usize;
        self.paths = self.paths.checked_add(n).ok_or("Text path overflow")?;
        if self.paths > MAX_PATHS || n > self.bytes.len() / 4 {
            return Err("Text path count limit".into());
        }
        let mut paths = Vec::new();
        paths
            .try_reserve_exact(n)
            .map_err(|_| "Text path allocation refused")?;
        for _ in 0..n {
            let count = self.u32()? as usize;
            self.points = self
                .points
                .checked_add(count)
                .ok_or("Text point overflow")?;
            if self.points > MAX_POINTS || count > self.bytes.len() / 16 {
                return Err("Text point count limit".into());
            }
            let mut path = Vec::new();
            path.try_reserve_exact(count)
                .map_err(|_| "Text point allocation refused")?;
            for _ in 0..count {
                path.push([self.f64()?, self.f64()?]);
            }
            paths.push(path);
        }
        Ok(paths)
    }
}
fn decode(bytes: &[u8]) -> Result<Geometry, String> {
    let mut r = Reader {
        bytes,
        points: 0,
        paths: 0,
    };
    if r.take(8)? != b"DTXT01\r\n" {
        return Err("Unsupported text appearance format".into());
    }
    let rich = match r.take(1)?[0] {
        0 => false,
        1 => true,
        _ => return Err("Invalid text appearance mode".into()),
    };
    let glyphs = r.u32()?;
    let lines = r.u32()?;
    let paths = r.paths()?;
    let backgrounds = r.paths()?;
    let count = r.u32()? as usize;
    if count > 65536 || count > r.bytes.len() / 8 {
        return Err("Text advance count limit".into());
    }
    let mut widths = Vec::new();
    widths
        .try_reserve_exact(count)
        .map_err(|_| "Text advance allocation refused")?;
    for _ in 0..count {
        widths.push(r.f64()?);
    }
    let count = r.u32()? as usize;
    if count > 1024 * 1024 {
        return Err("Text paint cache limit".into());
    }
    let paint: Vec<PaintRange> =
        serde_json::from_slice(r.take(count)?).map_err(|e| e.to_string())?;
    let background = r.take(4)?.try_into().map_err(|_| "Text background")?;
    if !r.bytes.is_empty() {
        return Err("Unexpected text appearance bytes".into());
    }
    Ok(Geometry {
        rich,
        paths,
        backgrounds,
        paint,
        background,
        widths,
        glyphs,
        lines,
    })
}
#[cfg(test)]
#[path = "text_appearance_verify.rs"]
mod tests;
