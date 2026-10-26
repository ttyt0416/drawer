//! Editable text. Font references and Unicode remain in the document.
#[path = "text_reading.rs"]
mod reading;
use crate::scene::{PageObject, Point, Rect};
pub use reading::{Flow, Ruby, Side};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Font {
    #[default]
    Korean,
    Ubuntu,
    Hack,
    Emoji,
    Local(crate::font_library::Reference),
}
impl Font {
    pub const ALL: [Self; 4] = [Self::Korean, Self::Ubuntu, Self::Hack, Self::Emoji];
    pub fn valid(self) -> bool {
        match self {
            Self::Local(r) => r.valid(),
            _ => true,
        }
    }
    pub fn label(self) -> String {
        match self {
            Self::Korean => "Malgun Gothic (local system)".into(),
            Self::Ubuntu => "Ubuntu Light".into(),
            Self::Hack => "Hack Regular".into(),
            Self::Emoji => "Noto Emoji (monochrome)".into(),
            Self::Local(reference) => reference.name().into(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub font: Font,
    pub align: Align,
    pub line_height: f64,
    pub wrap_width: f64,
    pub flow: Flow,
    pub tracking: f64,
    pub ligatures: bool,
    pub ruby: Vec<Ruby>,
    pub rich: Option<Box<crate::text_format::Rich>>,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            font: Font::Korean,
            align: Align::Left,
            line_height: 1.2,
            wrap_width: 320.,
            flow: Flow::Horizontal,
            tracking: 0.,
            ligatures: false,
            ruby: Vec::new(),
            rich: None,
        }
    }
}
impl Style {
    pub fn fonts(&self) -> Vec<Font> {
        let mut fonts = vec![self.font];
        fonts.extend(Font::ALL);
        fonts.extend(self.ruby.iter().map(|r| r.font));
        if let Some(r) = &self.rich {
            fonts.extend(r.spans.iter().filter_map(|s| s.attr.font));
            if let Some(m) = &r.mapping {
                fonts.push(m.base);
                fonts.extend(m.rules.iter().map(|r| r.font));
            }
        }
        fonts
    }
    pub fn legacy() -> Self {
        Self {
            font: Font::Ubuntu,
            wrap_width: 0.,
            ..Default::default()
        }
    }
    pub fn valid(&self) -> bool {
        self.fonts().iter().all(|f| f.valid())
            && (0.5..=8.).contains(&self.line_height)
            && (0. ..=32768.).contains(&self.wrap_width)
            && (-0.5..=4.).contains(&self.tracking)
            && self.ruby.len() <= 64
            && self.ruby.iter().all(Ruby::valid)
    }
    pub fn extended(&self) -> bool {
        self.flow != Flow::Horizontal
            || self.tracking != 0.
            || self.ligatures
            || !self.ruby.is_empty()
            || self.rich.is_some()
    }
    pub fn bytes(&self) -> usize {
        self.rich.as_ref().map_or(0, |r| r.bytes())
            + std::mem::size_of_val(self)
            + self
                .ruby
                .iter()
                .map(|r| std::mem::size_of_val(r) + r.text.len())
                .sum::<usize>()
    }
    pub fn text_valid(&self, text: &str) -> bool {
        self.valid()
            && reading::ranges_valid(text, &self.ruby)
            && self.rich.as_ref().is_none_or(|r| {
                r.valid(text)
                    && self.ruby.iter().all(|a| {
                        crate::text_format::boundary(text, a.start)
                            && crate::text_format::boundary(text, a.end)
                    })
            })
    }
    pub fn remap_text(&mut self, before: &str, after: &str) {
        reading::remap(before, after, &mut self.ruby);
        if let Some(r) = &mut self.rich {
            r.remap(before, after);
            let mut end = 0;
            self.ruby.retain_mut(|r| {
                let (a, b) = crate::text_format::expand(after, r.start, r.end);
                r.start = a;
                r.end = b;
                let keep = a >= end && a < b;
                end = b;
                keep
            });
        }
    }
    pub fn bounds(&self, baseline: Point, text: &str, size: f64) -> Rect {
        if self.extended()
            && let Ok(shape) = layout(baseline, text, size, self)
        {
            let mut bounds = Rect {
                min: [baseline[0] - size, baseline[1] - size],
                max: [baseline[0] + size, baseline[1] + size],
            };
            for p in shape.paths.iter().flatten() {
                for (axis, value) in p.iter().enumerate() {
                    bounds.min[axis] = bounds.min[axis].min(*value);
                    bounds.max[axis] = bounds.max[axis].max(*value);
                }
            }
            let stroke = shape
                .paint
                .iter()
                .filter_map(|p| p.paint.border)
                .map(|b| b.width / 2.)
                .fold(0., f64::max);
            bounds.min = bounds.min.map(|v| v - stroke);
            bounds.max = bounds.max.map(|v| v + stroke);
            return bounds;
        }
        let width = if self.wrap_width > 0. {
            self.wrap_width
        } else {
            text.lines()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0)
                .max(1) as f64
                * size
                * 2.
        };
        let lines: usize = text
            .split('\n')
            .map(|line| {
                if self.wrap_width > 0. {
                    (line.chars().count() as f64 * size * 2. / self.wrap_width)
                        .ceil()
                        .max(1.) as usize
                } else {
                    1
                }
            })
            .sum();
        Rect {
            min: [baseline[0] - size, baseline[1] - size * 2.],
            max: [
                baseline[0] + width + size,
                baseline[1] + lines as f64 * size * self.line_height + size,
            ],
        }
    }
}
enum Bytes {
    Static(&'static [u8]),
    System(Arc<Vec<u8>>),
    Indexed(Arc<Vec<u8>>, u32),
}
impl Bytes {
    fn index(&self) -> u32 {
        match self {
            Self::Indexed(_, n) => *n,
            _ => 0,
        }
    }
    fn data(&self) -> &[u8] {
        match self {
            Self::Static(b) => b,
            Self::System(b) | Self::Indexed(b, _) => b,
        }
    }
}
type SystemCache = Option<(u64, std::time::SystemTime, Arc<Vec<u8>>)>;
static SYSTEM_FONT_CACHE: Mutex<SystemCache> = Mutex::new(None);
fn system_font() -> Option<Arc<Vec<u8>>> {
    let root = std::path::PathBuf::from(std::env::var_os("WINDIR")?);
    if !root.is_absolute() || root.to_string_lossy().starts_with("\\\\") {
        return None;
    }
    let path = root.join("Fonts/malgun.ttf");
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > 32 * 1024 * 1024 {
        return None;
    }
    let stamp = meta.modified().ok()?;
    let mut cached = SYSTEM_FONT_CACHE.lock().ok()?;
    if let Some((len, time, data)) = &*cached
        && *len == meta.len()
        && *time == stamp
    {
        return Some(data.clone());
    }
    let data = std::fs::read(path).ok()?;
    ttf_parser::Face::parse(&data, 0).ok()?;
    let data = Arc::new(data);
    *cached = Some((meta.len(), stamp, data.clone()));
    Some(data)
}
fn bytes(font: Font) -> Option<Bytes> {
    Some(match font {
        Font::Korean => Bytes::System(system_font()?),
        Font::Ubuntu => Bytes::Static(epaint_default_fonts::UBUNTU_LIGHT),
        Font::Hack => Bytes::Static(epaint_default_fonts::HACK_REGULAR),
        Font::Emoji => Bytes::Static(epaint_default_fonts::NOTO_EMOJI_REGULAR),
        Font::Local(reference) => {
            Bytes::Indexed(crate::font_library::data(reference)?, reference.index)
        }
    })
}
pub struct Layout {
    pub paths: Vec<Vec<Point>>,
    pub paint: Vec<crate::text_format::PaintRange>,
    pub lines: usize,
    pub glyphs: usize,
    pub unavailable_fonts: Vec<Font>,
    pub format_unavailable: Vec<Font>,
    pub missing: usize,
    pub substituted: usize,
    pub widths: Vec<f64>,
    pub requested_available: bool,
    pub fallback_fonts: Vec<Font>,
    pub missing_characters: Vec<char>,
    pub ligature_unavailable: Vec<Font>,
}
struct Glyph {
    font: usize,
    missing: bool,
    id: ttf_parser::GlyphId,
    x: f64,
    line: usize,
}
pub fn layout(baseline: Point, text: &str, size: f64, style: &Style) -> Result<Layout, String> {
    let mut fonts = Vec::new();
    for font in style.fonts() {
        if !fonts.iter().any(|(f, _)| *f == font)
            && let Some(b) = bytes(font)
            && fonts
                .iter()
                .map(|(_, b): &(Font, Bytes)| b.data().len())
                .sum::<usize>()
                + b.data().len()
                <= 64 * 1024 * 1024
        {
            fonts.push((font, b));
        }
    }
    layout_with_fonts(baseline, text, size, style, fonts)
}
fn layout_with_fonts(
    baseline: Point,
    text: &str,
    size: f64,
    style: &Style,
    fonts: Vec<(Font, Bytes)>,
) -> Result<Layout, String> {
    if !style.text_valid(text)
        || !(0.1..=4096.).contains(&size)
        || text.len() > 65536
        || text.chars().count() > 4096
        || baseline.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
    {
        return Err("Text size0.1–4096, Unicode4096 characters/64KiB, wrap0–32768 and line spacing0.5–8 required".into());
    }
    if style.extended() {
        return reading::layout(baseline, text, size, style, &fonts);
    }
    let requested_available = fonts.iter().any(|(font, _)| *font == style.font);
    let mut fallback_fonts = Vec::new();
    let mut missing_characters = Vec::new();
    let faces: Vec<_> = fonts
        .iter()
        .map(|(_, b)| {
            ttf_parser::Face::parse(b.data(), b.index()).map_err(|e| format!("Font parse: {e:?}"))
        })
        .collect::<Result<_, _>>()?;
    let mut glyphs = Vec::new();
    let mut widths = vec![0.];
    let (mut line, mut missing, mut substituted) = (0, 0, 0);
    for c in text.chars() {
        if c == '\r' {
            continue;
        }
        if c == '\n' {
            line += 1;
            widths.push(0.);
            continue;
        }
        if matches!(c, '\u{200d}' | '\u{fe0e}' | '\u{fe0f}') {
            continue;
        }
        let c = if c == '\t' { ' ' } else { c };
        let found = faces.iter().enumerate().find_map(|(i, f)| {
            glyph_supported(f, c)
                .then(|| f.glyph_index(c).map(|id| (i, id)))
                .flatten()
        });
        let (i, id) = found.unwrap_or((0, ttf_parser::GlyphId(0)));
        if found.is_none() {
            missing += 1;
            if missing_characters.len() < 16 && !missing_characters.contains(&c) {
                missing_characters.push(c);
            }
        } else if fonts[i].0 != style.font {
            substituted += 1;
            if !fallback_fonts.contains(&fonts[i].0) {
                fallback_fonts.push(fonts[i].0);
            }
        }
        let advance = if found.is_none() {
            size * 0.7
        } else {
            let face = &faces[i];
            f64::from(face.glyph_hor_advance(id).unwrap_or(face.units_per_em()))
                / f64::from(face.units_per_em())
                * size
        };
        if style.wrap_width > 0. && widths[line] > 0. && widths[line] + advance > style.wrap_width {
            line += 1;
            widths.push(0.);
        }
        glyphs.push(Glyph {
            font: i,
            missing: found.is_none(),
            id,
            x: widths[line],
            line,
        });
        widths[line] += advance;
    }
    let box_width = if style.wrap_width > 0. {
        style.wrap_width
    } else {
        widths.iter().copied().fold(0., f64::max)
    };
    let mut paths = Vec::new();
    let mut points = 0usize;
    let glyph_count = glyphs.len();
    for glyph in glyphs {
        let shift = match style.align {
            Align::Left => 0.,
            Align::Center => (box_width - widths[glyph.line]) / 2.,
            Align::Right => box_width - widths[glyph.line],
        };
        let origin = [
            baseline[0] + glyph.x + shift,
            baseline[1] + glyph.line as f64 * size * style.line_height,
        ];
        let current = if glyph.missing {
            missing_box(origin, size)
        } else {
            crate::render::glyph_paths(&faces[glyph.font], glyph.id, origin, size)
        };
        points = points.saturating_add(current.iter().map(Vec::len).sum::<usize>());
        if points > 512 * 1024 {
            return Err(
                "Text outline point budget exceeded; split the text into smaller objects".into(),
            );
        }
        paths.extend(current);
    }
    Ok(Layout {
        paths,
        paint: Vec::new(),
        lines: widths.len(),
        glyphs: glyph_count,
        unavailable_fonts: Vec::new(),
        format_unavailable: Vec::new(),
        missing,
        substituted,
        widths,
        requested_available,
        fallback_fonts,
        missing_characters,
        ligature_unavailable: Vec::new(),
    })
}
// Own outline ensures that a font with an empty .notdef glyph cannot hide missing text.
fn missing_box([x, y]: Point, size: f64) -> Vec<Vec<Point>> {
    let (left, right, top, bottom) = (x + size * 0.08, x + size * 0.62, y - size * 0.8, y);
    let t = size * 0.065;
    vec![
        vec![[left, top], [right, top], [right, bottom], [left, bottom]],
        vec![
            [left + t, top + t],
            [left + t, bottom - t],
            [right - t, bottom - t],
            [right - t, top + t],
        ],
    ]
}
impl Layout {
    pub fn notice(&self, requested: Font) -> String {
        let mut messages = Vec::new();
        if !self.requested_available {
            messages.push(format!("Requested font unavailable: {}", requested.label()));
        }
        for font in &self.unavailable_fonts {
            if *font != requested {
                messages.push(format!(
                    "Requested reading font unavailable: {}",
                    font.label()
                ));
            }
        }
        for font in &self.format_unavailable {
            messages.push(format!(
                "Requested format font unavailable: {}",
                font.label()
            ));
        }
        if self.substituted > 0 {
            messages.push(format!(
                "Fallback characters: {}; fonts: {}",
                self.substituted,
                self.fallback_fonts
                    .iter()
                    .map(|f| f.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.missing > 0 {
            messages.push(format!(
                "Missing glyph boxes: {}; {}",
                self.missing,
                self.missing_characters
                    .iter()
                    .map(|c| format!("U+{:04X}", *c as u32))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.ligature_unavailable.is_empty() {
            messages.push(format!(
                "Standard ligatures unavailable in {}; separate glyphs retained",
                self.ligature_unavailable
                    .iter()
                    .map(|f| f.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !messages.is_empty() {
            messages.push("Original Unicode is preserved".into());
        }
        messages.join(". ")
    }
}
pub fn remap_ruby(before: &str, after: &str, ruby: &mut Vec<Ruby>) {
    reading::remap(before, after, ruby);
}
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub baseline: Point,
    pub text: String,
    pub size: f64,
    pub color: [u8; 4],
    pub style: Style,
}
impl Settings {
    pub fn from_object(object: &PageObject) -> Option<Self> {
        let PageObject::Text {
            baseline,
            text,
            size_px,
            color,
            layout,
        } = object
        else {
            return None;
        };
        Some(Self {
            baseline: *baseline,
            text: text.clone(),
            size: *size_px,
            color: *color,
            style: layout.as_deref().cloned().unwrap_or_else(Style::legacy),
        })
    }
}
pub struct Prepared {
    pub state: crate::sizing::DocumentState,
    pub id: u64,
    pub missing: usize,
    pub substituted: usize,
    pub notice: String,
}
pub fn prepare(
    d: &crate::document::Document,
    id: Option<u64>,
    s: &Settings,
) -> Result<Prepared, String> {
    if d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Text snapshot memory limit".into());
    }
    let shape = layout(s.baseline, &s.text, s.size, &s.style)?;
    let notice = shape.notice(s.style.font);
    let mut state = d.snapshot();
    let object = PageObject::Text {
        baseline: s.baseline,
        text: s.text.clone(),
        size_px: s.size,
        color: s.color,
        layout: Some(Box::new(s.style.clone())),
    };
    let n = if let Some(id) = id {
        let n = d
            .scene
            .catalog
            .objects
            .index(id, d.scene.objects.len())
            .ok_or("Text object disappeared")?;
        if !matches!(d.scene.objects[n], PageObject::Text { .. }) {
            return Err("Select a text object".into());
        }
        if matches!(
            d.scene.objects[n],
            PageObject::Text {
                layout: Some(_),
                ..
            }
        ) && Settings::from_object(&d.scene.objects[n]).as_ref() == Some(s)
        {
            return Ok(Prepared {
                state,
                id,
                missing: shape.missing,
                substituted: shape.substituted,
                notice,
            });
        }
        state.scene.objects[n] = object;
        n
    } else {
        if state.scene.objects.len() >= 4096 {
            return Err("Page object limit4096".into());
        }
        state.scene.objects.push(object);
        state.scene.objects.len() - 1
    };
    validate_scene(&state.scene)?;
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    crate::layer_groups::finish(d, &state)?;
    if id.is_none() {
        let object_id = state.scene.catalog.objects.id(n);
        state.scene.render_order.entries = crate::render_order::normalized(&d.scene);
        state
            .scene
            .render_order
            .entries
            .push(crate::render_order::Entry {
                key: crate::layer_search::Key::Object(object_id),
                group: crate::render_order::Group::Object,
            });
        crate::layer_groups::finish(d, &state)?;
    }
    let id = state.scene.catalog.objects.id(n);
    Ok(Prepared {
        state,
        id,
        missing: shape.missing,
        substituted: shape.substituted,
        notice,
    })
}

pub fn validate_scene(scene: &crate::scene::Scene) -> Result<(), String> {
    let mut total = 0usize;
    for o in &scene.objects {
        if let PageObject::Text {
            layout: Some(style),
            baseline,
            text,
            size_px,
            ..
        } = o
        {
            let l = layout(*baseline, text, *size_px, style)?;
            total = total.saturating_add(l.paths.iter().map(Vec::len).sum::<usize>());
            if total > 1024 * 1024 {
                return Err("Page text outline point budget exceeded".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn glyph_supported(face: &ttf_parser::Face<'_>, c: char) -> bool {
    face.glyph_index(c).is_some_and(|id| {
        c.is_whitespace()
            || face
                .glyph_bounding_box(id)
                .is_some_and(|r| r.x_max > r.x_min && r.y_max > r.y_min)
    })
}
pub fn font_covers(font: Font, text: &str) -> bool {
    bytes(font).is_some_and(|b| {
        ttf_parser::Face::parse(b.data(), b.index()).is_ok_and(|f| {
            text.chars()
                .all(|c| c.is_whitespace() || glyph_supported(&f, c))
        })
    })
}
pub fn invalidate_font_data() {
    if let Ok(mut c) = SYSTEM_FONT_CACHE.lock() {
        *c = None;
    }
    crate::font_library::invalidate_data();
}
pub fn font_supports(font: Font, c: char) -> bool {
    bytes(font).is_some_and(|b| {
        ttf_parser::Face::parse(b.data(), b.index()).is_ok_and(|f| glyph_supported(&f, c))
    })
}

pub fn font_bytes(font: Font) -> Option<(Arc<Vec<u8>>, u32)> {
    let b = bytes(font)?;
    let index = b.index();
    let data = match b {
        Bytes::Static(b) => Arc::new(b.to_vec()),
        Bytes::System(b) | Bytes::Indexed(b, _) => b,
    };
    Some((data, index))
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    #[test]
    fn unavailable_system_and_no_fallback_glyph_still_get_notice_and_visible_outline() {
        let style = Style::default();
        let l = layout_with_fonts(
            [0., 20.],
            "한\u{10ffff}",
            20.,
            &style,
            vec![(
                Font::Emoji,
                Bytes::Static(epaint_default_fonts::NOTO_EMOJI_REGULAR),
            )],
        )
        .unwrap();
        assert!(!l.requested_available);
        assert_eq!(l.missing, 2);
        assert_eq!(l.paths.len(), 4);
        assert!(l.notice(style.font).contains("Requested font unavailable"));
        assert!(l.notice(style.font).contains("U+10FFFF"));
        let empty = layout_with_fonts([0., 20.], "한", 20., &style, vec![]).unwrap();
        assert_eq!(empty.missing, 1);
        assert_eq!(empty.paths.len(), 2);
        assert!(empty.widths[0] > 0.);
        let style = Style {
            flow: Flow::Vertical,
            ruby: vec![Ruby {
                start: 0,
                end: 1,
                text: "독음".into(),
                font: Font::Hack,
                ..Default::default()
            }],
            ..style
        };
        let l = layout_with_fonts([30., 30.], "한", 20., &style, vec![]).unwrap();
        assert_eq!(l.missing, 3);
        assert_eq!(l.paths.len(), 6);
        assert!(l.notice(style.font).contains("reading font unavailable"));
        let style = Style {
            font: Font::Ubuntu,
            rich: Some(Box::new(crate::text_format::Rich {
                spans: vec![crate::text_format::Span {
                    start: 0,
                    end: 2,
                    attr: crate::text_format::Attr {
                        font: Some(Font::Korean),
                        ..Default::default()
                    },
                }],
                mapping: None,
            })),
            ..Default::default()
        };
        let l = layout_with_fonts([0.; 2], "a\u{301}", 20., &style, vec![]).unwrap();
        assert_eq!(l.glyphs, 1);
        assert_eq!(l.paint.len(), 1);
        assert!(l.notice(style.font).contains("format font unavailable"));
    }
}
