//! Small page renderer used by sizing fixtures and the canvas preview.
//! Later editors share these editable objects; this is not a text layout engine
//! or a shaded 3D renderer.
use crate::{
    document::Document,
    scene::{PageObject, Point, Rect},
};

pub struct Renderer<'a> {
    document: &'a Document,
    stroke: Option<&'a crate::painting::Stroke>,
    text_contours: Vec<Vec<Contour>>,
}
struct Contour {
    points: Vec<Point>,
    bounds: Rect,
}
impl<'a> Renderer<'a> {
    pub fn new(document: &'a Document) -> Self {
        Self::preview(document, None)
    }
    pub fn preview(document: &'a Document, stroke: Option<&'a crate::painting::Stroke>) -> Self {
        let text_contours = document
            .scene
            .objects
            .iter()
            .map(|object| match object {
                PageObject::Text {
                    baseline,
                    text,
                    size_px,
                    ..
                } => glyph_contours(*baseline, text, *size_px),
                _ => Vec::new(),
            })
            .collect();
        Self {
            document,
            stroke,
            text_contours,
        }
    }
    pub fn pixel(&self, point: Point) -> [u8; 4] {
        let doc = self.document;
        if point[0] < 0.0
            || point[1] < 0.0
            || point[0] >= doc.width as f64
            || point[1] >= doc.height as f64
        {
            return [0; 4];
        }
        let x = point[0] as u32;
        let y = point[1] as u32;
        let mut result = doc.composite_pixel_with(x, y, |index, layer| {
            self.stroke
                .and_then(|s| s.pixel(doc, index, layer, x, y))
                .unwrap_or_else(|| layer.pixel(doc.width, doc.height, x as i64, y as i64))
        });
        let scene = &doc.scene;
        let original = scene.transform.inverse().map(point);
        if scene.clips.iter().any(|clip| !inside(clip, original)) {
            return result;
        }
        for (index, object) in scene.objects.iter().enumerate() {
            let color = match object {
                PageObject::Text { color, .. } => {
                    let winding: i32 = self.text_contours[index]
                        .iter()
                        .filter(|path| path.bounds.contains(original))
                        .map(|path| winding(&path.points, original))
                        .sum();
                    if winding != 0 { *color } else { [0; 4] }
                }
                PageObject::Line {
                    start,
                    end,
                    width_px,
                    color,
                } => {
                    if distance(original, *start, *end) <= width_px / 2.0 {
                        *color
                    } else {
                        [0; 4]
                    }
                }
                PageObject::Tone {
                    bounds,
                    period_px,
                    coverage,
                } => {
                    let cell = [
                        (original[0] / period_px).rem_euclid(1.0) - 0.5,
                        (original[1] / period_px).rem_euclid(1.0) - 0.5,
                    ];
                    if bounds.contains(original)
                        && *coverage > 0.0
                        && (*coverage == 1.0
                            || cell[0] * cell[0] + cell[1] * cell[1]
                                <= coverage / std::f64::consts::PI)
                    {
                        [0, 0, 0, 255]
                    } else {
                        [0; 4]
                    }
                }
                PageObject::Shadow {
                    bounds,
                    offset,
                    radius_px,
                    color,
                } => {
                    let point = [original[0] - offset[0], original[1] - offset[1]];
                    let delta = [
                        (bounds.min[0] - point[0])
                            .max(0.0)
                            .max(point[0] - bounds.max[0]),
                        (bounds.min[1] - point[1])
                            .max(0.0)
                            .max(point[1] - bounds.max[1]),
                    ];
                    let alpha = (1.0 - delta[0].hypot(delta[1]) / radius_px).clamp(0.0, 1.0);
                    [
                        color[0],
                        color[1],
                        color[2],
                        (color[3] as f64 * alpha).round() as u8,
                    ]
                }
            };
            result = over(result, color);
        }
        if let Some(camera) = &scene.camera
            && camera.page_bounds.contains(original)
        {
            let center = [
                (camera.page_bounds.min[0] + camera.page_bounds.max[0]) / 2.0,
                (camera.page_bounds.min[1] + camera.page_bounds.max[1]) / 2.0,
            ];
            for mesh in &scene.worlds {
                let project = |vertex: [f64; 3]| {
                    [
                        center[0] + (vertex[0] - camera.position[0]) * camera.pixels_per_unit,
                        center[1] - (vertex[1] - camera.position[1]) * camera.pixels_per_unit,
                    ]
                };
                if mesh.edges.iter().any(|edge| {
                    distance(
                        original,
                        project(mesh.vertices[edge[0]]),
                        project(mesh.vertices[edge[1]]),
                    ) <= 0.5
                }) {
                    result = over(result, mesh.color);
                }
            }
        }
        result
    }
    /// Output at a requested DPI while preserving the document's physical size.
    /// Returns RGBA pixels; output file codecs are separate checklist features.
    pub fn output(&self, dpi: f64, budget: usize) -> Result<(u32, u32, Vec<[u8; 4]>), String> {
        if !dpi.is_finite() || dpi <= 0.0 {
            return Err("Invalid output DPI.".into());
        }
        let ratio = dpi / self.document.dpi;
        let wf = (self.document.width as f64 * ratio).round();
        let hf = (self.document.height as f64 * ratio).round();
        if !(1.0..=32768.0).contains(&wf) || !(1.0..=32768.0).contains(&hf) {
            return Err("Output size exceeds supported dimensions.".into());
        }
        let (width, height) = (wf as u32, hf as u32);
        let count = width as usize * height as usize;
        if count.saturating_mul(4) > budget {
            return Err("Output exceeds memory budget.".into());
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| "Not enough output memory.")?;
        for y in 0..height {
            for x in 0..width {
                pixels.push(self.pixel([(x as f64 + 0.5) / ratio, (y as f64 + 0.5) / ratio]));
            }
        }
        Ok((width, height, pixels))
    }
}
fn over(destination: [u8; 4], source: [u8; 4]) -> [u8; 4] {
    let sa = source[3] as f64 / 255.0;
    let da = destination[3] as f64 / 255.0;
    let alpha = sa + da * (1.0 - sa);
    if alpha == 0.0 {
        return [0; 4];
    }
    let mut color = [0; 4];
    for i in 0..3 {
        color[i] = ((source[i] as f64 * sa + destination[i] as f64 * da * (1.0 - sa)) / alpha)
            .round() as u8;
    }
    color[3] = (alpha * 255.0).round() as u8;
    color
}
fn distance(point: Point, start: Point, end: Point) -> f64 {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let length = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if length == 0.0 {
        0.0
    } else {
        ((point[0] - start[0]) * delta[0] + (point[1] - start[1]) * delta[1]) / length
    }
    .clamp(0.0, 1.0);
    (point[0] - start[0] - t * delta[0]).hypot(point[1] - start[1] - t * delta[1])
}
fn winding(path: &[Point], point: Point) -> i32 {
    if path.is_empty() {
        return 0;
    }
    let mut result = 0;
    let mut previous = path[path.len() - 1];
    for &next in path {
        let cross = (next[0] - previous[0]) * (point[1] - previous[1])
            - (point[0] - previous[0]) * (next[1] - previous[1]);
        if previous[1] <= point[1] && next[1] > point[1] && cross > 0.0 {
            result += 1;
        }
        if previous[1] > point[1] && next[1] <= point[1] && cross < 0.0 {
            result -= 1;
        }
        previous = next;
    }
    result
}
fn inside(path: &[Point], point: Point) -> bool {
    winding(path, point) != 0
}
struct Outline {
    paths: Vec<Vec<Point>>,
    current: Point,
    scale: f64,
    origin: Point,
}
impl Outline {
    fn push(&mut self, point: Point) {
        self.paths.last_mut().unwrap().push([
            self.origin[0] + point[0] * self.scale,
            self.origin[1] - point[1] * self.scale,
        ]);
        self.current = point;
    }
}
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.paths.push(Vec::new());
        self.push([x as f64, y as f64]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push([x as f64, y as f64]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let from = self.current;
        for n in 1..=16 {
            let t = n as f64 / 16.0;
            let u = 1.0 - t;
            self.push([
                u * u * from[0] + 2.0 * u * t * x1 as f64 + t * t * x as f64,
                u * u * from[1] + 2.0 * u * t * y1 as f64 + t * t * y as f64,
            ]);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let from = self.current;
        for n in 1..=24 {
            let t = n as f64 / 24.0;
            let u = 1.0 - t;
            self.push([
                u * u * u * from[0]
                    + 3.0 * u * u * t * x1 as f64
                    + 3.0 * u * t * t * x2 as f64
                    + t * t * t * x as f64,
                u * u * u * from[1]
                    + 3.0 * u * u * t * y1 as f64
                    + 3.0 * u * t * t * y2 as f64
                    + t * t * t * y as f64,
            ]);
        }
    }
    fn close(&mut self) {}
}
fn glyph_contours(baseline: Point, text: &str, size: f64) -> Vec<Contour> {
    let face = ttf_parser::Face::parse(epaint_default_fonts::UBUNTU_LIGHT, 0)
        .expect("Embedded font is tested at build time");
    let scale = size / f64::from(face.units_per_em());
    let mut outline = Outline {
        paths: Vec::new(),
        current: [0.0; 2],
        scale,
        origin: baseline,
    };
    for character in text.chars() {
        if character == '\n' {
            outline.origin[0] = baseline[0];
            outline.origin[1] += size * 1.2;
            continue;
        }
        let id = face
            .glyph_index(character)
            .unwrap_or(ttf_parser::GlyphId(0));
        face.outline_glyph(id, &mut outline);
        outline.origin[0] +=
            f64::from(face.glyph_hor_advance(id).unwrap_or(face.units_per_em())) * scale;
    }
    outline
        .paths
        .into_iter()
        .filter(|path| !path.is_empty())
        .map(|points| {
            let mut bounds = Rect {
                min: points[0],
                max: points[0],
            };
            for point in &points {
                for (i, value) in point.iter().enumerate() {
                    bounds.min[i] = bounds.min[i].min(*value);
                    bounds.max[i] = bounds.max[i].max(*value);
                }
            }
            Contour { points, bounds }
        })
        .collect()
}
