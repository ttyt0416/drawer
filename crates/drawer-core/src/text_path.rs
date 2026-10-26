//! A circular view never deletes the underlying multiline text or regular formatting.
use crate::{
    scene::{PageObject, Point},
    text_format::{Paint, PaintRange},
    text_layout::{Flow, Style},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    #[default]
    Outside,
    Inside,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circle {
    pub radius: f64,
    pub angle: f64,
    pub side: Side,
    pub equal: bool,
}
impl Default for Circle {
    fn default() -> Self {
        Self {
            radius: 80.,
            angle: -90.,
            side: Side::Outside,
            equal: false,
        }
    }
}
impl Circle {
    pub fn valid(self) -> bool {
        (0.1..=32768.).contains(&self.radius) && (-360. ..=360.).contains(&self.angle)
    }
    pub fn map(
        self,
        p: Point,
        baseline: Point,
        along: f64,
        advance: f64,
        slot: usize,
        count: usize,
    ) -> Point {
        let sign = if self.side == Side::Outside { 1. } else { -1. };
        let angle = self.angle.to_radians()
            + sign
                * if self.equal {
                    slot as f64 * std::f64::consts::TAU / count.max(1) as f64
                } else {
                    (along + advance / 2.) / self.radius
                };
        let rotation = angle + sign * std::f64::consts::FRAC_PI_2;
        let (s, c) = rotation.sin_cos();
        let x = p[0] - baseline[0] - along - advance / 2.;
        let y = p[1] - baseline[1];
        [
            baseline[0] + self.radius * angle.cos() + c * x - s * y,
            baseline[1] + self.radius * angle.sin() + s * x + c * y,
        ]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Effects {
    pub underline: bool,
    pub strike: bool,
    pub slant: f64,
}
impl Effects {
    pub fn valid(self) -> bool {
        (-60. ..=60.).contains(&self.slant)
    }
}
pub fn hidden(text: &str, s: &Style) -> Vec<String> {
    if s.circle.is_none() {
        return vec![];
    }
    let mut notes = vec![];
    let first = text.split(['\n', '\r']).next().unwrap_or("");
    let rest = text.chars().count() - first.chars().count();
    if rest > 0 {
        notes.push(format!("Circular layout hides {rest} characters after the first explicit line; all Unicode remains saved."));
    }
    if !s.ruby.is_empty() {
        notes.push(format!(
            "{} reading annotations are hidden in circular layout and restored in regular layout.",
            s.ruby.len()
        ));
    }
    if s.effects
        .is_some_and(|e| e.underline || e.strike || e.slant != 0.)
    {
        notes.push(
            "Underline, strikethrough and slant are preserved but hidden in circular layout."
                .into(),
        );
    }
    if s.wrap_width > 0. || s.flow == Flow::Vertical {
        notes.push(
            "Wrapping and vertical flow are preserved but inactive in circular layout.".into(),
        );
    }
    notes
}
pub fn output_notes(d: &crate::document::Document) -> Vec<String> {
    d.scene
        .objects
        .iter()
        .enumerate()
        .flat_map(|(i, o)| match o {
            PageObject::Text {
                text,
                layout: Some(s),
                ..
            } => hidden(text, s)
                .into_iter()
                .map(|n| format!("Text #{}: {n}", d.scene.catalog.objects.id(i)))
                .collect(),
            _ => vec![],
        })
        .collect()
}
pub fn transient(text: &str, style: &Style) -> (String, Style) {
    let first = text.split(['\n', '\r']).next().unwrap_or("").to_owned();
    let n = first.chars().count();
    let mut s = style.clone();
    s.wrap_width = 0.;
    s.flow = Flow::Horizontal;
    s.effects = None;
    s.ruby.clear();
    if let Some(r) = &mut s.rich {
        r.spans.retain_mut(|span| {
            span.end = span.end.min(n);
            span.start < span.end
        });
    }
    if let Some(o) = &mut s.typeset {
        o.horizontal.clear();
        o.justify = false;
        o.last = crate::text_typeset::LastLine::Left;
    }
    (first, s)
}
pub(crate) fn decorate(
    paths: &mut Vec<Vec<Point>>,
    paint: &mut Vec<PaintRange>,
    advance: f64,
    size: f64,
    flow: Flow,
    e: Effects,
) {
    let width = (size / 16.).max(0.5);
    for offset in [
        e.underline.then_some(size * 0.12),
        e.strike.then_some(-size * 0.3),
    ]
    .into_iter()
    .flatten()
    {
        let rect = if flow == Flow::Horizontal {
            vec![
                [0., offset],
                [advance, offset],
                [advance, offset + width],
                [0., offset + width],
            ]
        } else {
            let x = if offset > 0. { size * 0.5 } else { 0. };
            vec![[x, 0.], [x + width, 0.], [x + width, advance], [x, advance]]
        };
        let start = paths.len();
        paths.push(rect);
        paint.push(PaintRange {
            start,
            end: paths.len(),
            paint: Paint::default(),
        });
    }
    let skew = e.slant.to_radians().tan();
    for p in paths.iter_mut().flatten() {
        if flow == Flow::Horizontal {
            p[0] -= skew * p[1];
        } else {
            p[1] += skew * p[0];
        }
    }
}
