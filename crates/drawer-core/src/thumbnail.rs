use crate::{document::Document, scene::Rect};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Raster(usize),
    Track(usize),
    Folder(usize),
    Vector(u64),
    Source(crate::layer_search::Key),
}
type SourcePixels<'a> = (&'a [[u8; 4]], [u32; 2], [i64; 2]);
fn displayed(document: &Document, target: Target) -> bool {
    match target {
        Target::Raster(n) => {
            crate::appearance::active(document, crate::layer_properties::raster(document, n))
        }
        Target::Vector(id) => {
            crate::appearance::active(document, crate::layer_properties::Target::Vector(id))
        }
        _ => false,
    }
}
fn cache<'a>(
    d: &Document,
    c: &'a crate::appearance::Caches,
    t: Target,
) -> Option<&'a crate::appearance::Cache> {
    match t {
        Target::Raster(n) => c.rasters.get(n)?.as_ref(),
        Target::Vector(id) => c
            .vectors
            .get(d.scene.lines.layers.iter().position(|l| l.id == id)?)?
            .as_ref(),
        _ => None,
    }
}
fn source(document: &Document, target: Target) -> Option<SourcePixels<'_>> {
    match target {
        Target::Raster(n) => document.layers.get(n).map(|layer| {
            let extent = layer.extent(document.width, document.height);
            (
                layer.pixels.as_slice(),
                [extent.width, extent.height],
                extent.offset,
            )
        }),
        Target::Track(n) => document
            .animation
            .as_ref()
            .and_then(|a| a.tracks.get(n))
            .and_then(|track| {
                if document.current_frame >= track.end {
                    return None;
                }
                let n = track
                    .exposures
                    .partition_point(|e| e.frame <= document.current_frame);
                if n == 0 {
                    None
                } else {
                    track
                        .cells
                        .get(track.exposures[n - 1].cell)
                        .map(|c| (c.pixels.as_slice(), [c.width, c.height], c.offset))
                }
            }),
        Target::Folder(_) | Target::Vector(_) | Target::Source(_) => None,
    }
}
pub fn bounds(document: &Document, target: Target, content: bool) -> Rect {
    if let Some(key) = crate::layer_catalog::from_thumbnail(document, target)
        && let Some(l) = document.scene.file_objects.get(key)
    {
        let mut m = l.cache.clone();
        m.transform = m.transform.then(document.scene.transform);
        return m.bounds();
    }
    if content
        && displayed(document, target)
        && let Ok(caches) = crate::appearance::Caches::new(document, None)
        && let Some(rect) = cache(document, &caches, target).and_then(|c| c.bounds())
    {
        return rect;
    }
    if let Target::Source(key) = target {
        return crate::layer_catalog::bounds(document, key).unwrap_or(Rect {
            min: [0.; 2],
            max: [1.; 2],
        });
    }
    if !content {
        return Rect {
            min: [0.0; 2],
            max: [document.width as f64, document.height as f64],
        };
    }
    if let Some((pixels, size, offset)) = source(document, target) {
        let mut min = size;
        let mut max = [0u32; 2];
        for (n, _) in pixels.iter().enumerate().filter(|(_, p)| p[3] > 0) {
            let x = n as u32 % size[0];
            let y = n as u32 / size[0];
            min = [min[0].min(x), min[1].min(y)];
            max = [max[0].max(x + 1), max[1].max(y + 1)];
        }
        if min[0] < max[0] && min[1] < max[1] {
            return Rect {
                min: [
                    offset[0] as f64 + min[0] as f64,
                    offset[1] as f64 + min[1] as f64,
                ],
                max: [
                    offset[0] as f64 + max[0] as f64,
                    offset[1] as f64 + max[1] as f64,
                ],
            };
        }
    }
    Rect {
        min: [0.0; 2],
        max: [1.0; 2],
    }
}
pub fn image(
    document: &Document,
    target: Target,
    size: usize,
    content: bool,
    checker: bool,
) -> Result<Vec<[u8; 4]>, String> {
    if !(1..=128).contains(&size) {
        return Err("Thumbnail size must be from 1 to 128".into());
    }
    let linked = crate::layer_catalog::from_thumbnail(document, target)
        .filter(|k| document.scene.file_objects.get(*k).is_some());
    let target = linked.map_or(target, Target::Source);
    let source = source(document, target);
    let caches = displayed(document, target)
        .then(|| crate::appearance::Caches::new(document, None))
        .transpose()?;
    let appearance = caches.as_ref().and_then(|c| cache(document, c, target));
    let renderer =
        matches!(target, Target::Source(_)).then(|| crate::render::Renderer::new(document));
    let bounds = if content {
        appearance
            .and_then(|c| c.bounds())
            .unwrap_or_else(|| bounds(document, target, content))
    } else {
        bounds(document, target, content)
    };
    let width = bounds.max[0] - bounds.min[0];
    let height = bounds.max[1] - bounds.min[1];
    let scale = (size as f64 / width).min(size as f64 / height);
    let pad = [
        (size as f64 - width * scale) / 2.0,
        (size as f64 - height * scale) / 2.0,
    ];
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let px = (x as f64 + 0.5 - pad[0]) / scale;
            let py = (y as f64 + 0.5 - pad[1]) / scale;
            let value = if px >= 0.0 && py >= 0.0 && px < width && py < height {
                if let Some(cache) = appearance {
                    cache.pixel([
                        (px + bounds.min[0]).floor() as i64,
                        (py + bounds.min[1]).floor() as i64,
                    ])
                } else if let (Target::Source(key), Some(renderer)) = (target, &renderer) {
                    renderer.source_pixel(key, [px + bounds.min[0], py + bounds.min[1]])
                } else {
                    source
                        .and_then(|(pixels, extent, offset)| {
                            let x = (px + bounds.min[0]).floor() as i64 - offset[0];
                            let y = (py + bounds.min[1]).floor() as i64 - offset[1];
                            if x < 0 || y < 0 || x >= extent[0] as i64 || y >= extent[1] as i64 {
                                None
                            } else {
                                pixels.get((y * extent[0] as i64 + x) as usize).copied()
                            }
                        })
                        .unwrap_or([0; 4])
                }
            } else {
                [0; 4]
            };
            let value = if let Target::Raster(n) = target {
                document.scene.paper.layer_pixel(
                    document,
                    n,
                    [
                        (px + bounds.min[0]).floor() as i64,
                        (py + bounds.min[1]).floor() as i64,
                    ],
                    value,
                )
            } else {
                value
            };
            if checker {
                let background = if (x / 4 + y / 4).is_multiple_of(2) {
                    180u8
                } else {
                    220
                };
                let alpha = value[3] as f64 / 255.0;
                pixels.push([
                    (value[0] as f64 * alpha + background as f64 * (1.0 - alpha)).round() as u8,
                    (value[1] as f64 * alpha + background as f64 * (1.0 - alpha)).round() as u8,
                    (value[2] as f64 * alpha + background as f64 * (1.0 - alpha)).round() as u8,
                    255,
                ]);
            } else {
                pixels.push(value);
            }
        }
    }
    Ok(pixels)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Background, CanvasSpec, PlacedRaster};
    #[test]
    fn outside_content_empty_layers_checker_and_modes_preserve_source_pixels() {
        let mut doc = Document::create(
            CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            100,
        )
        .unwrap();
        doc.layers[0].placement = Some(PlacedRaster {
            width: 2,
            height: 1,
            offset: [50, -40],
        });
        doc.layers[0].pixels = vec![[255, 0, 0, 128], [8, 7, 6, 0]];
        let before = doc.snapshot();
        assert!(
            image(&doc, Target::Raster(0), 16, false, false)
                .unwrap()
                .iter()
                .all(|p| *p == [0; 4])
        );
        assert_eq!(
            bounds(&doc, Target::Raster(0), true),
            Rect {
                min: [50.0, -40.0],
                max: [51.0, -39.0]
            }
        );
        assert!(
            image(&doc, Target::Raster(0), 16, true, false)
                .unwrap()
                .iter()
                .all(|p| *p == [255, 0, 0, 128])
        );
        assert!(
            image(&doc, Target::Raster(0), 16, true, true)
                .unwrap()
                .iter()
                .all(|p| p[3] == 255)
        );
        assert_eq!(doc.snapshot(), before);
        assert!(image(&doc, Target::Raster(0), 0, true, false).is_err());
        doc.layers[0].pixels.fill([8, 7, 6, 0]);
        assert_eq!(
            bounds(&doc, Target::Raster(0), true),
            Rect {
                min: [0.0; 2],
                max: [1.0; 2]
            }
        );
        assert!(
            image(&doc, Target::Track(999), 16, true, false)
                .unwrap()
                .iter()
                .all(|p| *p == [0; 4])
        );
    }
}
