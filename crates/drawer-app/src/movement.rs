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
    target: Target,
    origin: Point,
    total: [i64; 2],
    engine: Nudge,
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
        let layer = if self.move_target == Target::Boundary {
            layer.unwrap_or(0)
        } else {
            layer.ok_or("Select a raster layer")?
        };
        let engine = Nudge::begin(doc, layer, self.move_target)?;
        self.movement = Some(Gesture {
            document: doc.id,
            revision: doc.revision,
            view,
            projection,
            source,
            root,
            layer,
            target: self.move_target,
            origin: projection.point(pos),
            total: [0; 2],
            engine,
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
            || doc.view != s.projection.view
            || self.mode != super::super::precision::Mode::Move
            || s.target != self.move_target
            || (s.target != Target::Boundary && self.move_layer != Some(s.layer))
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
            let delta = [p[0] - s.origin[0], p[1] - s.origin[1]];
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
            if let Err(e) = s
                .engine
                .preview_total(doc, s.total)
                .and_then(|state| doc.commit_state(state))
            {
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
            let state = s.engine.preview_total(doc, s.total)?;
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
                || self.move_target != s.target
                || (s.target != Target::Boundary && self.move_layer != Some(s.layer))
                || documents
                    .iter()
                    .find(|d| d.id == s.document)
                    .is_none_or(|d| d.revision != s.revision || d.view != s.projection.view)
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
fn movement_revision(total: [i64; 2]) -> u64 {
    ((total[0] + 1_000_000) as u64) * 2_000_001 + (total[1] + 1_000_000) as u64
}
