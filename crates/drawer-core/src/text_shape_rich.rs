use super::*;
use crate::text_format::{Attr, Paint, Rich};
use unicode_segmentation::UnicodeSegmentation;
fn ignored(c: char) -> bool {
    matches!(c, '\u{200d}' | '\u{fe0e}' | '\u{fe0f}' | '\r')
}
impl Engine<'_> {
    pub(super) fn formatted(
        &mut self,
        text: &str,
        start: usize,
        font: Font,
        size: f64,
        style: &Style,
        body: bool,
    ) -> Result<Vec<Token>, String> {
        let empty = Rich::default();
        let rich = if body {
            style.rich.as_deref().unwrap_or(&empty)
        } else {
            &empty
        };
        let bidi = unicode_bidi::BidiInfo::new(text, None);
        type Segment = (
            String,
            Attr,
            Option<usize>,
            unicode_bidi::Level,
            unicode_bidi::Level,
            Option<unicode_script::Script>,
        );
        let mut segments: Vec<Segment> = vec![];
        let mut scalar = start;
        for (byte, g) in text.grapheme_indices(true) {
            let level = if style.typeset.is_none() {
                unicode_bidi::Level::ltr()
            } else if body {
                self.levels
                    .get(scalar)
                    .copied()
                    .unwrap_or(unicode_bidi::Level::ltr())
            } else {
                bidi.levels
                    .get(byte)
                    .copied()
                    .unwrap_or(unicode_bidi::Level::ltr())
            };
            let base = if body {
                self.bases
                    .get(scalar)
                    .copied()
                    .unwrap_or(unicode_bidi::Level::ltr())
            } else {
                bidi.paragraphs
                    .iter()
                    .find(|p| p.range.contains(&byte))
                    .map_or(unicode_bidi::Level::ltr(), |p| p.level)
            };
            let script = g
                .chars()
                .map(|c| unicode_script::UnicodeScript::script(&c))
                .find(|s| {
                    !matches!(
                        s,
                        unicode_script::Script::Common | unicode_script::Script::Inherited
                    )
                });
            let attr = rich.resolve(g, scalar, font);
            scalar += g.chars().count();
            if g.chars().all(|c|ignored(c)||(style.typeset.is_some()&&matches!(c,'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))) {
                continue;
            }
            let covers = |i: usize| {
                g.chars()
                    .filter(|c| !ignored(*c))
                    .all(|c| glyph_supported(&self.faces[i], if c == '\t' { ' ' } else { c }))
            };
            let requested = attr.font.unwrap_or(font);
            let face = self
                .fonts
                .iter()
                .position(|(f, _)| *f == requested)
                .filter(|i| covers(*i))
                .or_else(|| (0..self.faces.len()).find(|i| covers(*i)));
            if g != "\n"
                && g != "\r\n"
                && let Some((last, a, f, l, _, last_script)) = segments.last_mut()
                && *l == level
                && (!style.typeset.as_ref().is_some_and(|o| o.half_punctuation)
                    || (!g.chars().any(crate::text_typeset::punctuation)
                        && !last
                            .chars()
                            .last()
                            .is_some_and(crate::text_typeset::punctuation)))
                && (style.typeset.is_none()
                    || script.is_none()
                    || last_script.is_none()
                    || *last_script == script)
                && *a == attr
                && *f == face
                && !last.ends_with('\n')
            {
                *last_script = last_script.or(script);
                last.push_str(g);
            } else {
                segments.push((g.into(), attr, face, level, base, script));
            }
        }
        let mut tokens = vec![];
        for (run, attr, face, level, base, _) in segments {
            let token_start = tokens.len();
            if run == "\n" || run == "\r\n" {
                tokens.push(Token::Break);
                continue;
            }
            let size = size * attr.ratio.unwrap_or(1.);
            let paint = Paint {
                color: attr.color,
                border: attr.border,
            };
            let Some(f) = face else {
                for g in run.graphemes(true) {
                    if g.chars().all(ignored) {
                        continue;
                    }
                    let count = g.chars().filter(|c| !ignored(*c)).count();
                    self.missing += count;
                    self.glyphs += 1;
                    for c in g.chars().filter(|c| !ignored(*c)) {
                        if self.missing_characters.len() < 16
                            && !self.missing_characters.contains(&c)
                        {
                            self.missing_characters.push(c);
                        }
                    }
                    let mut paths = missing_box([0.; 2], size);
                    orient(
                        &mut paths,
                        size * 0.7,
                        size,
                        style.flow,
                        upright(g.chars().next().unwrap_or(' ')),
                    );
                    self.outline(&paths)?;
                    tokens.push(Token::Cell(Cell {
                        paths,
                        advance: size * 0.7,
                        paint: Some(paint),
                        level,
                        ranges: vec![],
                        base,
                        whitespace: g.chars().all(char::is_whitespace),
                    }));
                }
                continue;
            };
            let requested = attr.font.unwrap_or(font);
            if self.fonts[f].0 != requested {
                self.substituted += run.chars().filter(|c| !ignored(*c)).count();
                if !self.fallback_fonts.contains(&self.fonts[f].0) {
                    self.fallback_fonts.push(self.fonts[f].0);
                }
            }
            let face = rustybuzz::Face::from_slice(self.fonts[f].1.data(), self.fonts[f].1.index())
                .ok_or("Shaping font parse failed")?;
            let options = style.typeset.as_deref();
            let ligatures = style.ligatures
                || options.is_some_and(|o| o.feature(crate::text_typeset::Feature::Ligatures));
            if ligatures
                && !face.tables().gsub.is_some_and(|t| {
                    t.features
                        .into_iter()
                        .any(|f| matches!(&f.tag.to_bytes(), b"liga" | b"clig"))
                })
                && !self.ligature_unavailable.contains(&self.fonts[f].0)
            {
                self.ligature_unavailable.push(self.fonts[f].0);
            }
            if let Some(o) = options {
                if o.feature(crate::text_typeset::Feature::ContextualAlternates)
                    && !face.tables().gsub.is_some_and(|t| {
                        t.features.into_iter().any(|f| &f.tag.to_bytes() == b"calt")
                    })
                {
                    let note = format!(
                        "Contextual alternates unavailable: {}",
                        self.fonts[f].0.label()
                    );
                    if !self.feature_unavailable.contains(&note) {
                        self.feature_unavailable.push(note);
                    }
                }
                if o.kerning == crate::text_typeset::Kerning::Metrics
                    && !face.tables().gpos.is_some_and(|t| {
                        t.features.into_iter().any(|f| &f.tag.to_bytes() == b"kern")
                    })
                    && face.tables().kern.is_none()
                {
                    let note = format!("Metric kerning unavailable: {}", self.fonts[f].0.label());
                    if !self.feature_unavailable.contains(&note) {
                        self.feature_unavailable.push(note);
                    }
                }
            }
            let run = run.replace('\t', " ");
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&run);
            buffer.guess_segment_properties();
            buffer.set_direction(if level.is_rtl() {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            buffer.set_cluster_level(rustybuzz::BufferClusterLevel::MonotoneGraphemes);
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
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"kern"),
                    u32::from(
                        options.is_some_and(|o| o.kerning == crate::text_typeset::Kerning::Metrics),
                    ),
                    ..,
                ),
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"calt"),
                    u32::from(options.is_some_and(|o| {
                        o.feature(crate::text_typeset::Feature::ContextualAlternates)
                    })),
                    ..,
                ),
            ];
            let glyphs = rustybuzz::shape(&face, &features, buffer);
            let infos = glyphs.glyph_infos();
            let positions = glyphs.glyph_positions();
            let factor = size / f64::from(self.faces[f].units_per_em());
            self.glyphs += infos.len();
            let mut n = 0;
            while n < infos.len() {
                let cluster = infos[n].cluster;
                let mut paths = vec![];
                let mut advance = 0.;
                while n < infos.len() && infos[n].cluster == cluster {
                    let p = positions[n];
                    let current = crate::render::glyph_paths(
                        &self.faces[f],
                        ttf_parser::GlyphId(infos[n].glyph_id as u16),
                        [
                            advance + f64::from(p.x_offset) * factor,
                            -f64::from(p.y_offset) * factor,
                        ],
                        size,
                    );
                    self.outline(&current)?;
                    paths.extend(current);
                    advance += f64::from(p.x_advance) * factor;
                    n += 1;
                }
                let c = run
                    .get(cluster as usize..)
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                if options.is_some_and(|o| o.half_punctuation)
                    && matches!(
                        c,
                        '、' | '。'
                            | '「'
                            | '」'
                            | '『'
                            | '』'
                            | '（'
                            | '）'
                            | '［'
                            | '］'
                            | '｛'
                            | '｝'
                            | '！'
                            | '？'
                            | '：'
                            | '；'
                    )
                {
                    let length = advance.min(size * 0.5);
                    for p in paths.iter_mut().flatten() {
                        p[0] -= (advance - length) / 2.;
                    }
                    advance = length;
                }
                orient(&mut paths, advance, size, style.flow, upright(c));
                tokens.push(Token::Cell(Cell {
                    paths,
                    advance: if style.flow == Flow::Vertical && upright(c) {
                        if options.is_some_and(|o| o.half_punctuation)
                            && matches!(
                                c,
                                '、' | '。'
                                    | '「'
                                    | '」'
                                    | '『'
                                    | '』'
                                    | '（'
                                    | '）'
                                    | '［'
                                    | '］'
                                    | '｛'
                                    | '｝'
                                    | '！'
                                    | '？'
                                    | '：'
                                    | '；'
                            )
                        {
                            size * 0.5
                        } else {
                            size
                        }
                    } else {
                        advance.max(0.)
                    },
                    paint: Some(paint),
                    level,
                    ranges: vec![],
                    base,
                    whitespace: c.is_whitespace(),
                }));
            }
            if level.is_rtl() {
                tokens[token_start..].reverse();
            }
        }
        Ok(tokens)
    }
}
