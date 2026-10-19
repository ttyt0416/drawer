//! Canvas picking uses the existing renderer's visibility and alpha rules.
use crate::{document::Document, paper::Node, scene::Point, thumbnail::Target};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier {
    #[default]
    Alt,
    Ctrl,
    Shift,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub modifier: Modifier,
    pub include_locked: bool,
    pub folders: bool,
}
fn nodes(doc: &Document, node: Node, folders: bool, out: &mut Vec<Target>, depth: usize) {
    if depth > 16 {
        return;
    }
    match node {
        Node::Raster(n) => {
            if doc.scene.paper.raster_visible(n, doc.layers.len()) {
                out.push(Target::Raster(n));
            }
        }
        Node::Folder(n) => {
            if let Some(f) = doc.scene.paper.folders.get(n)
                && !f.paper.hidden
            {
                for c in &f.children {
                    nodes(doc, *c, folders, out, depth + 1);
                }
                if folders {
                    out.push(Target::Folder(n));
                }
            }
        }
    }
}
pub fn visible(doc: &Document, settings: Settings) -> Vec<Target> {
    let mut out = Vec::new();
    for n in doc.scene.paper.root_nodes(doc.layers.len()) {
        nodes(doc, n, settings.folders, &mut out, 0);
    }
    if let Some(a) = &doc.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            if doc.current_frame < t.end && t.exposures.iter().any(|e| e.frame <= doc.current_frame)
            {
                out.push(Target::Track(n));
            }
        }
    }
    for l in doc.scene.lines.layers.iter().filter(|l| !l.hidden) {
        out.push(Target::Vector(l.id));
    }
    out.retain(|target| {
        !crate::layer_properties::inherited(
            doc,
            crate::layer_properties::from_thumbnail(doc, *target),
        )
        .hidden
            && (settings.include_locked || !locked(doc, *target))
    });
    out.reverse();
    out
}
pub fn locked(doc: &Document, target: Target) -> bool {
    let p = crate::layer_properties::inherited(
        doc,
        crate::layer_properties::from_thumbnail(doc, target),
    );
    if p.locked || p.pixels_locked || p.position_locked {
        return true;
    }
    match target {
        Target::Raster(n) => doc.layers.get(n).is_some_and(|l| l.locked),
        Target::Vector(id) => doc
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .is_some_and(|l| l.locked),
        _ => false,
    }
}
pub fn label(doc: &Document, target: Target) -> String {
    match target {
        Target::Raster(n) => doc.layers.get(n).map(|l| {
            format!(
                "Raster {}: {}{}",
                n + 1,
                l.name,
                if l.locked { " [locked]" } else { "" }
            )
        }),
        Target::Track(n) => doc
            .animation
            .as_ref()
            .and_then(|a| a.tracks.get(n))
            .map(|t| format!("Animation: {}", t.name)),
        Target::Folder(n) => doc
            .scene
            .paper
            .folders
            .get(n)
            .map(|f| format!("Folder: {}", f.name)),
        Target::Vector(id) => doc.scene.lines.layers.iter().find(|l| l.id == id).map(|l| {
            format!(
                "Vector: {}{}",
                l.name,
                if l.locked { " [locked]" } else { "" }
            )
        }),
    }
    .unwrap_or_else(|| "Unavailable layer".into())
}
pub fn candidates(doc: &Document, point: Point, settings: Settings) -> Vec<Target> {
    if point.iter().any(|v| !v.is_finite())
        || point[0] < 0.0
        || point[1] < 0.0
        || point[0] >= doc.width as f64
        || point[1] >= doc.height as f64
    {
        return vec![];
    }
    let x = point[0].floor() as u32;
    let y = point[1].floor() as u32;
    visible(doc, settings)
        .into_iter()
        .filter(|target| match *target {
            Target::Raster(n) => {
                if doc
                    .scene
                    .clipping
                    .binding(crate::layer_properties::raster(doc, n))
                    .is_some()
                {
                    let alpha = doc.scene.paper.clipped_node_pixel(
                        doc,
                        Node::Raster(n),
                        [x as i64, y as i64],
                    )[3] as f64;
                    let mut weight = 1.0;
                    let mut node = Node::Raster(n);
                    for _ in 0..16 {
                        let Some(parent) = doc
                            .scene
                            .paper
                            .folders
                            .iter()
                            .position(|f| f.children.contains(&node))
                        else {
                            break;
                        };
                        weight *= crate::layer_properties::factor(
                            doc,
                            crate::layer_properties::Target::Folder(parent as u16),
                        );
                        node = Node::Folder(parent);
                    }
                    return (alpha * weight).round() > 0.0;
                }
                doc.composite_pixel_filtered(x, y, false, |i, l| {
                    if i == n {
                        l.pixel(doc.width, doc.height, x as i64, y as i64)
                    } else {
                        [0; 4]
                    }
                })[3]
                    > 0
            }
            Target::Folder(n) => {
                doc.scene
                    .paper
                    .node_pixel(doc, Node::Folder(n), [x as i64, y as i64])[3]
                    > 0
            }
            Target::Track(n) => doc
                .animation
                .as_ref()
                .and_then(|a| {
                    a.indexed_visible_cells(doc.current_frame)
                        .find(|(track, _, _)| *track == n)
                })
                .is_some_and(|(track, cell, c)| {
                    let w = crate::layer_properties::factor(
                        doc,
                        crate::layer_properties::Target::Track(track as u16),
                    ) * crate::layer_properties::factor(
                        doc,
                        crate::layer_properties::Target::Cell {
                            track: track as u16,
                            cell: cell as u16,
                        },
                    );
                    (c.pixel(x as i64, y as i64)[3] as f64 * w).round() > 0.0
                }),
            Target::Vector(id) => {
                if crate::layer_properties::factor(doc, crate::layer_properties::Target::Vector(id))
                    == 0.0
                {
                    return false;
                }
                let p = doc.scene.transform.inverse().map(point);
                let mask = if let Some(base) = doc
                    .scene
                    .clipping
                    .binding(crate::layer_properties::Target::Vector(id))
                {
                    if !crate::clipping::active(doc, crate::layer_properties::Target::Vector(id)) {
                        return false;
                    }
                    let crate::layer_properties::Target::Vector(base) = base else {
                        return false;
                    };
                    let Some(l) = doc.scene.lines.layers.iter().find(|l| l.id == base) else {
                        return false;
                    };
                    if l.hidden {
                        return false;
                    }
                    l.paths
                        .iter()
                        .fold([0; 4], |out, path| crate::render::over(out, path.pixel(p)))[3]
                        as f64
                        / 255.0
                        * crate::layer_properties::factor(
                            doc,
                            crate::layer_properties::Target::Vector(base),
                        )
                } else {
                    1.0
                };
                !doc.scene.clips.iter().any(|r| !crate::render::inside(r, p))
                    && doc
                        .scene
                        .lines
                        .layers
                        .iter()
                        .find(|l| l.id == id)
                        .is_some_and(|l| {
                            let pixel = l
                                .paths
                                .iter()
                                .fold([0; 4], |out, path| crate::render::over(out, path.pixel(p)));
                            (pixel[3] as f64
                                * mask
                                * crate::layer_properties::factor(
                                    doc,
                                    crate::layer_properties::Target::Vector(id),
                                ))
                            .round()
                                > 0.0
                        })
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Background, CanvasSpec};
    #[test]
    fn vector_clip_and_animation_follow_render_order() {
        use crate::animation::{Cell, Exposure, Fps, Timeline, Track};
        use std::sync::Arc;
        let mut d = Document::create(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1000,
        )
        .unwrap();
        d.layers[0].pixels.fill([0, 0, 0, 255]);
        d.animation = Some(Timeline {
            fps: Fps {
                numerator: 24,
                denominator: 1,
            },
            frames: 1,
            tracks: vec![Track {
                name: "Track".into(),
                cells: vec![Cell {
                    name: "Cell".into(),
                    width: 1,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[0, 0, 0, 128]]),
                }],
                exposures: vec![Exposure { frame: 0, cell: 0 }],
                end: 1,
            }],
            audio: vec![],
        });
        let id = d.scene.lines.add_layer("Line").unwrap();
        d.scene
            .lines
            .add_path(
                id,
                vec![[0.0, 0.5], [2.0, 0.5]],
                false,
                1.0,
                [20, 30, 40, 255],
            )
            .unwrap();
        assert_eq!(
            candidates(&d, [0.5, 0.5], Settings::default()),
            vec![Target::Vector(id), Target::Track(0), Target::Raster(0)]
        );
        d.scene
            .clips
            .push([[1.0, 0.0], [3.0, 0.0], [3.0, 2.0], [1.0, 2.0]]);
        assert_eq!(
            candidates(&d, [0.5, 0.5], Settings::default()),
            vec![Target::Track(0), Target::Raster(0)]
        );
        d.scene.lines.layers[0].hidden = true;
        assert_eq!(
            candidates(&d, [1.5, 0.5], Settings::default()),
            vec![Target::Raster(0)]
        );
    }
    #[test]
    fn ordered_overlap_holes_hidden_lock_folder_and_no_pixel_history_changes() {
        let mut d = Document::create(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1000,
        )
        .unwrap();
        d.layers[0].pixels.fill([1, 2, 3, 255]);
        for _ in 0..2 {
            let l = d.layers[0].clone();
            d.layers.push(l);
        }
        d.layers[2].pixels[0] = [200, 20, 30, 0];
        let s = Settings::default();
        let before = d.snapshot();
        assert_eq!(
            candidates(&d, [0.5, 0.5], s),
            vec![Target::Raster(1), Target::Raster(0)]
        );
        assert_eq!(
            candidates(&d, [1.5, 0.5], s),
            vec![Target::Raster(2), Target::Raster(1), Target::Raster(0)]
        );
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        d.layers[2].locked = true;
        assert_eq!(candidates(&d, [1.5, 0.5], s)[0], Target::Raster(1));
        assert_eq!(
            candidates(
                &d,
                [1.5, 0.5],
                Settings {
                    include_locked: true,
                    ..s
                }
            )[0],
            Target::Raster(2)
        );
        assert!(d.layers[2].locked);
        d.scene.paper.layers.entry(1).or_default().hidden = true;
        assert_eq!(candidates(&d, [1.5, 0.5], s), vec![Target::Raster(0)]);
        let f = d.scene.paper.group_all(3, "Group").unwrap();
        assert_eq!(
            candidates(&d, [1.5, 0.5], Settings { folders: true, ..s })[0],
            Target::Folder(f)
        );
        d.scene.paper.folders[f].paper.hidden = true;
        assert!(candidates(&d, [1.5, 0.5], s).is_empty());
        assert!(candidates(&d, [f64::NAN, 0.0], s).is_empty());
    }
}
