//! Editable horizontal text. Font references and Unicode remain in the document.
use crate::scene::{PageObject, Point, Rect};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Font {
    #[default]
    Korean,
    Ubuntu,
    Hack,
    Emoji,
}
impl Font {
    pub const ALL: [Self; 4] = [Self::Korean, Self::Ubuntu, Self::Hack, Self::Emoji];
    pub fn label(self) -> &'static str {
        match self {
            Self::Korean => "Malgun Gothic (local system)",
            Self::Ubuntu => "Ubuntu Light",
            Self::Hack => "Hack Regular",
            Self::Emoji => "Noto Emoji (monochrome)",
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
}
impl Default for Style {
    fn default() -> Self {
        Self {
            font: Font::Korean,
            align: Align::Left,
            line_height: 1.2,
            wrap_width: 320.,
        }
    }
}
impl Style {
    pub fn legacy() -> Self {
        Self {
            font: Font::Ubuntu,
            wrap_width: 0.,
            ..Default::default()
        }
    }
    pub fn valid(&self) -> bool {
        (0.5..=8.).contains(&self.line_height) && (0. ..=32768.).contains(&self.wrap_width)
    }
    pub fn bounds(&self, baseline: Point, text: &str, size: f64) -> Rect {
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
}
impl Bytes {
    fn data(&self) -> &[u8] {
        match self {
            Self::Static(b) => b,
            Self::System(b) => b,
        }
    }
}
type SystemCache = Option<(u64, std::time::SystemTime, Arc<Vec<u8>>)>;
fn system_font() -> Option<Arc<Vec<u8>>> {
    static CACHE: Mutex<SystemCache> = Mutex::new(None);
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
    let mut cached = CACHE.lock().ok()?;
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
    })
}
pub struct Layout {
    pub paths: Vec<Vec<Point>>,
    pub lines: usize,
    pub missing: usize,
    pub substituted: usize,
    pub widths: Vec<f64>,
}
struct Glyph {
    font: usize,
    id: ttf_parser::GlyphId,
    x: f64,
    line: usize,
}
pub fn layout(baseline: Point, text: &str, size: f64, style: &Style) -> Result<Layout, String> {
    if !style.valid()
        || !(0.1..=4096.).contains(&size)
        || text.len() > 65536
        || text.chars().count() > 4096
        || baseline.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
    {
        return Err("Text size0.1–4096, Unicode4096 characters/64KiB, wrap0–32768 and line spacing0.5–8 required".into());
    }
    let mut fonts = Vec::new();
    for font in [
        style.font,
        Font::Korean,
        Font::Ubuntu,
        Font::Hack,
        Font::Emoji,
    ] {
        if !fonts.iter().any(|(f, _)| *f == font)
            && let Some(b) = bytes(font)
        {
            fonts.push((font, b));
        }
    }
    let faces: Vec<_> = fonts
        .iter()
        .map(|(_, b)| {
            ttf_parser::Face::parse(b.data(), 0).map_err(|e| format!("Font parse: {e:?}"))
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
        let found = faces
            .iter()
            .enumerate()
            .find_map(|(i, f)| f.glyph_index(c).map(|id| (i, id)));
        let (i, id) = found.unwrap_or((0, ttf_parser::GlyphId(0)));
        if found.is_none() {
            missing += 1;
        } else if fonts[i].0 != style.font {
            substituted += 1;
        }
        let face = &faces[i];
        let advance = f64::from(face.glyph_hor_advance(id).unwrap_or(face.units_per_em()))
            / f64::from(face.units_per_em())
            * size;
        if style.wrap_width > 0. && widths[line] > 0. && widths[line] + advance > style.wrap_width {
            line += 1;
            widths.push(0.);
        }
        glyphs.push(Glyph {
            font: i,
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
        let current = crate::render::glyph_paths(&faces[glyph.font], glyph.id, origin, size);
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
        lines: widths.len(),
        missing,
        substituted,
        widths,
    })
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
