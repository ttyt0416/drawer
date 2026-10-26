#[path = "text_shape_rich.rs"]
mod rich_shape;
use super::*;
use crate::text_format::{Paint, PaintRange};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    #[default]
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    #[default]
    Before,
    After,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ruby {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub font: Font,
    pub ratio: f64,
    pub align: Align,
    pub side: Side,
    pub gap: f64,
    pub tracking: f64,
}
impl Default for Ruby {
    fn default() -> Self {
        Self {
            start: 0,
            end: 1,
            text: String::new(),
            font: Font::Korean,
            ratio: 0.5,
            align: Align::Center,
            side: Side::Before,
            gap: 0.1,
            tracking: 0.,
        }
    }
}
impl Ruby {
    pub fn valid(&self) -> bool {
        self.start < self.end
            && self.end <= 4096
            && self.text.len() <= 4096
            && self.text.chars().count() <= 1024
            && !self.text.contains(['\n', '\r'])
            && (0.1..=2.).contains(&self.ratio)
            && (0. ..=4.).contains(&self.gap)
            && (-0.5..=4.).contains(&self.tracking)
    }
}
pub(super) fn ranges_valid(text: &str, ruby: &[Ruby]) -> bool {
    let chars: Vec<_> = text.chars().collect();
    let mut end = 0;
    ruby.iter().all(|r| {
        let valid = r.start >= end
            && r.end <= chars.len()
            && r.start < r.end
            && !chars[r.start..r.end].contains(&'\n');
        end = r.end;
        valid
    }) && ruby.iter().map(|r| r.text.chars().count()).sum::<usize>() <= 4096
}
pub(super) fn remap(before: &str, after: &str, ruby: &mut Vec<Ruby>) {
    if before == after {
        return;
    }
    let a: Vec<_> = before.chars().collect();
    let b: Vec<_> = after.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (old_end, new_end) = (a.len() - suffix, b.len() - suffix);
    let shift = |n: usize| n.saturating_sub(old_end) + new_end;
    ruby.retain_mut(|r| {
        if r.end <= prefix {
            return true;
        }
        if r.start >= old_end {
            r.start = shift(r.start);
            r.end = shift(r.end);
            return true;
        }
        // A fully removed annotation disappears. Replacement text takes its place in the range.
        let start = if r.start <= prefix { r.start } else { prefix };
        let end = if r.end >= old_end {
            shift(r.end)
        } else {
            new_end
        };
        r.start = start;
        r.end = end;
        start < end && end <= b.len() && !b[start..end].contains(&'\n')
    });
    let mut end = 0;
    ruby.retain(|r| {
        let keep = r.start >= end;
        if keep {
            end = r.end;
        }
        keep
    });
}
struct Cell {
    paths: Vec<Vec<Point>>,
    advance: f64,
    paint: Option<Paint>,
}
enum Token {
    Cell(Cell),
    Break,
}
struct Engine<'a> {
    fonts: &'a [(Font, Bytes)],
    faces: Vec<ttf_parser::Face<'a>>,
    missing: usize,
    substituted: usize,
    fallback_fonts: Vec<Font>,
    missing_characters: Vec<char>,
    ligature_unavailable: Vec<Font>,
    points: usize,
    glyphs: usize,
}
impl<'a> Engine<'a> {
    fn dispatch(
        &mut self,
        text: &str,
        start: usize,
        font: Font,
        size: f64,
        style: &Style,
        body: bool,
    ) -> Result<Vec<Token>, String> {
        if style.rich.is_some() {
            self.formatted(text, start, font, size, style, body)
        } else {
            self.shape(text, font, size, style.flow, style.ligatures)
        }
    }
    fn font(&self, c: char, requested: Font) -> Option<usize> {
        self.fonts
            .iter()
            .position(|(f, _)| *f == requested)
            .filter(|i| glyph_supported(&self.faces[*i], c))
            .or_else(|| self.faces.iter().position(|f| glyph_supported(f, c)))
    }
    fn outline(&mut self, paths: &[Vec<Point>]) -> Result<(), String> {
        self.points = self
            .points
            .saturating_add(paths.iter().map(Vec::len).sum::<usize>());
        if self.points > 512 * 1024 {
            Err("Text outline point budget exceeded; split the text into smaller objects".into())
        } else {
            Ok(())
        }
    }
    fn shape(
        &mut self,
        text: &str,
        font: Font,
        size: f64,
        flow: Flow,
        ligatures: bool,
    ) -> Result<Vec<Token>, String> {
        let chars: Vec<_> = text.char_indices().collect();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let (byte, c) = chars[i];
            if c == '\n' {
                tokens.push(Token::Break);
                i += 1;
                continue;
            }
            if matches!(c, '\r' | '\u{200d}' | '\u{fe0e}' | '\u{fe0f}') {
                i += 1;
                continue;
            }
            let c = if c == '\t' { ' ' } else { c };
            let Some(f) = self.font(c, font) else {
                self.missing += 1;
                self.glyphs += 1;
                if self.missing_characters.len() < 16 && !self.missing_characters.contains(&c) {
                    self.missing_characters.push(c);
                }
                let mut paths = missing_box([0.; 2], size);
                orient(&mut paths, size * 0.7, size, flow, upright(c));
                self.outline(&paths)?;
                tokens.push(Token::Cell(Cell {
                    paths,
                    paint: None,
                    advance: if flow == Flow::Vertical && upright(c) {
                        size
                    } else {
                        size * 0.7
                    },
                }));
                i += 1;
                continue;
            };
            let start = i;
            i += 1;
            while i < chars.len()
                && !matches!(
                    chars[i].1,
                    '\n' | '\r' | '\u{200d}' | '\u{fe0e}' | '\u{fe0f}'
                )
                && self.font(if chars[i].1 == '\t' { ' ' } else { chars[i].1 }, font) == Some(f)
            {
                i += 1;
            }
            let end = chars.get(i).map_or(text.len(), |v| v.0);
            let run = text[byte..end].replace('\t', " ");
            if self.fonts[f].0 != font {
                self.substituted += i - start;
                if !self.fallback_fonts.contains(&self.fonts[f].0) {
                    self.fallback_fonts.push(self.fonts[f].0);
                }
            }
            let face = rustybuzz::Face::from_slice(self.fonts[f].1.data(), self.fonts[f].1.index())
                .ok_or("Shaping font parse failed")?;
            if ligatures
                && !face.tables().gsub.is_some_and(|table| {
                    table
                        .features
                        .into_iter()
                        .any(|v| matches!(&v.tag.to_bytes(), b"liga" | b"clig"))
                })
                && !self.ligature_unavailable.contains(&self.fonts[f].0)
            {
                self.ligature_unavailable.push(self.fonts[f].0);
            }
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&run);
            buffer.guess_segment_properties();
            buffer.set_direction(rustybuzz::Direction::LeftToRight);
            let features = [
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"liga"),
                    u32::from(ligatures),
                    ..,
                ),
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"clig"),
                    u32::from(ligatures),
                    ..,
                ),
                rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(b"kern"), 0, ..),
                rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(b"calt"), 0, ..),
            ];
            let shaped = rustybuzz::shape(&face, &features, buffer);
            let infos = shaped.glyph_infos();
            self.glyphs += infos.len();
            let positions = shaped.glyph_positions();
            let scale = size / f64::from(self.faces[f].units_per_em());
            let mut n = 0;
            while n < infos.len() {
                let first = n;
                let cluster = infos[n].cluster;
                let mut paths = Vec::new();
                let mut advance = 0.;
                while n < infos.len() && infos[n].cluster == cluster {
                    let p = positions[n];
                    let id = ttf_parser::GlyphId(infos[n].glyph_id as u16);
                    let current = if id.0 == 0 {
                        missing_box([advance, 0.], size)
                    } else {
                        crate::render::glyph_paths(
                            &self.faces[f],
                            id,
                            [
                                advance + f64::from(p.x_offset) * scale,
                                -f64::from(p.y_offset) * scale,
                            ],
                            size,
                        )
                    };
                    self.outline(&current)?;
                    paths.extend(current);
                    advance += f64::from(p.x_advance) * scale;
                    n += 1;
                }
                let c = run
                    .get(infos[first].cluster as usize..)
                    .and_then(|v| v.chars().next())
                    .unwrap_or(' ');
                orient(&mut paths, advance, size, flow, upright(c));
                tokens.push(Token::Cell(Cell {
                    paths,
                    paint: None,
                    advance: if flow == Flow::Vertical && upright(c) {
                        size
                    } else {
                        advance.max(0.)
                    },
                }));
            }
        }
        Ok(tokens)
    }
}
fn upright(c: char) -> bool {
    matches!(c as u32,0x1100..=0x11ff|0x2e80..=0xa4cf|0xac00..=0xd7ff|0xf900..=0xfaff|0xff00..=0xffef|0x1f000..=0x1ffff)
}
fn orient(paths: &mut [Vec<Point>], advance: f64, size: f64, flow: Flow, upright: bool) {
    if flow == Flow::Vertical {
        for p in paths.iter_mut().flatten() {
            *p = if upright {
                [p[0] - advance / 2., p[1] + size * 0.8]
            } else {
                [-p[1] - size * 0.35, p[0]]
            };
        }
    }
}
fn translate(paths: &mut [Vec<Point>], flow: Flow, along: f64, cross: f64) {
    for p in paths.iter_mut().flatten() {
        if flow == Flow::Horizontal {
            p[0] += along;
            p[1] += cross;
        } else {
            p[1] += along;
            p[0] += cross;
        }
    }
}
type Packed = (Vec<Vec<Point>>, f64, Vec<PaintRange>);
fn pack(tokens: Vec<Token>, size: f64, tracking: f64, flow: Flow) -> Result<Packed, String> {
    let mut paths = Vec::new();
    let mut paint = vec![];
    let mut cursor = 0.;
    let mut last = 0.0_f64;
    for token in tokens {
        let Token::Cell(mut c) = token else {
            return Err("Reading annotation cannot cross a hard newline".into());
        };
        translate(&mut c.paths, flow, cursor, 0.);
        if let Some(p) = c.paint {
            paint.push(PaintRange {
                start: paths.len(),
                end: paths.len() + c.paths.len(),
                paint: p,
            });
        }
        paths.extend(c.paths);
        last = last.max(cursor + c.advance);
        cursor += (c.advance + size * tracking).max(size * 0.05);
    }
    Ok((paths, last, paint))
}
struct Group {
    paint: Vec<PaintRange>,
    paths: Vec<Vec<Point>>,
    advance: f64,
}
enum GroupToken {
    Group(Group),
    Break,
}
fn align_offset(align: Align, space: f64) -> f64 {
    space
        * match align {
            Align::Left => 0.,
            Align::Center => 0.5,
            Align::Right => 1.,
        }
}
pub(super) fn layout(
    baseline: Point,
    text: &str,
    size: f64,
    style: &Style,
    fonts: &[(Font, Bytes)],
) -> Result<Layout, String> {
    let faces = fonts
        .iter()
        .map(|(_, b)| {
            ttf_parser::Face::parse(b.data(), b.index()).map_err(|e| format!("Font parse: {e:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut engine = Engine {
        fonts,
        faces,
        missing: 0,
        substituted: 0,
        fallback_fonts: Vec::new(),
        missing_characters: Vec::new(),
        ligature_unavailable: Vec::new(),
        points: 0,
        glyphs: 0,
    };
    let offsets: Vec<_> = text
        .char_indices()
        .map(|(n, _)| n)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut groups = Vec::new();
    let mut cursor = 0;
    for annotation in style.ruby.iter().map(Some).chain(std::iter::once(None)) {
        let end = annotation.map_or(offsets.len() - 1, |r| r.start);
        for token in engine.dispatch(
            &text[offsets[cursor]..offsets[end]],
            cursor,
            style.font,
            size,
            style,
            true,
        )? {
            groups.push(match token {
                Token::Break => GroupToken::Break,
                Token::Cell(c) => GroupToken::Group(Group {
                    paint: c
                        .paint
                        .map(|paint| {
                            vec![PaintRange {
                                start: 0,
                                end: c.paths.len(),
                                paint,
                            }]
                        })
                        .unwrap_or_default(),
                    paths: c.paths,
                    advance: c.advance,
                }),
            });
        }
        let Some(r) = annotation else {
            break;
        };
        let (mut body, body_length, mut body_paint) = pack(
            engine.dispatch(
                &text[offsets[r.start]..offsets[r.end]],
                r.start,
                style.font,
                size,
                style,
                true,
            )?,
            size,
            style.tracking,
            style.flow,
        )?;
        let ruby_size = size * r.ratio;
        let (mut reading, reading_length, reading_paint) = pack(
            engine.dispatch(&r.text, 0, r.font, ruby_size, style, false)?,
            ruby_size,
            r.tracking,
            style.flow,
        )?;
        let length = body_length.max(reading_length);
        translate(&mut body, style.flow, (length - body_length) / 2., 0.);
        let cross = match (style.flow, r.side) {
            (Flow::Horizontal, Side::Before) => -size * (1. + r.gap),
            (Flow::Horizontal, Side::After) => size * (0.35 + r.gap) + ruby_size * 0.8,
            (Flow::Vertical, Side::Before) => size * (0.6 + r.gap) + ruby_size * 0.5,
            (Flow::Vertical, Side::After) => -size * (0.6 + r.gap) - ruby_size * 0.5,
        };
        translate(
            &mut reading,
            style.flow,
            align_offset(r.align, length - reading_length),
            cross,
        );
        body_paint.extend(reading_paint.into_iter().map(|mut p| {
            p.start += body.len();
            p.end += body.len();
            p
        }));
        body.extend(reading);
        groups.push(GroupToken::Group(Group {
            paint: body_paint,
            paths: body,
            advance: length,
        }));
        cursor = r.end;
    }
    let mut widths = vec![0.0_f64];
    let mut placements = Vec::new();
    let mut line = 0;
    let mut cursor = 0.;
    let mut has_group = false;
    for token in groups {
        let GroupToken::Group(group) = token else {
            line += 1;
            widths.push(0.);
            cursor = 0.;
            has_group = false;
            continue;
        };
        if style.wrap_width > 0. && has_group && cursor + group.advance > style.wrap_width {
            line += 1;
            widths.push(0.);
            cursor = 0.;
        }
        widths[line] = widths[line].max(cursor + group.advance);
        let start = cursor;
        cursor += (group.advance + size * style.tracking).max(size * 0.05);
        placements.push((group, line, start));
        has_group = true;
    }
    let box_width = if style.wrap_width > 0. {
        style.wrap_width
    } else {
        widths.iter().copied().fold(0., f64::max)
    };
    let mut paths = Vec::new();
    let mut paint = vec![];
    for (mut g, line, start) in placements {
        let along = start + align_offset(style.align, (box_width - widths[line]).max(0.));
        let cross = line as f64
            * size
            * style.line_height
            * if style.flow == Flow::Vertical {
                -1.
            } else {
                1.
            };
        translate(&mut g.paths, style.flow, along, cross);
        for p in g.paths.iter_mut().flatten() {
            p[0] += baseline[0];
            p[1] += baseline[1]
                - if style.flow == Flow::Vertical {
                    size * 0.8
                } else {
                    0.
                };
        }
        paint.extend(g.paint.into_iter().map(|mut p| {
            p.start += paths.len();
            p.end += paths.len();
            p
        }));
        paths.extend(g.paths);
    }
    Ok(Layout {
        paths,
        paint,
        lines: widths.len(),
        glyphs: engine.glyphs,
        unavailable_fonts: style
            .ruby
            .iter()
            .map(|r| r.font)
            .filter(|f| !fonts.iter().any(|(v, _)| v == f))
            .collect(),
        format_unavailable: style.rich.as_ref().map_or_else(Vec::new, |r| {
            let mut requested: Vec<Font> = r.spans.iter().filter_map(|s| s.attr.font).collect();
            if let Some(m) = &r.mapping {
                requested.push(m.base);
                requested.extend(m.rules.iter().map(|r| r.font));
            }
            let mut missing = vec![];
            for f in requested {
                if !fonts.iter().any(|(v, _)| *v == f) && !missing.contains(&f) {
                    missing.push(f);
                }
            }
            missing
        }),
        widths,
        missing: engine.missing,
        substituted: engine.substituted,
        requested_available: fonts.iter().any(|(f, _)| *f == style.font),
        fallback_fonts: engine.fallback_fonts,
        missing_characters: engine.missing_characters,
        ligature_unavailable: engine.ligature_unavailable,
    })
}
