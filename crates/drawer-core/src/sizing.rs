use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, MAX_DIMENSION, PlacedRaster, RasterLayer},
    scene::{Affine, Rect, Scene},
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentState {
    pub color: crate::color_storage::Config,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub layers: Vec<RasterLayer>,
    pub scene: Scene,
    pub animation: Option<crate::animation::Timeline>,
}
impl DocumentState {
    pub fn memory_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(RasterLayer::memory_bytes)
            .sum::<usize>()
            + self.color.memory_bytes()
            + self.scene.size_bytes()
            + self.animation.as_ref().map_or(0, |a| a.memory_bytes())
    }
    pub(crate) fn restore(&self, document: &mut Document) {
        document.color = self.color.clone();
        document.width = self.width;
        document.height = self.height;
        document.dpi = self.dpi;
        document.layers = self.layers.clone();
        document.scene = self.scene.clone();
        document.animation = self.animation.clone();
        document.current_frame = document
            .animation
            .as_ref()
            .map_or(0, |a| document.current_frame.min(a.frames - 1));
    }
}
impl Document {
    pub fn snapshot(&self) -> DocumentState {
        DocumentState {
            color: self.color.clone(),
            width: self.width,
            height: self.height,
            dpi: self.dpi,
            layers: self.layers.clone(),
            scene: self.scene.clone(),
            animation: self.animation.clone(),
        }
    }
}
pub use crate::interpolation::Filter as Interpolation;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Resample {
        width: u32,
        height: u32,
        dpi: f64,
        filter: Interpolation,
    },
    Dpi(f64),
    Canvas {
        width: u32,
        height: u32,
        anchor: [f64; 2],
    },
    CanvasEdges {
        left: i64,
        top: i64,
        right: i64,
        bottom: i64,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    FlipHorizontal,
    FlipVertical,
    Rotate90 {
        clockwise: bool,
    },
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub offset: [i64; 2],
    pub removed_pixels: u64,
    pub clipped_objects: Vec<usize>,
    pub source_boundary: Rect,
}
fn projected(
    source: &DocumentState,
    layer: &RasterLayer,
    change: Change,
    width: u32,
    height: u32,
) -> Result<Option<PlacedRaster>, String> {
    let Some(mut p) = layer.placement else {
        return Ok(None);
    };
    match change {
        Change::Canvas { .. } | Change::CanvasEdges { .. } | Change::Crop { .. } => {
            return Ok(None);
        }
        Change::Dpi(_) => {}
        Change::Resample { .. } => {
            let scale = [
                width as f64 / source.width as f64,
                height as f64 / source.height as f64,
            ];
            let w = (p.width as f64 * scale[0]).round().max(1.0);
            let h = (p.height as f64 * scale[1]).round().max(1.0);
            if w > MAX_DIMENSION as f64 || h > MAX_DIMENSION as f64 {
                return Err(
                    "A placed raster exceeds the supported dimensions after resampling.".into(),
                );
            }
            p.width = w as u32;
            p.height = h as u32;
            p.offset = [
                (p.offset[0] as f64 * scale[0]).round() as i64,
                (p.offset[1] as f64 * scale[1]).round() as i64,
            ];
        }
        Change::FlipHorizontal => p.offset[0] = source.width as i64 - p.offset[0] - p.width as i64,
        Change::FlipVertical => p.offset[1] = source.height as i64 - p.offset[1] - p.height as i64,
        Change::Rotate90 { clockwise: true } => {
            p.offset = [
                source.height as i64 - p.offset[1] - p.height as i64,
                p.offset[0],
            ];
            std::mem::swap(&mut p.width, &mut p.height);
        }
        Change::Rotate90 { clockwise: false } => {
            p.offset = [
                p.offset[1],
                source.width as i64 - p.offset[0] - p.width as i64,
            ];
            std::mem::swap(&mut p.width, &mut p.height);
        }
    }
    if !p.valid() {
        return Err("Placed raster coordinates exceed the supported range.".into());
    }
    Ok(Some(p))
}
pub fn plan(source: &DocumentState, change: Change, budget: usize) -> Result<Plan, String> {
    if !(1..=MAX_DIMENSION).contains(&source.width)
        || !(1..=MAX_DIMENSION).contains(&source.height)
        || source.layers.is_empty()
        || !source.dpi.is_finite()
        || source.dpi <= 0.0
    {
        return Err("Invalid source image.".into());
    }
    let (width, height, dpi, offset) = match change {
        Change::Resample {
            width, height, dpi, ..
        } => (width, height, dpi, [0, 0]),
        Change::Dpi(dpi) => (source.width, source.height, dpi, [0, 0]),
        Change::FlipHorizontal | Change::FlipVertical => {
            (source.width, source.height, source.dpi, [0, 0])
        }
        Change::Rotate90 { .. } => (source.height, source.width, source.dpi, [0, 0]),
        Change::Canvas {
            width,
            height,
            anchor,
        } => {
            if anchor.iter().any(|v| ![0.0, 0.5, 1.0].contains(v)) {
                return Err("Choose one of the nine canvas anchors.".into());
            }
            (
                width,
                height,
                source.dpi,
                [
                    ((width as f64 - source.width as f64) * anchor[0]).round() as i64,
                    ((height as f64 - source.height as f64) * anchor[1]).round() as i64,
                ],
            )
        }
        Change::Crop {
            x,
            y,
            width,
            height,
        } => {
            if x.checked_add(width).is_none_or(|r| r > source.width)
                || y.checked_add(height).is_none_or(|b| b > source.height)
            {
                return Err("The crop must be inside the image.".into());
            }
            (width, height, source.dpi, [-i64::from(x), -i64::from(y)])
        }
        Change::CanvasEdges {
            left,
            top,
            right,
            bottom,
        } => {
            if [left, top, right, bottom]
                .iter()
                .any(|n| n.unsigned_abs() > u64::from(MAX_DIMENSION) * 2)
            {
                return Err("Edge increment exceeds the supported size.".into());
            }
            let width = i64::from(source.width)
                .checked_add(left)
                .and_then(|n| n.checked_add(right))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid width after edge increments.")?;
            let height = i64::from(source.height)
                .checked_add(top)
                .and_then(|n| n.checked_add(bottom))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid height after edge increments.")?;
            (width, height, source.dpi, [left, top])
        }
    };
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err("Image dimensions must be from 1 to 32768 pixels.".into());
    }
    if !dpi.is_finite() || dpi <= 0.0 {
        return Err("DPI must be positive and finite.".into());
    }
    if !source.scene.valid() || source.animation.as_ref().is_some_and(|a| !a.valid()) {
        return Err("Invalid page geometry.".into());
    }
    let mut result_bytes = source.scene.size_bytes();
    for layer in &source.layers {
        if !layer.valid(source.width, source.height) {
            return Err("Invalid raster data.".into());
        }
        let extent = projected(source, layer, change, width, height)?.unwrap_or(PlacedRaster {
            width,
            height,
            offset: [0; 2],
        });
        result_bytes = result_bytes
            .checked_add(
                extent.width as usize
                    * extent.height as usize
                    * if source.color.depth == crate::color_precision::Depth::Sixteen {
                        12
                    } else {
                        4
                    },
            )
            .ok_or("Image size overflow.")?;
    }
    if let Some(animation) = &source.animation {
        result_bytes = result_bytes.saturating_add(
            animation.memory_bytes()
                - animation
                    .tracks
                    .iter()
                    .flat_map(|t| &t.cells)
                    .map(|c| c.pixels.len() * 4)
                    .sum::<usize>(),
        );
        for cell in animation.tracks.iter().flat_map(|t| &t.cells) {
            let layer = RasterLayer {
                precise: None,
                locked: false,
                name: cell.name.clone(),
                pixels: Vec::new(),
                placement: Some(PlacedRaster {
                    width: cell.width,
                    height: cell.height,
                    offset: cell.offset,
                }),
            };
            let extent =
                projected(source, &layer, change, width, height)?.unwrap_or(PlacedRaster {
                    width,
                    height,
                    offset: [0; 2],
                });
            result_bytes = result_bytes
                .checked_add(extent.width as usize * extent.height as usize * 4)
                .ok_or("Animation size overflow")?;
        }
    }
    let history_bytes = source
        .memory_bytes()
        .checked_add(result_bytes)
        .ok_or("Image size overflow.")?;
    if history_bytes > DEFAULT_DOCUMENT_BUDGET / 4 || history_bytes.saturating_mul(2) > budget {
        return Err("This change exceeds the available memory / undo budget. Reduce the dimensions or close other drawings.".into());
    }
    let source_boundary = match change {
        Change::Canvas { .. } | Change::CanvasEdges { .. } | Change::Crop { .. } => Rect {
            min: [-offset[0] as f64, -offset[1] as f64],
            max: [
                width as f64 - offset[0] as f64,
                height as f64 - offset[1] as f64,
            ],
        },
        _ => Rect {
            min: [0.0, 0.0],
            max: [source.width as f64, source.height as f64],
        },
    };
    let mut removed_pixels = source
        .layers
        .iter()
        .map(|layer| {
            if layer.placement.is_some()
                && !matches!(
                    change,
                    Change::Canvas { .. } | Change::CanvasEdges { .. } | Change::Crop { .. }
                )
            {
                return 0;
            }
            let p = layer.extent(source.width, source.height);
            let bounds = Rect {
                min: [p.offset[0] as f64, p.offset[1] as f64],
                max: [
                    p.offset[0] as f64 + p.width as f64,
                    p.offset[1] as f64 + p.height as f64,
                ],
            };
            let kept = source_boundary.intersection(bounds);
            u64::from(p.width) * u64::from(p.height)
                - ((kept.max[0] - kept.min[0]) * (kept.max[1] - kept.min[1])) as u64
        })
        .sum::<u64>();
    if matches!(
        change,
        Change::Canvas { .. } | Change::CanvasEdges { .. } | Change::Crop { .. }
    ) && let Some(animation) = &source.animation
    {
        for cell in animation.tracks.iter().flat_map(|track| &track.cells) {
            let bounds = Rect {
                min: [cell.offset[0] as f64, cell.offset[1] as f64],
                max: [
                    cell.offset[0] as f64 + cell.width as f64,
                    cell.offset[1] as f64 + cell.height as f64,
                ],
            };
            let kept = source_boundary.intersection(bounds);
            removed_pixels += u64::from(cell.width) * u64::from(cell.height)
                - ((kept.max[0] - kept.min[0]) * (kept.max[1] - kept.min[1])) as u64;
        }
    }
    Ok(Plan {
        width,
        height,
        dpi,
        offset,
        removed_pixels,
        clipped_objects: source.scene.clipped_objects(source_boundary),
        source_boundary,
    })
}
pub fn prepare(
    source: &DocumentState,
    change: Change,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<DocumentState, String> {
    let plan = plan(source, change, budget)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Change canceled.".into());
    }
    let mut result = DocumentState {
        color: source.color.clone(),
        width: plan.width,
        height: plan.height,
        dpi: plan.dpi,
        layers: Vec::new(),
        scene: source.scene.clone(),
        animation: source.animation.clone(),
    };
    let scale = [
        plan.width as f64 / source.width as f64,
        plan.height as f64 / source.height as f64,
    ];
    for layer in &source.layers {
        if let Some(placement) = projected(source, layer, change, plan.width, plan.height)? {
            let extent = layer.placement.unwrap();
            let local = DocumentState {
                color: source.color.clone(),
                width: extent.width,
                height: extent.height,
                dpi: source.dpi,
                layers: vec![RasterLayer {
                    placement: None,
                    ..layer.clone()
                }],
                scene: Default::default(),
                animation: None,
            };
            let local_change = if let Change::Resample { dpi, filter, .. } = change {
                Change::Resample {
                    width: placement.width,
                    height: placement.height,
                    dpi,
                    filter,
                }
            } else {
                change
            };
            let mut local_result = prepare(&local, local_change, budget, cancel)?;
            let mut output = local_result.layers.remove(0);
            output.placement = Some(placement);
            result.layers.push(output);
            continue;
        }
        if !layer.valid(source.width, source.height) {
            return Err("Invalid raster data.".into());
        }
        let floating = source.color != crate::color_storage::Config::legacy();
        let count = plan.width as usize * plan.height as usize;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| "Not enough image memory.")?;
        let mut precise = (floating
            && source.color.depth == crate::color_precision::Depth::Sixteen)
            .then(|| Vec::with_capacity(count));
        for y in 0..plan.height {
            if cancel.load(Ordering::Relaxed) {
                return Err("Change canceled.".into());
            }
            for x in 0..plan.width {
                let (sx, sy) = match change {
                    Change::FlipHorizontal => {
                        (i64::from(source.width) - 1 - i64::from(x), i64::from(y))
                    }
                    Change::FlipVertical => {
                        (i64::from(x), i64::from(source.height) - 1 - i64::from(y))
                    }
                    Change::Rotate90 { clockwise: true } => {
                        (i64::from(y), i64::from(source.height) - 1 - i64::from(x))
                    }
                    Change::Rotate90 { clockwise: false } => {
                        (i64::from(source.width) - 1 - i64::from(y), i64::from(x))
                    }
                    _ => (x as i64 - plan.offset[0], y as i64 - plan.offset[1]),
                };
                let point = [
                    (x as f64 + 0.5) / scale[0] - 0.5,
                    (y as f64 + 0.5) / scale[1] - 0.5,
                ];
                if floating {
                    let color = if let Change::Resample { filter, .. } = change {
                        crate::interpolation::sample_float(
                            point,
                            crate::interpolation::box_footprint(
                                point,
                                [1. / scale[0], 1. / scale[1]],
                            ),
                            filter,
                            |px, py| {
                                layer.sample(
                                    source.width,
                                    source.height,
                                    px.clamp(0, source.width as i64 - 1),
                                    py.clamp(0, source.height as i64 - 1),
                                )
                            },
                        )
                    } else {
                        layer.sample(source.width, source.height, sx, sy)
                    };
                    let stored = color.store(
                        source.color.depth,
                        source.color.dither,
                        [x as i64, y as i64],
                    );
                    if let Some(high) = &mut precise {
                        high.push(stored);
                        pixels.push(crate::color_precision::Sample::from16(stored).rgba8());
                    } else {
                        pixels.push(stored.map(|v| v as u8));
                    }
                } else {
                    pixels.push(if let Change::Resample { filter, .. } = change {
                        sample(
                            layer,
                            source.width,
                            source.height,
                            point,
                            filter,
                            [1. / scale[0], 1. / scale[1]],
                        )
                    } else {
                        layer.pixel(source.width, source.height, sx, sy)
                    });
                }
            }
        }
        result.layers.push(RasterLayer {
            precise,
            locked: layer.locked,
            name: layer.name.clone(),
            pixels,
            placement: None,
        });
    }
    let transform = match change {
        Change::Resample { .. } => Affine([scale[0], 0.0, 0.0, scale[1], 0.0, 0.0]),
        Change::FlipHorizontal => Affine([-1.0, 0.0, 0.0, 1.0, source.width as f64, 0.0]),
        Change::FlipVertical => Affine([1.0, 0.0, 0.0, -1.0, 0.0, source.height as f64]),
        Change::Rotate90 { clockwise: true } => {
            Affine([0.0, 1.0, -1.0, 0.0, source.height as f64, 0.0])
        }
        Change::Rotate90 { clockwise: false } => {
            Affine([0.0, -1.0, 1.0, 0.0, 0.0, source.width as f64])
        }
        _ => Affine([
            1.0,
            0.0,
            0.0,
            1.0,
            plan.offset[0] as f64,
            plan.offset[1] as f64,
        ]),
    };
    if !matches!(change, Change::Dpi(_))
        && let Some(animation) = &mut result.animation
    {
        for cell in animation.tracks.iter_mut().flat_map(|t| &mut t.cells) {
            let cell_source = DocumentState {
                color: source.color.clone(),
                width: source.width,
                height: source.height,
                dpi: source.dpi,
                layers: vec![RasterLayer {
                    precise: None,
                    name: cell.name.clone(),
                    locked: false,
                    pixels: cell.pixels.as_ref().clone(),
                    placement: Some(PlacedRaster {
                        width: cell.width,
                        height: cell.height,
                        offset: cell.offset,
                    }),
                }],
                scene: Default::default(),
                animation: None,
            };
            let mut prepared = prepare(&cell_source, change, budget, cancel)?;
            let output = prepared.layers.remove(0);
            let extent = output.extent(plan.width, plan.height);
            cell.width = extent.width;
            cell.height = extent.height;
            cell.offset = extent.offset;
            cell.pixels = std::sync::Arc::new(output.pixels);
        }
    }
    if matches!(change, Change::Crop { .. }) {
        let inverse = source.scene.transform.inverse();
        let boundary = plan.source_boundary;
        result.scene.clips.push(
            [
                boundary.min,
                [boundary.max[0], boundary.min[1]],
                boundary.max,
                [boundary.min[0], boundary.max[1]],
            ]
            .map(|p| inverse.map(p)),
        );
    }
    result.scene.transform = result.scene.transform.then(transform);
    let translation = transform.map(source.scene.selection_translation);
    let origin = transform.map([0.0; 2]);
    result.scene.selection_translation = [translation[0] - origin[0], translation[1] - origin[1]];
    result.scene.selections.transform_translation(transform);
    if !result.scene.valid() {
        return Err("The resulting geometry exceeds the supported coordinate range.".into());
    }
    Ok(result)
}
fn sample(
    layer: &RasterLayer,
    width: u32,
    height: u32,
    point: [f64; 2],
    filter: Interpolation,
    step: [f64; 2],
) -> [u8; 4] {
    crate::interpolation::sample(
        point,
        crate::interpolation::box_footprint(point, step),
        filter,
        |x, y| {
            layer.pixel(
                width,
                height,
                x.clamp(0, i64::from(width) - 1),
                y.clamp(0, i64::from(height) - 1),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        files,
        render::Renderer,
        scene::{Camera, PageObject, WorldMesh},
        test_support::TempDir,
    };
    fn fixture() -> Document {
        let mut doc = Document::create(
            CanvasSpec {
                width: 120,
                height: 80,
                dpi: 300.0,
                background: Background::Transparent,
            },
            100000,
        )
        .unwrap();
        doc.layers[0].pixels[0] = [255, 0, 0, 255];
        doc.layers[0].pixels[119] = [0, 255, 0, 255];
        doc.layers[0].pixels[120 * 79] = [0, 0, 255, 255];
        doc.layers[0].pixels[120 * 80 - 1] = [255, 255, 0, 255];
        doc.scene.objects = vec![
            PageObject::Text {
                baseline: [2.0, 38.0],
                text: "Hi".into(),
                size_px: 10.0 * 300.0 / 72.0,
                color: [200, 10, 40, 255],
            },
            PageObject::Line {
                start: [0.0, 45.0],
                end: [120.0, 45.0],
                width_px: 10.0,
                color: [10, 90, 180, 255],
            },
            PageObject::Tone {
                bounds: Rect {
                    min: [0.0, 50.0],
                    max: [120.0, 80.0],
                },
                period_px: 300.0 / 60.0,
                coverage: 0.25,
            },
            PageObject::Shadow {
                bounds: Rect {
                    min: [60.0, 2.0],
                    max: [75.0, 10.0],
                },
                offset: [10.0, 10.0],
                radius_px: 10.0,
                color: [40, 20, 0, 180],
            },
        ];
        doc.scene.selection = Some(Rect {
            min: [10.0, 5.0],
            max: [50.0, 60.0],
        });
        doc.scene.guides = vec![[[15.0, 0.0], [15.0, 80.0]]];
        doc.scene.camera = Some(Camera {
            page_bounds: Rect {
                min: [85.0, 0.0],
                max: [120.0, 40.0],
            },
            position: [0.0, 0.0, 10.0],
            pixels_per_unit: 10.0,
        });
        doc.scene.worlds = vec![WorldMesh {
            vertices: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 1.0], [0.0, 1.0, 0.0]],
            edges: vec![[0, 1], [1, 2], [2, 0]],
            color: [0, 120, 30, 255],
        }];
        doc
    }
    fn prepared(doc: &Document, change: Change) -> DocumentState {
        prepare(
            &doc.snapshot(),
            change,
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    #[test]
    fn all_nine_anchors_signed_edges_save_and_undo_keep_expected_corner_pixels() {
        let root = TempDir::new();
        let mut doc = Document::create(
            CanvasSpec {
                width: 4,
                height: 3,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1000,
        )
        .unwrap();
        for (n, p) in doc.layers[0].pixels.iter_mut().enumerate() {
            *p = [n as u8, 0, 0, 255];
        }
        let source = doc.snapshot();
        for w in [2, 6] {
            for h in [1, 5] {
                for ax in [0.0, 0.5, 1.0] {
                    for ay in [0.0, 0.5, 1.0] {
                        let change = Change::Canvas {
                            width: w,
                            height: h,
                            anchor: [ax, ay],
                        };
                        let output = prepared(&doc, change);
                        let ox = ((w as f64 - 4.0) * ax).round() as i64;
                        let oy = ((h as f64 - 3.0) * ay).round() as i64;
                        for y in 0..h {
                            for x in 0..w {
                                let sx = x as i64 - ox;
                                let sy = y as i64 - oy;
                                let expected = if (0..4).contains(&sx) && (0..3).contains(&sy) {
                                    source.layers[0].pixels[(sy * 4 + sx) as usize]
                                } else {
                                    [0; 4]
                                };
                                assert_eq!(output.layers[0].pixels[(y * w + x) as usize], expected);
                            }
                        }
                    }
                }
            }
        }
        let edges = Change::CanvasEdges {
            left: -1,
            top: -1,
            right: 2,
            bottom: 1,
        };
        let output = prepared(&doc, edges);
        assert_eq!((output.width, output.height), (5, 3));
        assert_eq!(output.layers[0].pixels[0], source.layers[0].pixels[5]);
        assert_eq!(output.layers[0].pixels[4], [0; 4]);
        doc.commit_state(output).unwrap();
        assert_eq!(doc.history_counts(), (1, 0));
        let path = root.0.join("edges.drawer");
        files::save(&mut doc, &path).unwrap();
        assert_eq!(
            files::open(&path, 10000).unwrap().snapshot(),
            doc.snapshot()
        );
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), source);
        assert!(
            plan(
                &source,
                Change::CanvasEdges {
                    left: -4,
                    top: 0,
                    right: 0,
                    bottom: 0
                },
                10000
            )
            .is_err()
        );
        assert!(
            plan(
                &source,
                Change::CanvasEdges {
                    left: i64::MIN,
                    top: 0,
                    right: i64::MAX,
                    bottom: 0
                },
                10000
            )
            .is_err()
        );
    }
    #[test]
    fn mixed_geometry_resamples_without_rewriting_original_properties_and_round_trips() {
        let root = TempDir::new();
        for (width, height) in [(240, 160), (240, 120)] {
            let mut doc = fixture();
            let before = doc.snapshot();
            let changed = prepared(
                &doc,
                Change::Resample {
                    width,
                    height,
                    dpi: 300.0,
                    filter: Interpolation::Nearest,
                },
            );
            let scale = [width as f64 / 120.0, height as f64 / 80.0];
            let original_renderer = Renderer::new(&doc);
            let expected: Vec<_> = (0..80)
                .flat_map(|y| (0..120).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                .map(|p| original_renderer.pixel(p))
                .collect();
            assert_eq!(changed.scene.objects, before.scene.objects);
            assert_eq!(changed.scene.selection, before.scene.selection);
            assert_eq!(changed.scene.guides, before.scene.guides);
            assert_eq!(changed.scene.camera, before.scene.camera);
            assert_eq!(changed.scene.worlds, before.scene.worlds);
            assert_eq!(
                changed.scene.transform.map([15.0, 30.0]),
                [15.0 * scale[0], 30.0 * scale[1]]
            );
            doc.commit_state(changed).unwrap();
            let renderer = Renderer::new(&doc);
            for y in 0..80 {
                for x in 0..120 {
                    let p = [(x as f64 + 0.5) * scale[0], (y as f64 + 0.5) * scale[1]];
                    assert_eq!(renderer.pixel(p), expected[y * 120 + x], "{x},{y}");
                }
            }
            assert_eq!(
                doc.scene.physical_points(10.0 * 300.0 / 72.0, 300.0),
                [10.0 * scale[0], 10.0 * scale[1]]
            );
            let path = root.0.join("mixed.drawer");
            files::save(&mut doc, &path).unwrap();
            let reopened = files::open(&path, DEFAULT_DOCUMENT_BUDGET).unwrap();
            assert_eq!(doc.snapshot(), reopened.snapshot());
            assert_eq!(
                Renderer::new(&doc).output(300.0, 1000000).unwrap(),
                Renderer::new(&reopened).output(300.0, 1000000).unwrap()
            );
            assert_eq!(doc.history_counts(), (1, 0));
            assert!(doc.undo());
            assert_eq!(doc.snapshot(), before);
            assert!(doc.redo());
            assert_eq!(doc.snapshot(), reopened.snapshot());
        }
    }
    #[test]
    fn dpi_only_keeps_pixels_geometry_and_tone_period_and_output_uses_physical_resolution() {
        let mut doc = fixture();
        let original = doc.snapshot();
        let output300 = Renderer::new(&doc).output(300.0, 1000000).unwrap();
        assert!(
            output300
                .2
                .iter()
                .filter(|p| **p == [200, 10, 40, 255])
                .count()
                > 100,
            "Font contours must actually render"
        );
        assert!(output300.2.contains(&[10, 90, 180, 255]));
        assert!(output300.2.contains(&[40, 20, 0, 180]));
        assert!(output300.2.contains(&[0, 120, 30, 255]));
        let output600 = Renderer::new(&doc).output(600.0, 1000000).unwrap();
        let runs = |width: usize, pixels: &[[u8; 4]], row: usize| -> usize {
            let mut count = 0;
            let mut black = false;
            for pixel in &pixels[row * width..(row + 1) * width] {
                let next = *pixel == [0, 0, 0, 255];
                if next && !black {
                    count += 1;
                }
                black = next;
            }
            count
        };
        assert_eq!(runs(120, &output300.2, 52), 24);
        assert_eq!(runs(240, &output600.2, 105), 24);
        assert_eq!(doc.scene.tone_lpi(5.0, 300.0), [60.0; 2]);
        doc.commit_state(prepared(&doc, Change::Dpi(600.0)))
            .unwrap();
        assert_eq!(doc.layers, original.layers);
        assert_eq!(doc.scene, original.scene);
        assert_eq!(
            doc.scene.physical_points(10.0 * 300.0 / 72.0, 600.0),
            [5.0; 2]
        );
        assert_eq!(doc.scene.tone_lpi(5.0, 600.0), [120.0; 2]);
        assert_eq!(
            Renderer::new(&doc).output(600.0, 1000000).unwrap(),
            output300
        );
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), original);
    }
    #[test]
    fn canvas_anchor_and_crop_keep_editable_geometry_and_undo_restores_removed_pixels() {
        let mut doc = fixture();
        let original = doc.snapshot();
        let canvas = Change::Canvas {
            width: 100,
            height: 60,
            anchor: [0.5; 2],
        };
        let preview = plan(&original, canvas, DEFAULT_DOCUMENT_BUDGET).unwrap();
        assert_eq!(preview.offset, [-10, -10]);
        assert_eq!(preview.removed_pixels, 3600);
        doc.commit_state(prepared(&doc, canvas)).unwrap();
        assert_eq!(doc.scene.objects, original.scene.objects);
        assert_eq!(doc.scene.worlds, original.scene.worlds);
        assert!(doc.scene.clips.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), original);
        let crop = Change::Crop {
            x: 30,
            y: 20,
            width: 60,
            height: 40,
        };
        let preview = plan(&original, crop, DEFAULT_DOCUMENT_BUDGET).unwrap();
        assert!(!preview.clipped_objects.is_empty());
        doc.commit_state(prepared(&doc, crop)).unwrap();
        assert_eq!(doc.scene.objects, original.scene.objects);
        assert_eq!(doc.scene.worlds, original.scene.worlds);
        assert_eq!(doc.scene.clips.len(), 1);
        assert_eq!(doc.width, 60);
        let cropped = doc.snapshot();
        doc.commit_state(prepared(
            &doc,
            Change::Canvas {
                width: 120,
                height: 80,
                anchor: [0.5; 2],
            },
        ))
        .unwrap();
        assert_eq!(Renderer::new(&doc).pixel([2.5, 20.5]), [0; 4]);
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), cropped);
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), original);
    }
    #[test]
    fn bilinear_uses_premultiplied_alpha_and_bad_or_canceled_changes_leave_source_untouched() {
        let mut doc = Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.0,
                background: Background::Transparent,
            },
            8,
        )
        .unwrap();
        doc.layers[0].pixels = vec![[255, 0, 0, 255], [0, 0, 255, 0]];
        let original = doc.snapshot();
        let changed = prepared(
            &doc,
            Change::Resample {
                width: 3,
                height: 1,
                dpi: 300.0,
                filter: Interpolation::Bilinear,
            },
        );
        assert_eq!(changed.layers[0].pixels[1], [255, 0, 0, 128]);
        for change in [
            Change::Dpi(f64::NAN),
            Change::Crop {
                x: 1,
                y: 0,
                width: 2,
                height: 1,
            },
            Change::Canvas {
                width: 0,
                height: 1,
                anchor: [0.5; 2],
            },
            Change::Resample {
                width: 32768,
                height: 32768,
                dpi: 300.0,
                filter: Interpolation::Nearest,
            },
        ] {
            assert!(
                prepare(
                    &original,
                    change,
                    DEFAULT_DOCUMENT_BUDGET,
                    &AtomicBool::new(false)
                )
                .is_err()
            );
        }
        assert!(
            prepare(
                &original,
                Change::Dpi(600.0),
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(true)
            )
            .is_err()
        );
        assert_eq!(doc.snapshot(), original);
        assert_eq!(doc.history_counts(), (0, 0));
    }
    #[test]
    fn all_boundary_and_dpi_changes_reopen_with_same_appearance_and_saved_undo_checkpoint() {
        let root = TempDir::new();
        for change in [
            Change::Dpi(600.0),
            Change::Canvas {
                width: 150,
                height: 100,
                anchor: [1.0, 1.0],
            },
            Change::Crop {
                x: 30,
                y: 20,
                width: 60,
                height: 40,
            },
        ] {
            let mut doc = fixture();
            let original = doc.snapshot();
            doc.commit_state(prepared(&doc, change)).unwrap();
            let path = root.0.join("result.drawer");
            files::save(&mut doc, &path).unwrap();
            let reopened = files::open(&path, DEFAULT_DOCUMENT_BUDGET).unwrap();
            assert_eq!(doc.snapshot(), reopened.snapshot());
            assert_eq!(
                Renderer::new(&doc).output(doc.dpi, 1000000).unwrap(),
                Renderer::new(&reopened)
                    .output(reopened.dpi, 1000000)
                    .unwrap()
            );
            assert!(!doc.dirty);
            assert!(doc.undo());
            assert_eq!(doc.snapshot(), original);
            assert!(doc.dirty);
            assert!(doc.redo());
            assert!(!doc.dirty);
            assert_eq!(doc.snapshot(), reopened.snapshot());
        }
    }
    #[test]
    fn document_flip_and_quarter_turn_transform_mixed_geometry_and_undo_as_one_command() {
        for change in [
            Change::FlipHorizontal,
            Change::FlipVertical,
            Change::Rotate90 { clockwise: true },
            Change::Rotate90 { clockwise: false },
        ] {
            let mut doc = fixture();
            let original = doc.snapshot();
            let renderer = Renderer::new(&doc);
            let expected: Vec<_> = (0..80)
                .flat_map(|y| (0..120).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                .map(|p| renderer.pixel(p))
                .collect();
            doc.commit_state(prepared(&doc, change)).unwrap();
            let renderer = Renderer::new(&doc);
            for y in 0..80 {
                for x in 0..120 {
                    let point = doc.scene.transform.map([x as f64 + 0.5, y as f64 + 0.5]);
                    assert_eq!(renderer.pixel(point), expected[y * 120 + x]);
                }
            }
            assert_eq!(doc.scene.objects, original.scene.objects);
            assert_eq!(doc.scene.worlds, original.scene.worlds);
            assert_eq!(doc.history_counts(), (1, 0));
            assert!(doc.undo());
            assert_eq!(doc.snapshot(), original);
            assert!(doc.redo());
        }
        let mut doc = fixture();
        let original = doc.snapshot();
        for _ in 0..4 {
            doc.commit_state(prepared(&doc, Change::Rotate90 { clockwise: true }))
                .unwrap();
        }
        assert_eq!(doc.snapshot(), original);
    }
}
