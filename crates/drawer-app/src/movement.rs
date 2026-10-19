//! Pointer movement shares the precision engine and commits only on release.
use super::*;
use drawer_core::precision::{Nudge, Target};
pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    pub projection: Projection,
    revision: u64,
    source: drawer_core::pointer_input::Source,
    root: egui::Rect,
    layer: usize,
    key: Option<drawer_core::layer_search::Key>,
    frame: u32,
    target: Target,
    origin: Point,
    total: [i64; 2],
    engine: Option<Nudge>,
    geometry: drawer_core::object_snap::Geometry,
    bounds: drawer_core::scene::Rect,
    snapping: drawer_core::snapping::Settings,
    snap_work: usize,
    preview: Option<Document>,
}
impl Drawing {
    pub(crate) fn movement_preview(&self, document: u64) -> Option<&Document> {
        self.movement
            .as_ref()
            .filter(|s| s.document == document)
            .and_then(|s| s.preview.as_ref())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_movement(
        &mut self,
        doc: &Document,
        view: u64,
        layer: Option<usize>,
        source: drawer_core::pointer_input::Source,
        projection: Projection,
        root: egui::Rect,
        pos: egui::Pos2,
    ) -> Result<(), String> {
        let key = (self.move_target == Target::Layer)
            .then_some(self.move_source)
            .flatten()
            .filter(|k| {
                !matches!(
                    k,
                    drawer_core::layer_search::Key::Layer(
                        drawer_core::layer_properties::Target::Raster(_)
                    )
                )
            });
        let layer = if self.move_target == Target::Boundary || key.is_some() {
            layer.unwrap_or(0)
        } else {
            layer.ok_or("Select a raster layer")?
        };
        let engine = if let Some(key) = key {
            drawer_core::search_move::prepare(doc, key, [0; 2])?;
            None
        } else {
            Some(Nudge::begin(doc, layer, self.move_target)?)
        };
        let excluded = match key {
            Some(drawer_core::layer_search::Key::Object(id)) => doc
                .scene
                .catalog
                .objects
                .index(id, doc.scene.objects.len())
                .map(drawer_core::alignment::Item::Object)
                .into_iter()
                .collect(),
            Some(drawer_core::layer_search::Key::Gradient(id)) => {
                vec![drawer_core::alignment::Item::Gradient(id)]
            }
            Some(drawer_core::layer_search::Key::Layer(
                drawer_core::layer_properties::Target::Vector(id),
            )) => doc
                .scene
                .lines
                .layers
                .iter()
                .filter(|l| l.id == id)
                .flat_map(|l| {
                    l.paths
                        .iter()
                        .map(|p| drawer_core::alignment::Item::Vector(p.id))
                })
                .collect(),
            Some(_) => vec![],
            None => vec![drawer_core::alignment::Item::Raster(layer)],
        };
        let geometry = drawer_core::object_snap::Geometry::new(doc, self.snap_settings, &excluded)?;
        let bounds = if let Some(key) = key {
            drawer_core::layer_catalog::bounds(doc, key).ok_or("Source has no canvas bounds")?
        } else {
            drawer_core::alignment::bounds(doc, drawer_core::alignment::Item::Raster(layer))?
        };
        self.movement = Some(Gesture {
            document: doc.id,
            revision: doc.revision,
            view,
            projection,
            source,
            root,
            layer,
            key,
            frame: doc.current_frame,
            target: self.move_target,
            origin: projection.point(pos),
            total: [0; 2],
            engine,
            geometry,
            bounds,
            snapping: self.snap_settings,
            snap_work: 0,
            preview: None,
        });
        Ok(())
    }
    pub(super) fn receive_movement(&mut self, index: usize, doc: &mut Document) {
        use drawer_core::pointer_input::Phase;
        if self.events[index].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(event) = self.events[index].0 else {
            return;
        };
        let Some(mut s) = self.movement.take() else {
            return;
        };
        if s.document != doc.id || s.source != event.source {
            self.movement = Some(s);
            return;
        }
        self.events[index].1 = true;
        if s.revision != doc.revision
            || s.frame != doc.current_frame
            || doc.view != s.projection.view
            || self.mode != super::super::precision::Mode::Move
            || s.target != self.move_target
            || !s.selected(self)
            || event.phase == Phase::Cancel
        {
            self.ownership.clear();
            return;
        }
        if matches!(event.phase, Phase::Move | Phase::Up) {
            let p = s.projection.point(egui::pos2(
                event.position[0] as f32,
                event.position[1] as f32,
            ));
            let mut delta = [p[0] - s.origin[0], p[1] - s.origin[1]];
            if s.target == Target::Layer && s.key.is_none() && !self.snap_disabled {
                s.snap_work = s
                    .snap_work
                    .saturating_add(s.geometry.work().saturating_mul(5));
                if s.snap_work > 64 * 1024 * 1024 {
                    self.error = Some("Movement snapping work limit".into());
                    self.ownership.clear();
                    return;
                }
                delta = s.geometry.movement(
                    s.bounds,
                    delta,
                    s.snapping.screen_distance * s.projection.ppp / s.projection.view.zoom,
                );
            }
            if delta
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
            {
                self.error = Some("Movement coordinate limit".into());
                self.ownership.clear();
                return;
            }
            s.total = delta.map(|v| v.round() as i64);
        }
        if event.phase == Phase::Up {
            self.ownership.clear();
            if let Err(e) = s.state(doc).and_then(|state| doc.commit_state(state)) {
                self.error = Some(e);
            }
            return;
        }
        self.movement = Some(s);
    }
    pub(crate) fn refresh_movement(&mut self, doc: &Document) {
        let Some(s) = self.movement.as_mut().filter(|s| s.document == doc.id) else {
            return;
        };
        if s.preview
            .as_ref()
            .is_some_and(|p| p.revision == movement_revision(s.total))
        {
            return;
        }
        let result = (|| {
            let state = s.state(doc)?;
            let mut p = Document::create(
                CanvasSpec {
                    width: 1,
                    height: 1,
                    dpi: state.dpi,
                    background: Background::Transparent,
                },
                DEFAULT_DOCUMENT_BUDGET,
            )
            .map_err(|e| e.to_string())?;
            p.width = state.width;
            p.height = state.height;
            p.layers = state.layers;
            p.scene = state.scene;
            p.animation = state.animation;
            p.view = doc.view;
            p.tool = doc.tool;
            p.current_frame = doc.current_frame;
            p.revision = movement_revision(s.total);
            Ok::<_, String>(p)
        })();
        match result {
            Ok(p) => s.preview = Some(p),
            Err(e) => {
                self.error = Some(e);
                self.cancel();
            }
        }
    }
    pub(super) fn advance_movement(&mut self, ctx: &egui::Context, documents: &mut [Document]) {
        if self.movement.as_ref().is_some_and(|s| {
            self.mode != super::super::precision::Mode::Move
                || self.snap_settings != s.snapping
                || self.move_target != s.target
                || !s.selected(self)
                || documents
                    .iter()
                    .find(|d| d.id == s.document)
                    .is_none_or(|d| {
                        d.revision != s.revision
                            || d.current_frame != s.frame
                            || d.view != s.projection.view
                    })
        }) {
            self.cancel();
            return;
        }
        if self.movement.as_ref().is_some_and(|s| {
            s.root != ctx.content_rect()
                || (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
        }) {
            self.cancel();
            return;
        }
        for n in 0..self.events.len() {
            let Some(id) = self.movement.as_ref().map(|s| s.document) else {
                break;
            };
            let Some(doc) = documents.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive_movement(n, doc);
        }
        if let Some(id) = self.movement.as_ref().map(|s| s.document)
            && let Some(doc) = documents.iter().find(|d| d.id == id)
        {
            self.refresh_movement(doc);
        }
    }
}
impl Gesture {
    fn selected(&self, d: &Drawing) -> bool {
        if let Some(k) = self.key {
            d.move_source == Some(k)
        } else {
            self.target == Target::Boundary || d.move_layer == Some(self.layer)
        }
    }
    fn state(&self, d: &Document) -> Result<drawer_core::sizing::DocumentState, String> {
        if let Some(e) = &self.engine {
            e.preview_total(d, self.total)
        } else {
            drawer_core::search_move::prepare(
                d,
                self.key.ok_or("Missing movement source")?,
                self.total,
            )
        }
    }
}
fn movement_revision(total: [i64; 2]) -> u64 {
    ((total[0] + 1_000_000) as u64) * 2_000_001 + (total[1] + 1_000_000) as u64
}
