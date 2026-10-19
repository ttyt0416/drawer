//! Explicit bounds choices shared by alignment and object snapping.
use crate::{
    alignment::{self, Item, Reference},
    document::Document,
    scene::{Point, Rect},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub text_pixels: bool,
    pub vector_paths: bool,
    pub raster_pixels: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub selected: Vec<Item>,
    pub reference: Reference,
    pub options: Options,
}
impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        let valid = |i: Item| match i {
            Item::Raster(n) => n < 256,
            Item::Object(n) => n < 4096,
            Item::Vector(n) | Item::Gradient(n) => n > 0,
        };
        self.selected.len() <= 8192
            && self.selected.iter().all(|i| valid(*i))
            && (self
                .selected
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.selected.len())
            && match self.reference {
                Reference::Key(i) => valid(i),
                Reference::Guide(n) => n < 4096,
                _ => true,
            }
    }
}
pub fn rectangle(points: impl IntoIterator<Item = Point>) -> Result<Rect, String> {
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for p in points {
        for (k, v) in p.into_iter().enumerate() {
            r.min[k] = r.min[k].min(v);
            r.max[k] = r.max[k].max(v);
        }
    }
    if r.valid() {
        Ok(r)
    } else {
        Err("Reference has no visible finite bounds".into())
    }
}
pub fn bounds(doc: &Document, item: Item, options: Options) -> Result<Rect, String> {
    bounds_budget(doc, item, options, &mut (64 * 1024 * 1024))
}
pub(crate) fn bounds_budget(
    doc: &Document,
    item: Item,
    options: Options,
    remaining: &mut usize,
) -> Result<Rect, String> {
    match item {
        Item::Object(n)
            if options.text_pixels
                && matches!(
                    doc.scene.objects.get(n),
                    Some(crate::scene::PageObject::Text { .. })
                ) =>
        {
            crate::render::text_pixel_bounds_budget(doc, n, remaining)
        }
        Item::Vector(id) if options.vector_paths => {
            let p = doc
                .scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .find(|p| p.id == id)
                .ok_or("Missing vector path")?;
            rectangle(p.points.iter().map(|p| doc.scene.transform.map(*p)))
        }
        Item::Raster(n) if options.raster_pixels => {
            let l = doc.layers.get(n).ok_or("Missing raster layer")?;
            if l.pixels.len() > 4 * 1024 * 1024 {
                return Err("Raster pixel bounds work limit".into());
            }
            *remaining = remaining
                .checked_sub(l.pixels.len())
                .ok_or("Alignment pixel bounds work limit")?;
            let e = l.extent(doc.width, doc.height);
            rectangle(
                l.pixels
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p[3] > 0)
                    .flat_map(|(i, _)| {
                        let x = e.offset[0] as f64 + (i % e.width as usize) as f64;
                        let y = e.offset[1] as f64 + (i / e.width as usize) as f64;
                        [[x, y], [x + 1.0, y + 1.0]]
                    }),
            )
        }
        _ => alignment::bounds(doc, item),
    }
}
