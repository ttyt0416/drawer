use super::*;
use crate::text_format::Paint;
#[derive(Default)]
pub(super) struct Text {
    plain: text_contour_tree::Set,
    color: [u8; 4],
    groups: Vec<(text_contour_tree::Set, Paint)>,
    index: text_contour_tree::Set,
}
impl Text {
    pub fn new(
        baseline: Point,
        text: &str,
        size: f64,
        color: [u8; 4],
        style: Option<&crate::text_layout::Style>,
    ) -> Result<Self, String> {
        let Some(style) = style.filter(|s| s.rich.is_some()) else {
            return Ok(Self {
                plain: text_contour_tree::Set::new(glyph_contours(baseline, text, size, style)?),
                color,
                ..Default::default()
            });
        };
        let layout = crate::text_layout::layout(baseline, text, size, style)?;
        let mut paths = layout.paths.into_iter().enumerate().peekable();
        let mut groups = vec![];
        let mut index = vec![];
        for p in layout.paint {
            let mut current = vec![];
            while paths.peek().is_some_and(|(i, _)| *i < p.end) {
                let (i, path) = paths.next().unwrap();
                if i >= p.start {
                    current.push(path);
                }
            }
            let set = text_contour_tree::Set::new(contour_paths(current));
            if let Some(mut b) = set.bounds() {
                let radius = p.paint.border.map_or(0., |b| b.width / 2.);
                b.min = b.min.map(|v| v - radius);
                b.max = b.max.map(|v| v + radius);
                index.push(Contour {
                    points: vec![],
                    bounds: b,
                });
                groups.push((set, p.paint));
            }
        }
        Ok(Self {
            color,
            groups,
            index: text_contour_tree::Set::new(index),
            ..Default::default()
        })
    }
    pub fn bounds(&self) -> Option<Rect> {
        self.index.bounds().or_else(|| self.plain.bounds())
    }
    pub fn pixel(&self, p: Point) -> [u8; 4] {
        if self.groups.is_empty() {
            return if self.plain.winding(p) != 0 {
                self.color
            } else {
                [0; 4]
            };
        }
        let mut hits = self.index.candidates(p);
        hits.sort_unstable();
        let mut result = [0; 4];
        for i in hits {
            let (set, paint) = &self.groups[i];
            if let Some(border) = paint.border
                && set.near(p, border.width / 2.)
            {
                result = over(result, border.color);
            }
            if set.winding(p) != 0 {
                result = over(result, paint.color.unwrap_or(self.color));
            }
        }
        result
    }
}
