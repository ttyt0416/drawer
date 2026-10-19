//! Editable page geometry. Image sizing composes a page transform instead of
//! rewriting font sizes, line widths, tone periods, or world-space vertices.
use serde::{Deserialize, Serialize};
pub type Point = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}
impl Rect {
    pub fn contains(self, point: Point) -> bool {
        (0..2).all(|i| point[i] >= self.min[i] && point[i] < self.max[i])
    }
    pub fn valid(self) -> bool {
        (0..2).all(|i| {
            self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i]
        })
    }
    pub fn intersection(self, other: Self) -> Self {
        let min = [self.min[0].max(other.min[0]), self.min[1].max(other.min[1])];
        Self {
            min,
            max: [
                self.max[0].min(other.max[0]).max(min[0]),
                self.max[1].min(other.max[1]).max(min[1]),
            ],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine(pub [f64; 6]);
impl Default for Affine {
    fn default() -> Self {
        Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }
}
impl Affine {
    pub fn map(self, [x, y]: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }
    pub fn inverse(self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        Self([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ])
    }
    pub fn then(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = next.0;
        Self([
            g * a + i * b,
            h * a + j * b,
            g * c + i * d,
            h * c + j * d,
            g * e + i * f + k,
            h * e + j * f + l,
        ])
    }
    pub fn valid(self) -> bool {
        let [a, b, c, d, _, _] = self.0;
        self.0.iter().all(|v| v.is_finite() && v.abs() <= 1e12) && (a * d - b * c).abs() > 1e-12
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PageObject {
    Text {
        baseline: Point,
        text: String,
        size_px: f64,
        color: [u8; 4],
    },
    Line {
        start: Point,
        end: Point,
        width_px: f64,
        color: [u8; 4],
    },
    Tone {
        bounds: Rect,
        period_px: f64,
        coverage: f64,
    },
    Shadow {
        bounds: Rect,
        offset: Point,
        radius_px: f64,
        color: [u8; 4],
    },
}
impl PageObject {
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Text {
                baseline,
                text,
                size_px,
                ..
            } => Rect {
                min: [baseline[0], baseline[1] - size_px],
                max: [
                    baseline[0] + size_px * text.chars().count() as f64,
                    baseline[1]
                        + size_px / 4.0
                        + (text.lines().count().saturating_sub(1) as f64) * size_px * 1.2,
                ],
            },
            Self::Line {
                start,
                end,
                width_px,
                ..
            } => Rect {
                min: [
                    start[0].min(end[0]) - width_px / 2.0,
                    start[1].min(end[1]) - width_px / 2.0,
                ],
                max: [
                    start[0].max(end[0]) + width_px / 2.0,
                    start[1].max(end[1]) + width_px / 2.0,
                ],
            },
            Self::Tone { bounds, .. } => *bounds,
            Self::Shadow {
                bounds,
                offset,
                radius_px,
                ..
            } => Rect {
                min: [
                    bounds.min[0] + offset[0] - radius_px,
                    bounds.min[1] + offset[1] - radius_px,
                ],
                max: [
                    bounds.max[0] + offset[0] + radius_px,
                    bounds.max[1] + offset[1] + radius_px,
                ],
            },
        }
    }
    fn valid(&self) -> bool {
        let positive = |v: f64| v.is_finite() && v > 0.0 && v <= 1e6;
        self.bounds().valid()
            && match self {
                Self::Text { text, size_px, .. } => text.len() <= 65536 && positive(*size_px),
                Self::Line { width_px, .. } => positive(*width_px),
                Self::Tone {
                    period_px,
                    coverage,
                    ..
                } => positive(*period_px) && (0.0..=1.0).contains(coverage),
                Self::Shadow {
                    offset, radius_px, ..
                } => positive(*radius_px) && offset.iter().all(|v| v.is_finite()),
            }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub page_bounds: Rect,
    pub position: [f64; 3],
    pub pixels_per_unit: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMesh {
    pub vertices: Vec<[f64; 3]>,
    pub edges: Vec<[usize; 2]>,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    #[serde(default, skip_serializing_if = "crate::appearance::Data::is_default")]
    pub appearance: crate::appearance::Data,
    #[serde(default, skip_serializing_if = "crate::layer_roles::is_false")]
    pub include_drafts_in_output: bool,
    #[serde(
        default,
        skip_serializing_if = "crate::layer_catalog::Data::is_default"
    )]
    pub catalog: crate::layer_catalog::Data,
    #[serde(
        default,
        skip_serializing_if = "crate::layer_effects::Data::is_default"
    )]
    pub layer_effects: crate::layer_effects::Data,
    #[serde(default, skip_serializing_if = "crate::clipping::Data::is_default")]
    pub clipping: crate::clipping::Data,
    #[serde(
        default,
        skip_serializing_if = "crate::layer_properties::Data::is_default"
    )]
    pub layer_properties: crate::layer_properties::Data,
    #[serde(default, skip_serializing_if = "crate::layers::Ids::is_default")]
    pub layer_ids: crate::layers::Ids,
    #[serde(
        default,
        skip_serializing_if = "crate::alignment_detail::Settings::is_default"
    )]
    pub alignment: crate::alignment_detail::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::animation_output::Data::is_default"
    )]
    pub animation_output: crate::animation_output::Data,
    #[serde(default, skip_serializing_if = "crate::gradient::Data::is_default")]
    pub gradients: crate::gradient::Data,
    #[serde(default, skip_serializing_if = "crate::rulers::Data::is_default")]
    pub rulers: crate::rulers::Data,
    #[serde(
        default,
        skip_serializing_if = "crate::selection_tools::Data::is_default"
    )]
    pub selections: crate::selection_tools::Data,
    #[serde(default, skip_serializing_if = "crate::line_tools::Data::is_default")]
    pub lines: crate::line_tools::Data,
    #[serde(default, skip_serializing_if = "crate::paper::Graph::is_empty")]
    pub paper: crate::paper::Graph,
    #[serde(default)]
    pub grid: crate::aids::Grid,
    pub transform: Affine,
    /// Crop mask in the preserved original coordinate system.
    pub clips: Vec<[Point; 4]>,
    pub objects: Vec<PageObject>,
    pub selection: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_mask: Option<crate::selection::Mask>,
    /// Document-space translation, independent of page object transforms.
    #[serde(default)]
    pub selection_translation: Point,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mouse_strokes: Vec<crate::mouse_pressure::Metadata>,
    pub guides: Vec<[Point; 2]>,
    pub camera: Option<Camera>,
    pub worlds: Vec<WorldMesh>,
}
impl Scene {
    pub fn valid(&self) -> bool {
        self.appearance.valid()
            && self
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .flat_map(|p| &p.points)
                .all(|p| {
                    self.transform
                        .map(*p)
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 1e12)
                })
            && self.catalog.valid()
            && self.catalog.objects.matches(self.objects.len())
            && self.catalog.worlds.matches(self.worlds.len())
            && self.clipping.valid()
            && self.layer_effects.valid()
            && self.layer_properties.valid()
            && self.layer_ids.valid()
            && self.alignment.valid()
            && self.animation_output.valid()
            && self.rulers.valid()
            && self.gradients.valid()
            && self.selections.valid()
            && self.lines.valid()
            && self.transform.valid()
            && self.paper.valid_for(256)
            && self.mouse_strokes.len() <= 4096
            && self
                .mouse_strokes
                .iter()
                .all(crate::mouse_pressure::Metadata::valid)
            && self
                .selection_translation
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
            && self.grid.valid()
            && self.clips.len() <= 4096
            && self.clips.iter().flatten().flatten().all(|v| v.is_finite())
            && self.selection.is_none_or(Rect::valid)
            && self
                .selection_mask
                .as_ref()
                .is_none_or(crate::selection::Mask::valid)
            && self.objects.len() <= 4096
            && self.objects.iter().all(PageObject::valid)
            && self
                .objects
                .iter()
                .map(|o| match o {
                    PageObject::Text { text, .. } => text.len(),
                    _ => 0,
                })
                .sum::<usize>()
                <= 16384
            && self.objects.iter().all(|o| {
                [o.bounds().min, o.bounds().max].into_iter().all(|point| {
                    self.transform
                        .map(point)
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 1e12)
                })
            })
            && self.guides.len() <= 4096
            && self
                .guides
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite())
            && self.camera.as_ref().is_none_or(|c| {
                c.page_bounds.valid()
                    && c.position.iter().all(|v| v.is_finite())
                    && c.pixels_per_unit.is_finite()
                    && c.pixels_per_unit > 0.0
            })
            && self.worlds.len() <= 256
            && self.worlds.iter().all(|mesh| {
                mesh.vertices.len() <= 65536
                    && mesh.edges.len() <= 65536
                    && mesh.vertices.iter().flatten().all(|v| v.is_finite())
                    && mesh
                        .edges
                        .iter()
                        .flatten()
                        .all(|index| *index < mesh.vertices.len())
            })
    }
    pub fn size_bytes(&self) -> usize {
        self.appearance.bytes()
            + self.catalog.bytes()
            + self.clipping.bytes()
            + self.layer_effects.bytes()
            + self.layer_properties.bytes()
            + self.layer_ids.rasters.len() * 8
            + self.alignment.selected.len() * std::mem::size_of::<crate::alignment::Item>()
            + self.animation_output.bytes()
            + self.rulers.bytes()
            + self.gradients.size_bytes()
            + self.selections.size_bytes()
            + self.lines.size_bytes()
            + self.paper.size_bytes()
            + self.clips.len() * 64
            + self
                .objects
                .iter()
                .map(|o| {
                    std::mem::size_of_val(o)
                        + match o {
                            PageObject::Text { text, .. } => text.len(),
                            _ => 0,
                        }
                })
                .sum::<usize>()
            + self.guides.len() * 32
            + self
                .worlds
                .iter()
                .map(|w| w.vertices.len() * 24 + w.edges.len() * 16)
                .sum::<usize>()
            + usize::from(self.selection.is_some()) * 32
            + self.selection_mask.as_ref().map_or(0, |m| {
                m.runs.len() * std::mem::size_of::<crate::selection::Run>()
            })
            + usize::from(self.camera.is_some()) * 64
            + self.mouse_strokes.len() * std::mem::size_of::<crate::mouse_pressure::Metadata>()
    }
    pub fn physical_points(&self, original_pixels: f64, dpi: f64) -> Point {
        [
            self.transform.0[0].hypot(self.transform.0[1]) * original_pixels * 72.0 / dpi,
            self.transform.0[2].hypot(self.transform.0[3]) * original_pixels * 72.0 / dpi,
        ]
    }
    pub fn tone_lpi(&self, period_px: f64, dpi: f64) -> Point {
        let points = self.physical_points(period_px, dpi);
        [72.0 / points[0], 72.0 / points[1]]
    }
    pub fn clipped_objects(&self, output: Rect) -> Vec<usize> {
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(index, object)| {
                let bounds = object.bounds();
                let corners = [
                    bounds.min,
                    [bounds.max[0], bounds.min[1]],
                    bounds.max,
                    [bounds.min[0], bounds.max[1]],
                ];
                corners
                    .into_iter()
                    .any(|p| !output.contains(self.transform.map(p)))
                    .then_some(index)
            })
            .collect()
    }
}
