//! Small page renderer used by sizing fixtures and the canvas preview.
//! Later editors share these editable objects; this is not a text layout engine
//! or a shaded 3D renderer.
use crate::{
    document::Document,
    scene::{PageObject, Point, Rect},
};

#[path = "text_contour_tree.rs"]
mod text_contour_tree;
#[path = "text_paint.rs"]
mod text_paint;
pub struct Renderer<'a> {
    page_parts: Vec<(u64, Box<Renderer<'a>>)>,
    float_plan: Result<Option<Box<crate::float_render::Plan<'a>>>, String>,
    document: &'a Document,
    owned: Option<Box<Document>>,
    appearance: Result<crate::appearance::Caches, String>,
    fill: Option<(usize, crate::bucket::Settings)>,
    fill_graph: Option<crate::paper::Graph>,
    stroke: Option<&'a crate::painting::Stroke>,
    references: Option<&'a crate::eyedropper::References>,
    text_contours: Vec<text_paint::Text>,
    comic_geometry: Vec<Option<crate::comic_object::Geometry>>,
    text_error: Option<String>,
    export_page: bool,
    comic_clips: bool,
    vector_bounds: Vec<Vec<Rect>>,
    order: Vec<crate::render_order::Entry>,
    deferred: Vec<bool>,
}
struct Contour {
    points: Vec<Point>,
    bounds: Rect,
}
impl<'a> Renderer<'a> {
    fn ordered(&self, mut result: [u8; 4], point: Point, original: Point) -> [u8; 4] {
        use crate::{layer_properties::Target, layer_search::Key};
        let d = self.doc();
        let mut i = 0;
        let objects = d.scene.clipping.solo.is_none()
            && self.references.is_none_or(|r| r.objects)
            && self.fill.is_none_or(|(_, s)| {
                matches!(
                    s.reference,
                    crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                )
            });
        while i < self.order.len() {
            let key = self.order[i].key;
            if matches!(key, Key::Layer(Target::Vector(_))) {
                let mut end = i + 1;
                while end < self.order.len()
                    && matches!(self.order[end].key, Key::Layer(Target::Vector(_)))
                {
                    end += 1;
                }
                result = crate::paper::clipped_vectors(
                    d,
                    point,
                    original,
                    result,
                    |n, l| {
                        self.order[i..end]
                            .iter()
                            .any(|e| e.key == Key::Layer(Target::Vector(l.id)))
                            && self.references.is_none_or(|r| r.vectors)
                            && self
                                .fill
                                .is_none_or(|(_, s)| crate::fill_reference::vector(l, n, s))
                    },
                    self.fill.is_some_and(|(_, s)| s.advanced.vector_centerline),
                    |n| Some(self.vector_pixel(n, point)),
                );
                i = end;
                continue;
            }
            match key {
                Key::Layer(Target::Raster(id)) => {
                    if let Some(n) = crate::layers::index(d, id)
                        && self.references.is_none_or(|r| r.raster(n))
                        && self
                            .fill
                            .is_none_or(|(a, s)| crate::fill_reference::raster(d, n, a, s))
                    {
                        let source = self.source_pixel_unbounded(key, point);
                        let mut output = crate::paper::Color::default();
                        output.add(result);
                        crate::layer_blend::mode(d, Target::Raster(id)).over(&mut output, {
                            let mut c = crate::paper::Color::default();
                            c.add(source);
                            c
                        });
                        result = output.rgba();
                    }
                }
                Key::Gradient(id) if objects => {
                    if let Some(g) = d.scene.gradients.layers.iter().find(|g| g.id == id) {
                        let c = d
                            .scene
                            .file_objects
                            .pixel(key, original)
                            .unwrap_or_else(|| g.pixel(original));
                        result = crate::layer_catalog::compose(d, key, result, c);
                    }
                }
                Key::Object(_) if objects => {
                    if let Some(n) = crate::layer_catalog::source_index(d, key) {
                        result = crate::layer_catalog::compose(
                            d,
                            key,
                            result,
                            self.object_pixel(n, &d.scene.objects[n], original),
                        );
                    }
                }
                Key::World(_) if objects => {
                    if let Some(n) = crate::layer_catalog::source_index(d, key) {
                        result = crate::layer_catalog::compose(
                            d,
                            key,
                            result,
                            self.world_pixel(n, original),
                        );
                    }
                }
                _ => {}
            }
            i += 1;
        }
        result
    }
    pub fn error(&self) -> Option<&str> {
        self.float_plan
            .as_ref()
            .err()
            .map(String::as_str)
            .or_else(|| self.appearance.as_ref().err().map(String::as_str))
            .or(self.text_error.as_deref())
            .or_else(|| self.page_parts.iter().find_map(|(_, r)| r.error()))
    }
    fn float_renderer(&self) -> Result<Option<crate::float_render::Renderer<'_>>, String> {
        match &self.float_plan {
            Err(e) => Err(e.clone()),
            Ok(None) => Ok(None),
            Ok(Some(plan)) => Ok(Some(crate::float_render::Renderer::prepared(
                self.doc(),
                self.stroke,
                plan,
                crate::float_render::Sampling {
                    pre_effects: false,
                    references: self.references,
                    fill: self.fill,
                    graph: self.fill_graph.as_ref(),
                },
            ))),
        }
    }
    pub fn sample(&self, point: Point) -> Result<crate::color_precision::Sample, String> {
        if let Some(e) = self.error() {
            return Err(e.into());
        }
        if let Some(renderer) = self.float_renderer()? {
            renderer.pixel(point)
        } else {
            Ok(crate::color_precision::Sample::from8(self.pixel(point)))
        }
    }
    pub fn display_sample(&self, point: Point) -> Result<crate::color_precision::Sample, String> {
        let sample = self.sample(point)?;
        self.to_display(sample)
    }
    pub fn to_display(
        &self,
        sample: crate::color_precision::Sample,
    ) -> Result<crate::color_precision::Sample, String> {
        if let Ok(Some(plan)) = &self.float_plan {
            plan.display(sample)
        } else {
            Ok(sample)
        }
    }
    pub(crate) fn raster_pixel(&self, n: usize, point: [i64; 2]) -> [u8; 4] {
        let doc = self.doc();
        let key = crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(
            doc.scene.layer_ids.id(n),
        ));
        if self.comic_clips
            && crate::comics::coverage(
                doc,
                key,
                doc.scene
                    .transform
                    .inverse()
                    .map(point.map(|v| v as f64 + 0.5)),
            ) == 0.
        {
            return [0; 4];
        }
        if let Some(c) = doc.scene.file_objects.pixel(
            key,
            doc.scene
                .transform
                .inverse()
                .map(point.map(|v| v as f64 + 0.5)),
        ) {
            return c;
        }
        if let Ok(caches) = &self.appearance
            && let Some(c) = caches.rasters.get(n).and_then(|c| c.as_ref())
        {
            return c.pixel(point);
        }
        let Some(layer) = doc.layers.get(n) else {
            return [0; 4];
        };
        self.stroke
            .and_then(|s| {
                if point.iter().all(|v| *v >= 0 && *v <= u32::MAX as i64) {
                    s.pixel(doc, n, layer, point[0] as u32, point[1] as u32)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| layer.pixel(doc.width, doc.height, point[0], point[1]))
    }
    pub(crate) fn vector_pixel(&self, n: usize, point: Point) -> [u8; 4] {
        let doc = self.doc();
        if let Some(l) = doc.scene.lines.layers.get(n)
            && self.comic_clips
            && crate::comics::coverage(
                doc,
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(l.id)),
                doc.scene.transform.inverse().map(point),
            ) == 0.
        {
            return [0; 4];
        }
        if let Some(l) = doc.scene.lines.layers.get(n)
            && let Some(c) = doc.scene.file_objects.pixel(
                crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(l.id)),
                doc.scene.transform.inverse().map(point),
            )
        {
            return c;
        }
        if let Ok(caches) = &self.appearance
            && let Some(c) = caches.vectors.get(n).and_then(|c| c.as_ref())
        {
            return c.pixel(point.map(|v| v.floor() as i64));
        }
        let original = doc.scene.transform.inverse().map(point);
        doc.scene.lines.layers.get(n).map_or([0; 4], |l| {
            l.paths
                .iter()
                .fold([0; 4], |out, p| over(out, p.pixel(original)))
        })
    }
    fn doc(&self) -> &Document {
        self.owned.as_deref().unwrap_or(self.document)
    }
    pub fn for_export(document: &'a Document) -> Result<Self, String> {
        document.scene.comics.page.guides(document)?;
        let mut r = Self::new(document);
        r.owned = crate::layer_roles::output_view(document)?;
        r.page_parts = document
            .scene
            .page_parts
            .entries
            .iter()
            .filter_map(|p| {
                p.export_document()
                    .map(|d| Self::for_export(d).map(|r| (p.object, Box::new(r))))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if document.color != crate::color_storage::Config::legacy()
            || !document.scene.surface_effects.rasters.is_empty()
            || crate::local_color::active(&document.scene)
        {
            r.float_plan = crate::float_render::Renderer::prepare_export_plan(document).map(Some);
        }
        r.export_page = true;
        Ok(r)
    }
    pub(crate) fn object_pixel(
        &self,
        index: usize,
        object: &PageObject,
        original: Point,
    ) -> [u8; 4] {
        if self.comic_clips
            && crate::comics::coverage(
                self.doc(),
                crate::layer_search::Key::Object(self.doc().scene.catalog.objects.id(index)),
                original,
            ) == 0.
        {
            return [0; 4];
        }
        if let Some(c) = self.doc().scene.file_objects.pixel(
            crate::layer_search::Key::Object(self.doc().scene.catalog.objects.id(index)),
            original,
        ) {
            return c;
        }
        match object {
            PageObject::Comic(_) => self
                .comic_geometry
                .get(index)
                .and_then(|g| g.as_ref())
                .map_or([0; 4], |g| {
                    g.pixel(original, self.doc().scene.transform, self.doc().dpi)
                }),
            PageObject::Material(m) => {
                let id = self.doc().scene.catalog.objects.id(index);
                if let Some((_, child)) = self.page_parts.iter().find(|(key, _)| *key == id) {
                    m.local(original).map_or([0; 4], |p| child.pixel(p))
                } else {
                    m.pixel(original)
                }
            }
            PageObject::Text { .. } => self.text_contours[index].pixel(original),
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
                        || cell[0] * cell[0] + cell[1] * cell[1] <= coverage / std::f64::consts::PI)
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
        }
    }
    pub(crate) fn world_pixel(&self, index: usize, original: Point) -> [u8; 4] {
        if self.comic_clips
            && crate::comics::coverage(
                self.doc(),
                crate::layer_search::Key::World(self.doc().scene.catalog.worlds.id(index)),
                original,
            ) == 0.
        {
            return [0; 4];
        }
        if let Some(c) = self.doc().scene.file_objects.pixel(
            crate::layer_search::Key::World(self.doc().scene.catalog.worlds.id(index)),
            original,
        ) {
            return c;
        }
        let Some(camera) = &self.doc().scene.camera else {
            return [0; 4];
        };
        if !camera.page_bounds.contains(original)
            && !self.doc().scene.comics.masks.iter().any(|m| {
                m.target
                    == crate::layer_search::Key::World(self.doc().scene.catalog.worlds.id(index))
            })
        {
            return [0; 4];
        }
        let Some(mesh) = self.doc().scene.worlds.get(index) else {
            return [0; 4];
        };
        let center = [
            (camera.page_bounds.min[0] + camera.page_bounds.max[0]) / 2.,
            (camera.page_bounds.min[1] + camera.page_bounds.max[1]) / 2.,
        ];
        let project = |v: [f64; 3]| {
            [
                center[0] + (v[0] - camera.position[0]) * camera.pixels_per_unit,
                center[1] - (v[1] - camera.position[1]) * camera.pixels_per_unit,
            ]
        };
        if mesh.edges.iter().any(|e| {
            distance(
                original,
                project(mesh.vertices[e[0]]),
                project(mesh.vertices[e[1]]),
            ) <= 0.5
        }) {
            mesh.color
        } else {
            [0; 4]
        }
    }
    /// Isolated page-source contribution; the normal layer tree handles its own blending.
    pub fn source_pixel(&self, key: crate::layer_search::Key, point: Point) -> [u8; 4] {
        let d = self.doc();
        if point[0] < 0.
            || point[1] < 0.
            || point[0] >= d.width as f64
            || point[1] >= d.height as f64
        {
            return [0; 4];
        }
        self.source_pixel_unbounded(key, point)
    }
    pub(crate) fn source_pixel_unbounded(
        &self,
        key: crate::layer_search::Key,
        point: Point,
    ) -> [u8; 4] {
        if self.error().is_some() {
            return [0; 4];
        }
        if self.comic_clips
            && crate::comics::coverage(
                self.doc(),
                key,
                self.doc().scene.transform.inverse().map(point),
            ) == 0.
        {
            return [0; 4];
        }
        if let Ok(Some(renderer)) = self.float_renderer() {
            return renderer
                .source_pixel(key, point)
                .map_or([0; 4], |p| p.rgba8());
        }
        use crate::{layer_properties::Target, layer_search::Key};
        let d = self.doc();
        if point.iter().any(|n| !n.is_finite()) {
            return [0; 4];
        }
        let original = d.scene.transform.inverse().map(point);
        let raw = match key {
            Key::Object(_) => {
                if d.scene.clipping.solo.is_some()
                    || d.scene.clips.iter().any(|r| !inside(r, original))
                {
                    return [0; 4];
                }
                let Some(n) = crate::layer_catalog::source_index(d, key) else {
                    return [0; 4];
                };
                self.object_pixel(n, &d.scene.objects[n], original)
            }
            Key::World(_) => {
                if d.scene.clipping.solo.is_some()
                    || d.scene.clips.iter().any(|r| !inside(r, original))
                {
                    return [0; 4];
                }
                let Some(n) = crate::layer_catalog::source_index(d, key) else {
                    return [0; 4];
                };
                self.world_pixel(n, original)
            }
            Key::Gradient(id) => {
                if d.scene.clipping.solo.is_some()
                    || d.scene.clips.iter().any(|r| !inside(r, original))
                {
                    return [0; 4];
                }
                let Some(g) = d.scene.gradients.layers.iter().find(|g| g.id == id) else {
                    return [0; 4];
                };
                d.scene
                    .file_objects
                    .pixel(key, original)
                    .unwrap_or_else(|| g.pixel(original))
            }
            Key::Selection(_) => return [0; 4],
            _ => {
                let Some(t) = crate::layer_catalog::target(d, key) else {
                    return [0; 4];
                };
                return match t {
                    Target::Raster(id) => crate::layers::index(d, id).map_or([0; 4], |n| {
                        d.scene.paper.clipped_node_pixel_with(
                            d,
                            crate::paper::Node::Raster(n),
                            point.map(|n| n.floor() as i64),
                            &mut |n, _| self.raster_pixel(n, point.map(|n| n.floor() as i64)),
                        )
                    }),
                    Target::Folder(n) => d.scene.paper.clipped_node_pixel_with(
                        d,
                        crate::paper::Node::Folder(n as usize),
                        point.map(|n| n.floor() as i64),
                        &mut |n, _| self.raster_pixel(n, point.map(|n| n.floor() as i64)),
                    ),
                    Target::Cell { track, cell } => {
                        let Some(c) = d
                            .animation
                            .as_ref()
                            .and_then(|a| a.tracks.get(track as usize))
                            .and_then(|t| t.cells.get(cell as usize))
                        else {
                            return [0; 4];
                        };
                        let track_id = d.scene.catalog.tracks.id(track as usize);
                        let track_key = Key::Track(track_id);
                        let mut value = d
                            .scene
                            .file_objects
                            .pixel(track_key, original)
                            .or_else(|| d.scene.file_objects.pixel(key, original))
                            .unwrap_or_else(|| {
                                c.pixel(point[0].floor() as i64, point[1].floor() as i64)
                            });
                        value[3] = (value[3] as f64
                            * if d.scene.file_objects.get(track_key).is_some() {
                                1.
                            } else {
                                crate::layer_properties::factor(d, t)
                            }
                            * crate::layer_properties::factor(d, Target::Track(track)))
                        .round() as u8;
                        value
                    }
                    Target::Track(track) => {
                        let Some((_, cell, _)) = d.animation.as_ref().and_then(|a| {
                            a.indexed_visible_cells(d.current_frame)
                                .find(|(n, _, _)| *n == track as usize)
                        }) else {
                            return [0; 4];
                        };
                        let id = d.scene.catalog.tracks.id(track as usize);
                        self.source_pixel(
                            Key::Cell {
                                track: id,
                                cell: d.scene.catalog.cell_id(id, cell),
                            },
                            point,
                        )
                    }
                    Target::Vector(id) => {
                        if d.scene.clips.iter().any(|r| !inside(r, original)) {
                            return [0; 4];
                        }
                        let Some(l) = d.scene.lines.layers.iter().find(|l| l.id == id) else {
                            return [0; 4];
                        };
                        if l.hidden {
                            return [0; 4];
                        }
                        let Some(index) = d.scene.lines.layers.iter().position(|v| v.id == id)
                        else {
                            return [0; 4];
                        };
                        let mut value = self.vector_pixel(index, point);
                        if let Some(base) = d.scene.clipping.binding(t) {
                            if !crate::clipping::active(d, t) {
                                return [0; 4];
                            }
                            let Target::Vector(base) = base else {
                                return [0; 4];
                            };
                            let Some(index) =
                                d.scene.lines.layers.iter().position(|v| v.id == base)
                            else {
                                return [0; 4];
                            };
                            let weight = self.vector_pixel(index, point)[3] as f64 / 255.
                                * crate::layer_properties::factor(d, Target::Vector(base))
                                * d.scene.layer_effects.mask(Target::Vector(base), original) as f64;
                            value[3] = (value[3] as f64 * weight).round() as u8;
                        }
                        value[3] = (value[3] as f64
                            * crate::layer_properties::factor(d, t)
                            * d.scene.layer_effects.mask(t, original) as f64)
                            .round() as u8;
                        value
                    }
                    _ => [0; 4],
                };
            }
        };
        crate::layer_catalog::compose(d, key, [0; 4], raw)
    }

    pub fn new(document: &'a Document) -> Self {
        Self::preview(document, None)
    }
    pub(crate) fn fill(
        document: &'a Document,
        active: usize,
        settings: crate::bucket::Settings,
    ) -> Result<Self, String> {
        let mut r = Self::new(document);
        r.owned = Some(crate::layer_roles::reference_view(
            document, active, settings,
        )?);
        r.fill_graph = Some(crate::fill_reference::graph(r.doc(), active, settings));
        r.fill = Some((active, settings));
        Ok(r)
    }
    pub fn sampling(document: &'a Document, references: &'a crate::eyedropper::References) -> Self {
        let mut r = Self::new(document);
        if references.active(document) {
            r.references = Some(references);
        }
        r
    }
    pub fn preview(document: &'a Document, stroke: Option<&'a crate::painting::Stroke>) -> Self {
        if document.color != crate::color_storage::Config::legacy()
            || !document.scene.surface_effects.rasters.is_empty()
            || crate::local_color::active(&document.scene)
        {
            let mut renderer = Self::legacy_build(document, stroke, false);
            renderer.float_plan =
                crate::float_render::Renderer::prepare_plan(document, stroke).map(Some);
            renderer
        } else {
            Self::legacy_preview(document, stroke)
        }
    }
    pub(crate) fn legacy_preview(
        document: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
    ) -> Self {
        Self::legacy_build(document, stroke, true)
    }
    pub(crate) fn geometry_preview(document: &'a Document) -> Self {
        Self::legacy_build(document, None, false)
    }
    fn legacy_build(
        document: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
        caches: bool,
    ) -> Self {
        let mut text_error = None;
        let text_contours = document
            .scene
            .objects
            .iter()
            .map(|object| match object {
                PageObject::Text {
                    baseline,
                    text,
                    size_px,
                    layout,
                    color,
                } => match text_paint::Text::new(
                    *baseline,
                    text,
                    *size_px,
                    *color,
                    layout.as_deref(),
                ) {
                    Ok(paths) => paths,
                    Err(error) => {
                        text_error = Some(error);
                        text_paint::Text::default()
                    }
                },
                _ => text_paint::Text::default(),
            })
            .collect();
        Self {
            page_parts: if caches {
                document
                    .scene
                    .page_parts
                    .entries
                    .iter()
                    .filter_map(|p| p.document().map(|d| (p.object, Box::new(Renderer::new(d)))))
                    .collect()
            } else {
                vec![]
            },
            float_plan: Ok(None),
            document,
            owned: None,
            appearance: if caches {
                crate::appearance::Caches::new(document, stroke)
            } else {
                Ok(crate::appearance::Caches {
                    rasters: vec![],
                    vectors: vec![],
                })
            },
            stroke,
            references: None,
            fill: None,
            fill_graph: None,
            text_contours,
            comic_geometry: document
                .scene
                .objects
                .iter()
                .map(|o| match o {
                    PageObject::Comic(c) => {
                        Some(crate::comic_object::Geometry::new(c, &document.scene))
                    }
                    _ => None,
                })
                .collect(),
            text_error,
            export_page: false,
            comic_clips: crate::comics::has_clips(&document.scene),
            deferred: (0..document.layers.len())
                .map(|n| {
                    document
                        .scene
                        .render_order
                        .deferred(document.scene.layer_ids.id(n))
                })
                .collect(),
            order: if document.scene.render_order.is_default() {
                vec![]
            } else {
                crate::render_order::normalized(&document.scene)
            },
            vector_bounds: document
                .scene
                .lines
                .layers
                .iter()
                .map(|l| {
                    l.paths
                        .iter()
                        .map(crate::line_tools::Path::bounds)
                        .collect()
                })
                .collect(),
        }
    }
    pub fn pixel(&self, point: Point) -> [u8; 4] {
        let pixel = self.raw_pixel(point);
        if self.export_page {
            over(
                pixel,
                self.doc().scene.comics.page.output_pixel(self.doc(), point),
            )
        } else {
            pixel
        }
    }
    fn raw_pixel(&self, point: Point) -> [u8; 4] {
        if self.error().is_some() {
            return [0; 4];
        }
        if let Ok(Some(renderer)) = self.float_renderer() {
            return renderer.pixel(point).map_or([0; 4], |p| p.rgba8());
        }
        let doc = self.doc();
        if point[0] < 0.0
            || point[1] < 0.0
            || point[0] >= doc.width as f64
            || point[1] >= doc.height as f64
        {
            return [0; 4];
        }
        let x = point[0] as u32;
        let y = point[1] as u32;
        let mut result = doc.composite_pixel_graph(
            x,
            y,
            self.references.is_none_or(|r| r.animation)
                && self.fill.is_none_or(|(_, s)| {
                    matches!(
                        s.reference,
                        crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                    )
                }),
            self.fill_graph.as_ref().unwrap_or(&doc.scene.paper),
            |index, _layer| {
                if self.deferred.get(index).copied().unwrap_or(false) {
                    return [0; 4];
                }
                if self.references.is_some_and(|r| !r.raster(index))
                    || self
                        .fill
                        .is_some_and(|(a, s)| !crate::fill_reference::raster(doc, index, a, s))
                {
                    return [0; 4];
                }
                self.raster_pixel(index, [x as i64, y as i64])
            },
        );
        let scene = &doc.scene;
        let original = scene.transform.inverse().map(point);
        if scene.clips.iter().any(|clip| !inside(clip, original)) {
            return result;
        }
        if !self.order.is_empty() {
            return self.ordered(result, point, original);
        }
        if scene.clipping.solo.is_none()
            && self.references.is_none_or(|r| r.objects)
            && self.fill.is_none_or(|(_, s)| {
                matches!(
                    s.reference,
                    crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                )
            })
        {
            for layer in &scene.gradients.layers {
                result = crate::layer_catalog::compose(
                    doc,
                    crate::layer_search::Key::Gradient(layer.id),
                    result,
                    scene
                        .file_objects
                        .pixel(crate::layer_search::Key::Gradient(layer.id), original)
                        .unwrap_or_else(|| layer.pixel(original)),
                );
            }
        }
        for (index, object) in scene.objects.iter().enumerate().filter(|(n, _)| {
            scene.clipping.solo.is_none()
                && self.references.is_none_or(|r| r.objects)
                && self.fill.is_none_or(|(_, s)| {
                    matches!(
                        s.reference,
                        crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                    ) && !crate::layer_properties::properties(
                        doc,
                        crate::layer_properties::Target::Object(scene.catalog.objects.id(*n)),
                    )
                    .hidden
                })
        }) {
            let color = self.object_pixel(index, object, original);
            result = crate::layer_catalog::compose(
                doc,
                crate::layer_search::Key::Object(scene.catalog.objects.id(index)),
                result,
                color,
            );
        }
        if !scene.clipping.vectors.is_empty()
            || crate::layer_blend::vectors_active(doc)
            || !doc.scene.layer_effects.vectors.is_empty()
            || doc.scene.file_objects.entries.iter().any(|l| {
                matches!(
                    l.target,
                    crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(_))
                )
            })
        {
            result = crate::paper::clipped_vectors(
                doc,
                point,
                original,
                result,
                |index, layer| {
                    self.references.is_none_or(|r| r.vectors)
                        && self
                            .fill
                            .is_none_or(|(_, s)| crate::fill_reference::vector(layer, index, s))
                },
                self.fill.is_some_and(|(_, s)| s.advanced.vector_centerline),
                |index| {
                    if let Some(l) = doc.scene.lines.layers.get(index)
                        && let Some(c) = doc.scene.file_objects.pixel(
                            crate::layer_search::Key::Layer(
                                crate::layer_properties::Target::Vector(l.id),
                            ),
                            original,
                        )
                    {
                        return Some(c);
                    }
                    self.appearance
                        .as_ref()
                        .ok()
                        .and_then(|c| c.vectors.get(index))
                        .and_then(|c| c.as_ref())
                        .map(|c| c.pixel(point.map(|v| v.floor() as i64)))
                },
            );
        } else {
            for (index, (layer, bounds)) in scene
                .lines
                .layers
                .iter()
                .zip(&self.vector_bounds)
                .enumerate()
            {
                let opacity = crate::layer_properties::factor(
                    doc,
                    crate::layer_properties::Target::Vector(layer.id),
                );
                if !layer.hidden
                    && opacity > 0.0
                    && self.references.is_none_or(|r| r.vectors)
                    && self
                        .fill
                        .is_none_or(|(_, s)| crate::fill_reference::vector(layer, index, s))
                {
                    let mut vector_result = if opacity == 1.0 { result } else { [0; 4] };
                    if let Some(c) = self
                        .appearance
                        .as_ref()
                        .ok()
                        .and_then(|c| c.vectors.get(index))
                        .and_then(|c| c.as_ref())
                    {
                        vector_result =
                            over(vector_result, c.pixel(point.map(|v| v.floor() as i64)));
                    } else {
                        for (path, bounds) in layer.paths.iter().zip(bounds) {
                            if self.fill.is_some_and(|(_, s)| s.advanced.vector_centerline) {
                                if path.segments().any(|(a, b)| {
                                    distance(point, scene.transform.map(a), scene.transform.map(b))
                                        <= 0.5
                                }) {
                                    vector_result = over(
                                        vector_result,
                                        if path.stroke {
                                            path.color
                                        } else {
                                            path.fill.unwrap_or(path.color)
                                        },
                                    );
                                }
                                continue;
                            }
                            if (0..2).all(|i| {
                                original[i] >= bounds.min[i] && original[i] <= bounds.max[i]
                            }) {
                                vector_result = over(vector_result, path.pixel(original));
                            }
                        }
                    }
                    result = if opacity == 1.0 {
                        vector_result
                    } else {
                        crate::brush_composite::Mode::Normal.compose(
                            result,
                            vector_result,
                            opacity,
                            original.map(|p| p.floor() as i64),
                            0,
                        )
                    };
                }
            }
        }
        if scene.clipping.solo.is_none()
            && self.references.is_none_or(|r| r.objects)
            && self.fill.is_none_or(|(_, s)| {
                matches!(
                    s.reference,
                    crate::bucket::Reference::Composite | crate::bucket::Reference::Marked
                )
            })
        {
            for index in 0..scene.worlds.len() {
                result = crate::layer_catalog::compose(
                    doc,
                    crate::layer_search::Key::World(scene.catalog.worlds.id(index)),
                    result,
                    self.world_pixel(index, original),
                );
            }
        }
        result
    }
    /// Premultiplied box sampling for fine comic tones at reduced view/output sizes.
    pub fn sample_box(&self, point: Point, size: Point) -> [u8; 4] {
        let mut sum = [0.; 4];
        for y in 0..4 {
            for x in 0..4 {
                let p = self.pixel([
                    point[0] + ((x as f64 + 0.5) / 4. - 0.5) * size[0],
                    point[1] + ((y as f64 + 0.5) / 4. - 0.5) * size[1],
                ]);
                let a = p[3] as f64 / 255.;
                for n in 0..3 {
                    sum[n] += p[n] as f64 * a;
                }
                sum[3] += a;
            }
        }
        if sum[3] == 0. {
            return [0; 4];
        }
        [
            (sum[0] / sum[3]).round() as u8,
            (sum[1] / sum[3]).round() as u8,
            (sum[2] / sum[3]).round() as u8,
            (sum[3] * 255. / 16.).round() as u8,
        ]
    }
    /// Output at a requested DPI while preserving the document's physical size.
    /// Returns RGBA pixels; output file codecs are separate checklist features.
    pub fn output(&self, dpi: f64, budget: usize) -> Result<(u32, u32, Vec<[u8; 4]>), String> {
        if let Some(e) = self.error() {
            return Err(e.into());
        }
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
        let tone=ratio<1.&&self.doc().scene.objects.iter().any(|o|matches!(o,PageObject::Comic(c)if matches!(c.kind,crate::comics::Kind::Tone(_))));
        for y in 0..height {
            for x in 0..width {
                let point = [(x as f64 + 0.5) / ratio, (y as f64 + 0.5) / ratio];
                pixels.push(if tone {
                    self.sample_box(point, [1. / ratio; 2])
                } else {
                    self.pixel(point)
                });
            }
        }
        Ok((width, height, pixels))
    }
}
pub(crate) fn over(destination: [u8; 4], source: [u8; 4]) -> [u8; 4] {
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
pub(crate) fn inside(path: &[Point], point: Point) -> bool {
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
pub(crate) fn glyph_paths(
    face: &ttf_parser::Face<'_>,
    id: ttf_parser::GlyphId,
    origin: Point,
    size: f64,
) -> Vec<Vec<Point>> {
    let mut outline = Outline {
        paths: vec![],
        current: [0.; 2],
        scale: size / f64::from(face.units_per_em()),
        origin,
    };
    face.outline_glyph(id, &mut outline);
    outline.paths
}
fn glyph_contours(
    baseline: Point,
    text: &str,
    size: f64,
    layout: Option<&crate::text_layout::Style>,
) -> Result<Vec<Contour>, String> {
    if let Some(style) = layout {
        return Ok(contour_paths(
            crate::text_layout::layout(baseline, text, size, style)?.paths,
        ));
    }
    Ok(contour_paths(legacy_text_paths(baseline, text, size)))
}
pub(crate) fn legacy_text_paths(baseline: Point, text: &str, size: f64) -> Vec<Vec<Point>> {
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
    outline.paths
}

fn contour_paths(paths: Vec<Vec<Point>>) -> Vec<Contour> {
    paths
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

/// Bounds of occupied glyph pixels on the document grid, independent of other layers.
pub fn text_pixel_bounds(doc: &Document, index: usize) -> Result<Rect, String> {
    text_pixel_bounds_budget(doc, index, &mut (64 * 1024 * 1024))
}
pub(crate) fn text_pixel_bounds_budget(
    doc: &Document,
    index: usize,
    remaining: &mut usize,
) -> Result<Rect, String> {
    let Some(PageObject::Text {
        baseline,
        text,
        size_px,
        color,
        layout,
    }) = doc.scene.objects.get(index)
    else {
        return Err("Choose a text object".into());
    };
    if color[3] == 0 && layout.as_ref().is_none_or(|s| s.rich.is_none()) {
        return Err("Text contains no visible pixels".into());
    }
    let paths = glyph_contours(*baseline, text, *size_px, layout.as_deref())?;
    let points = paths
        .iter()
        .flat_map(|p| &p.points)
        .map(|p| doc.scene.transform.map(*p));
    let geometry = text_paint::Text::new(*baseline, text, *size_px, *color, layout.as_deref())?;
    let bounds = if layout.as_ref().is_some_and(|s| s.rich.is_some()) {
        let b = geometry.bounds().ok_or("Text contains no visible pixels")?;
        crate::alignment_detail::rectangle(
            [
                [b.min[0], b.min[1]],
                [b.max[0], b.min[1]],
                [b.max[0], b.max[1]],
                [b.min[0], b.max[1]],
            ]
            .into_iter()
            .map(|p| doc.scene.transform.map(p)),
        )?
    } else {
        crate::alignment_detail::rectangle(points)?
    };
    let x0 = bounds.min[0].floor() as i64;
    let y0 = bounds.min[1].floor() as i64;
    let x1 = bounds.max[0].ceil() as i64;
    let y1 = bounds.max[1].ceil() as i64;
    if x1 - x0 > 32768 || y1 - y0 > 32768 {
        return Err("Text pixel bounds dimensions limit".into());
    }
    let count = (x1 - x0).max(0) as usize * (y1 - y0).max(0) as usize;
    let segments: usize = paths.iter().map(|p| p.points.len()).sum();
    if count > 4 * 1024 * 1024 || count.saturating_mul(segments) > 64 * 1024 * 1024 {
        return Err("Text pixel bounds work limit".into());
    }
    *remaining = remaining
        .checked_sub(count.saturating_mul(segments + doc.scene.clips.len() * 4))
        .ok_or("Alignment glyph bounds work limit")?;
    let inv = doc.scene.transform.inverse();
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let p = inv.map([x as f64 + 0.5, y as f64 + 0.5]);
            if !doc.scene.clips.iter().any(|clip| !inside(clip, p)) && geometry.pixel(p)[3] > 0 {
                r.min[0] = r.min[0].min(x as f64);
                r.min[1] = r.min[1].min(y as f64);
                r.max[0] = r.max[0].max(x as f64 + 1.0);
                r.max[1] = r.max[1].max(y as f64 + 1.0);
            }
        }
    }
    if r.valid() {
        Ok(r)
    } else {
        Err("Text contains no visible pixels".into())
    }
}
