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
        let mut segments: Vec<(String, Attr, Option<usize>)> = vec![];
        let mut scalar = start;
        for g in text.graphemes(true) {
            let attr = rich.resolve(g, scalar, font);
            scalar += g.chars().count();
            let covers = |i: usize| {
                g.chars().filter(|c| !ignored(*c)).all(|c| {
                    self.faces[i]
                        .glyph_index(if c == '\t' { ' ' } else { c })
                        .is_some()
                })
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
                && let Some((last, a, f)) = segments.last_mut()
                && *a == attr
                && *f == face
                && !last.ends_with('\n')
            {
                last.push_str(g);
            } else {
                segments.push((g.into(), attr, face));
            }
        }
        let mut tokens = vec![];
        for (run, attr, face) in segments {
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
            let face = rustybuzz::Face::from_slice(self.fonts[f].1.data(), 0)
                .ok_or("Shaping font parse failed")?;
            if style.ligatures
                && !face.tables().gsub.is_some_and(|t| {
                    t.features
                        .into_iter()
                        .any(|f| matches!(&f.tag.to_bytes(), b"liga" | b"clig"))
                })
                && !self.ligature_unavailable.contains(&self.fonts[f].0)
            {
                self.ligature_unavailable.push(self.fonts[f].0);
            }
            let run = run.replace('\t', " ");
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&run);
            buffer.guess_segment_properties();
            buffer.set_direction(rustybuzz::Direction::LeftToRight);
            buffer.set_cluster_level(rustybuzz::BufferClusterLevel::MonotoneGraphemes);
            let features = [
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"liga"),
                    u32::from(style.ligatures),
                    ..,
                ),
                rustybuzz::Feature::new(
                    ttf_parser::Tag::from_bytes(b"clig"),
                    u32::from(style.ligatures),
                    ..,
                ),
                rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(b"kern"), 0, ..),
                rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(b"calt"), 0, ..),
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
                orient(&mut paths, advance, size, style.flow, upright(c));
                tokens.push(Token::Cell(Cell {
                    paths,
                    advance: if style.flow == Flow::Vertical && upright(c) {
                        size
                    } else {
                        advance.max(0.)
                    },
                    paint: Some(paint),
                }));
            }
        }
        Ok(tokens)
    }
}
