use crate::{document::Document, sizing::DocumentState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelChange {
    pub offset: usize,
    pub before: [u8; 4],
    pub after: [u8; 4],
}
#[derive(Clone)]
enum EditKind {
    Multi {
        layers: Vec<(usize, Vec<PixelChange>)>,
        scene: Option<(Box<crate::scene::Scene>, Box<crate::scene::Scene>)>,
    },
    Pixels {
        layer: usize,
        changes: Vec<PixelChange>,
        scene: Option<(Box<crate::scene::Scene>, Box<crate::scene::Scene>)>,
    },
    Pixel {
        layer: usize,
        offset: usize,
        before: [u8; 4],
        after: [u8; 4],
    },
    State {
        before: Box<DocumentState>,
        after: Box<DocumentState>,
    },
    Scene {
        before: Box<crate::scene::Scene>,
        after: Box<crate::scene::Scene>,
    },
}
#[derive(Clone)]
struct Edit {
    cached: Option<usize>,
    label: Option<String>,
    group: Option<u64>,
    kind: EditKind,
    from: u64,
    to: u64,
}
impl Edit {
    fn bytes(&self) -> usize {
        self.label.as_ref().map_or(0, String::len)
            + self.cached.unwrap_or_else(|| self.kind_bytes())
    }
    fn kind_bytes(&self) -> usize {
        match &self.kind {
            EditKind::Multi { layers, scene } => {
                layers
                    .iter()
                    .map(|(_, p)| p.capacity() * std::mem::size_of::<PixelChange>())
                    .sum::<usize>()
                    + std::mem::size_of::<Self>()
                    + scene
                        .as_ref()
                        .map_or(0, |(a, b)| a.size_bytes() + b.size_bytes())
            }
            EditKind::Pixels { changes, scene, .. } => {
                changes.capacity() * std::mem::size_of::<PixelChange>()
                    + std::mem::size_of::<Self>()
                    + scene
                        .as_ref()
                        .map_or(0, |(a, b)| a.size_bytes() + b.size_bytes())
            }
            EditKind::Pixel { .. } => std::mem::size_of::<Self>(),
            EditKind::State { before, after } => crate::tiled_raster::history_bytes(before, after),
            EditKind::Scene { before, after } => {
                before.size_bytes() + after.size_bytes() + std::mem::size_of::<Self>()
            }
        }
    }
}
#[derive(Default, Clone)]
pub(crate) struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    current: u64,
    next: u64,
    saved: Option<u64>,
    meta: u64,
    saved_meta: u64,
    save_serial: u64,
    saved_serial: u64,
    inflight: std::collections::BTreeMap<u64, u64>,
    limits: limits::Settings,
    last_group: Option<std::time::Instant>,
    pruned: u64,
}
impl History {
    pub(crate) fn mark_saved(&mut self) {
        self.saved = Some(self.current);
        self.saved_meta = self.meta;
        self.save_serial = self.save_serial.saturating_add(1);
        self.saved_serial = self.save_serial;
    }
    fn push(&mut self, kind: EditKind) {
        self.next += 1;
        let mut edit = Edit {
            cached: None,
            label: None,
            group: None,
            kind,
            from: self.current,
            to: self.next,
        };
        edit.cached = Some(edit.kind_bytes());
        self.last_group = None;
        self.current = edit.to;
        self.redo.clear();
        self.undo.push(edit);
        while self.undo.len() > self.limits.steps as usize
            || self.undo.iter().map(Edit::bytes).sum::<usize>() > self.limits.bytes()
        {
            self.undo.remove(0);
            self.pruned = self.pruned.saturating_add(1);
        }
    }
}
fn staged_history_tiles(
    doc: &Document,
    edit: &Edit,
    before: bool,
) -> Result<std::collections::BTreeMap<usize, crate::tiled_raster::Image>, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut budget = crate::document::DEFAULT_DOCUMENT_BUDGET
        .saturating_sub(doc.resident_bytes())
        .saturating_sub(edit.bytes());
    let mut stage = |layer: usize, changes: &[PixelChange]| -> Result<(), String> {
        let raster = &doc.layers[layer];
        if let Some(image) = raster.prepare_pixels(
            changes
                .iter()
                .map(|p| (p.offset, if before { p.before } else { p.after })),
            budget
                .saturating_add(raster.allocation_bytes())
                .min(doc.tile_budget(layer)),
        )? {
            budget = budget.saturating_sub(image.bytes().saturating_sub(raster.allocation_bytes()));
            out.insert(layer, image);
        }
        Ok(())
    };
    match &edit.kind {
        EditKind::Multi { layers, .. } => {
            for (layer, pixels) in layers {
                stage(*layer, pixels)?;
            }
        }
        EditKind::Pixels { layer, changes, .. } => stage(*layer, changes)?,
        EditKind::Pixel {
            layer,
            offset,
            before,
            after,
        } => stage(
            *layer,
            &[PixelChange {
                offset: *offset,
                before: *before,
                after: *after,
            }],
        )?,
        _ => {}
    }
    Ok(out)
}
impl Document {
    pub(crate) fn commit_multi(
        &mut self,
        mut layers: Vec<(usize, Vec<PixelChange>)>,
        after: Option<crate::scene::Scene>,
    ) -> Result<bool, String> {
        if self.color != crate::color_storage::Config::legacy() {
            return Err("This pixel operation requires its high precision edit path".into());
        }
        if after
            .as_ref()
            .is_some_and(|s| !s.valid() || !s.paper.valid_for(self.layers.len()))
        {
            return Err("Invalid multi-layer scene".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for (n, changes) in &mut layers {
            let layer = self.layers.get(*n).ok_or("Missing raster target")?;
            if !seen.insert(*n)
                || layer.locked
                || !self.scene.paper.raster_visible(*n, self.layers.len())
            {
                return Err("Duplicate, hidden or locked raster target".into());
            }
            crate::keyframes::guard(
                self,
                crate::layer_search::Key::Layer(crate::layer_properties::raster(self, *n)),
            )?;
            changes.sort_by_key(|p| p.offset);
            if changes.windows(2).any(|p| p[0].offset == p[1].offset)
                || changes
                    .iter()
                    .any(|p| layer.read_at(p.offset) != Some(p.before))
            {
                return Err("Stale or duplicate pixel delta".into());
            }
            if crate::clipping::alpha_locked(self, *n) {
                for p in changes.iter_mut() {
                    p.after = crate::clipping::preserve(p.before, p.after);
                }
            }
            changes.retain(|p| p.before != p.after);
        }
        layers.retain(|(_, p)| !p.is_empty());
        let scene = after
            .filter(|s| *s != self.scene)
            .map(|s| (Box::new(self.scene.clone()), Box::new(s)));
        if layers.is_empty() && scene.is_none() {
            return Ok(false);
        }
        let kind = EditKind::Multi { layers, scene };
        let bytes = Edit {
            cached: None,
            label: None,
            group: None,
            kind,
            from: 0,
            to: 0,
        };
        self.admit_undo_cost(bytes.bytes(), self.allocation_bytes())?;
        let mut tiled_candidates = std::collections::BTreeMap::new();
        if let EditKind::Multi { layers, .. } = &bytes.kind {
            let mut budget = self
                .working_budget
                .saturating_sub(self.resident_bytes())
                .saturating_sub(bytes.bytes());
            for (n, changes) in layers {
                if let Some(image) = self.layers[*n].prepare_pixels(
                    changes.iter().map(|p| (p.offset, p.after)),
                    budget
                        .saturating_add(self.layers[*n].allocation_bytes())
                        .min(self.tile_budget(*n)),
                )? {
                    budget = budget.saturating_sub(
                        image
                            .bytes()
                            .saturating_sub(self.layers[*n].allocation_bytes()),
                    );
                    tiled_candidates.insert(*n, image);
                }
            }
        }
        if let EditKind::Multi { layers, scene } = &bytes.kind {
            for (n, changes) in layers {
                if let Some(image) = tiled_candidates.remove(n) {
                    self.layers[*n].tiled = Some(image);
                } else {
                    for p in changes {
                        self.layers[*n].pixels[p.offset] = p.after;
                    }
                }
            }
            if let Some((_, after)) = scene {
                self.scene = after.as_ref().clone();
            }
        }
        self.history.push(bytes.kind);
        self.history_changed();
        Ok(true)
    }
    pub(crate) fn commit_pixel_changes(
        &mut self,
        layer: usize,
        changes: Vec<PixelChange>,
    ) -> Result<bool, String> {
        self.commit_pixel_scene(layer, changes, None)
    }
    pub(crate) fn commit_mouse_changes(
        &mut self,
        layer: usize,
        changes: Vec<PixelChange>,
        metadata: crate::mouse_pressure::Metadata,
    ) -> Result<bool, String> {
        if changes.iter().all(|p| p.before == p.after) {
            return Ok(false);
        }
        if metadata.layer != layer {
            return Err("Frozen stroke layer mismatch".into());
        }
        let mut scene = self.scene.clone();
        scene.append_mouse(metadata)?;
        self.commit_pixel_scene(layer, changes, Some(scene))
    }
    fn commit_pixel_scene(
        &mut self,
        layer: usize,
        mut changes: Vec<PixelChange>,
        after_scene: Option<crate::scene::Scene>,
    ) -> Result<bool, String> {
        if self.color != crate::color_storage::Config::legacy() {
            return Err("This pixel operation requires its high precision edit path".into());
        }
        if after_scene
            .as_ref()
            .is_some_and(|scene| !scene.valid() || !scene.paper.valid_for(self.layers.len()))
        {
            return Err("Invalid stroke scene; document and history unchanged".into());
        }
        changes.sort_unstable_by_key(|change| change.offset);
        if changes
            .windows(2)
            .any(|pair| pair[0].offset == pair[1].offset)
        {
            return Err("Duplicate stroke pixel offset; document and history unchanged".into());
        }
        crate::layer_properties::paintable(self, layer)?;
        if crate::clipping::alpha_locked(self, layer) {
            for p in &mut changes {
                p.after = crate::clipping::preserve(p.before, p.after);
            }
        }
        changes.retain(|p| p.before != p.after);
        if changes.is_empty() {
            return Ok(false);
        }
        changes.shrink_to_fit();
        let scene = after_scene.map(|after| (Box::new(self.scene.clone()), Box::new(after)));
        let bytes = changes.len() * std::mem::size_of::<PixelChange>()
            + std::mem::size_of::<Edit>()
            + scene
                .as_ref()
                .map_or(0, |(a, b)| a.size_bytes() + b.size_bytes());
        self.admit_undo_cost(bytes, self.allocation_bytes())?;
        let tile_budget = self.tile_budget(layer).min(
            self.working_budget
                .saturating_sub(self.resident_bytes())
                .saturating_add(self.layers[layer].allocation_bytes())
                .saturating_sub(bytes),
        );
        let raster = self
            .layers
            .get_mut(layer)
            .ok_or("Raster layer no longer exists")?;
        if raster.locked {
            return Err("The raster layer is locked".into());
        }
        if changes
            .iter()
            .any(|p| raster.read_at(p.offset) != Some(p.before))
        {
            return Err("The layer changed during the stroke".into());
        }
        if let Some(image) =
            raster.prepare_pixels(changes.iter().map(|p| (p.offset, p.after)), tile_budget)?
        {
            raster.tiled = Some(image);
        } else {
            for p in &changes {
                raster.pixels[p.offset] = p.after;
            }
        }
        if let Some((_, after)) = &scene {
            self.scene = after.as_ref().clone();
        }
        self.history.push(EditKind::Pixels {
            layer,
            changes,
            scene,
        });
        self.history_changed();
        Ok(true)
    }
    pub fn edit_scene(&mut self, mut after: crate::scene::Scene) -> Result<bool, String> {
        crate::layer_catalog::reconcile_scene(&self.scene, &mut after)?;
        let mut source_check = self.snapshot();
        source_check.scene = after.clone();
        crate::clip_edit::reconcile(self, &mut source_check)?;
        crate::timelines::reconcile(self, &mut source_check)?;
        crate::layer_comps::reconcile(self, &mut source_check)?;
        after = source_check.scene.clone();
        crate::keyframes::validate_edit(self, &source_check)?;
        crate::layer_properties::validate_scene(self, &after)?;
        crate::appearance::limits(self.width, self.height, &self.layers, &after)?;
        if !after.valid()
            || !after.layer_ids.matches(self.layers.len())
            || !after.paper.valid_for(self.layers.len())
            || self.scene.size_bytes().saturating_add(after.size_bytes())
                > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
        {
            return Err("Invalid or oversized page geometry.".into());
        }
        let cost = self
            .scene
            .size_bytes()
            .saturating_add(after.size_bytes())
            .saturating_add(std::mem::size_of::<Edit>());
        let image = self
            .allocation_bytes()
            .saturating_sub(self.scene.size_bytes())
            .saturating_add(after.size_bytes());
        self.admit_undo_cost(cost, image)?;
        if self.scene == after {
            return Ok(false);
        }
        let before = std::mem::replace(&mut self.scene, after.clone());
        self.history.push(EditKind::Scene {
            before: Box::new(before),
            after: Box::new(after),
        });
        self.history_changed();
        Ok(true)
    }
    pub(crate) fn refresh_dirty(&mut self) {
        self.dirty = self.history.saved != Some(self.history.current)
            || self.history.meta != self.history.saved_meta
            || self.light_table.dirty();
    }
    pub(crate) fn trim_history_memory(&mut self) {
        let budget = self
            .working_budget
            .saturating_sub(self.allocation_bytes())
            .min(self.history.limits.bytes());
        while self.history.undo.len() + self.history.redo.len() > self.history.limits.steps as usize
            || self
                .history
                .undo
                .iter()
                .chain(&self.history.redo)
                .map(Edit::bytes)
                .sum::<usize>()
                > budget
        {
            if self.history.undo.is_empty() {
                self.resource_error=Some("Retained Redo exceeds the available history memory; resolve or explicitly clear it before further work".into());
                break;
            }
            self.history.undo.remove(0);
            self.history.pruned = self.history.pruned.saturating_add(1);
        }
    }
    fn history_changed(&mut self) {
        self.trim_history_memory();
        self.refresh_dirty();
        self.revision += 1;
    }
    pub fn edit_pixel(&mut self, layer: usize, x: u32, y: u32, after: [u8; 4]) -> bool {
        if self.color != crate::color_storage::Config::legacy() {
            return match crate::color_storage::edit_pixel8(self, layer, x, y, after) {
                Ok(v) => v,
                Err(e) => {
                    self.resource_error = Some(e);
                    false
                }
            };
        }
        if crate::layer_properties::paintable(self, layer).is_err() {
            return false;
        }
        if x >= self.width || y >= self.height {
            return false;
        }
        let record = std::mem::size_of::<Edit>();
        if let Err(e) = self.admit_undo_cost(record, self.allocation_bytes()) {
            self.resource_error = Some(e);
            return false;
        }
        let tile_budget = self.tile_budget(layer).saturating_sub(record);
        let protected = crate::clipping::alpha_locked(self, layer);
        let Some(layer_data) = self.layers.get_mut(layer) else {
            return false;
        };
        if layer_data.locked {
            return false;
        }
        let Some(offset) = layer_data.pixel_index(self.width, self.height, x as i64, y as i64)
        else {
            return false;
        };
        let before = layer_data.read_at(offset).unwrap_or([0; 4]);
        let after = if protected {
            crate::clipping::preserve(before, after)
        } else {
            after
        };
        if before == after {
            return false;
        }
        match layer_data.prepare_pixels(std::iter::once((offset, after)), tile_budget) {
            Ok(Some(image)) => layer_data.tiled = Some(image),
            Ok(None) => layer_data.pixels[offset] = after,
            Err(e) => {
                self.resource_error = Some(e);
                return false;
            }
        }
        self.history.push(EditKind::Pixel {
            layer,
            offset,
            before,
            after,
        });
        self.history_changed();
        true
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo.last().and_then(|e| e.label.as_deref())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo.last().and_then(|e| e.label.as_deref())
    }
    pub fn commit_state_named(
        &mut self,
        after: DocumentState,
        label: &str,
    ) -> Result<bool, String> {
        if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
            return Err("Invalid history label".into());
        }
        let changed = self.commit_state(after)?;
        if changed && let Some(edit) = self.history.undo.last_mut() {
            edit.label = Some(label.into());
        }
        Ok(changed)
    }
    pub fn commit_state(&mut self, after: DocumentState) -> Result<bool, String> {
        self.commit_state_group(after, None)
    }
    pub(crate) fn commit_state_group(
        &mut self,
        mut after: DocumentState,
        group: Option<u64>,
    ) -> Result<bool, String> {
        after.scene.layer_ids.append(after.layers.len())?;
        crate::layer_catalog::reconcile(self, &mut after)?;
        crate::text_appearance::refresh(self, &mut after)?;
        crate::cels::reconcile(self, &mut after)?;
        crate::timeline_timing::reconcile(&mut after);
        crate::render_order::reconcile(&mut after);
        crate::file_objects::prune(&mut after);
        crate::layer_groups::prune(&mut after)?;
        crate::paper_background::prune(&mut after);
        crate::clip_edit::reconcile(self, &mut after)?;
        crate::timelines::reconcile(self, &mut after)?;
        crate::layer_comps::reconcile(self, &mut after)?;
        crate::clipping::validate_state(self, &after)?;
        crate::layer_properties::validate_edit(self, &after)?;
        if !after.scene.keyframes.is_default()
            && after.animation.as_ref().is_none_or(|a| {
                after
                    .scene
                    .keyframes
                    .tracks
                    .iter()
                    .flat_map(|t| &t.keys)
                    .any(|k| k.frame >= a.frames)
            })
        {
            return Err("Layer keys require a timeline containing every key".into());
        }
        if !(1..=crate::document::MAX_DIMENSION).contains(&after.width)
            || !(1..=crate::document::MAX_DIMENSION).contains(&after.height)
            || !after.dpi.is_finite()
            || after.dpi <= 0.0
            || !(1..=256).contains(&after.layers.len())
            || !after.color.valid_layers(&after.layers)
            || !after.scene.valid()
            || !after.scene.layer_ids.matches(after.layers.len())
            || !after.scene.paper.valid_for(after.layers.len())
            || after.animation.as_ref().is_some_and(|a| !a.valid())
            || after
                .layers
                .iter()
                .any(|l| !l.valid(after.width, after.height))
        {
            return Err("Invalid image state.".into());
        }
        let dense = self
            .layers
            .iter()
            .chain(&after.layers)
            .filter(|l| l.tiled.is_none())
            .map(crate::document::RasterLayer::allocation_bytes)
            .sum::<usize>();
        if dense > crate::document::DEFAULT_DOCUMENT_BUDGET / 4 {
            return Err("Change exceeds dense Undo memory budget".into());
        }
        let retained = crate::tiled_raster::change_bytes(self, &after);
        self.admit_undo_cost(retained.saturating_add(256), after.allocation_bytes())?;
        if retained > self.working_budget.saturating_sub(after.allocation_bytes()) {
            return Err("Not enough working memory to retain the complete Undo command".into());
        }
        let before = self.snapshot();
        if crate::tiled_raster::history_bytes(&before, &after)
            > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
            || after.allocation_bytes().saturating_add(
                self.resident_bytes()
                    .saturating_sub(self.allocation_bytes()),
            ) > self.working_budget
        {
            return Err("Change exceeds the image/Undo memory budget".into());
        }
        if before == after {
            return Ok(false);
        }
        let mut cost = crate::tiled_raster::history_bytes(&before, &after);
        if group.is_some()
            && let Some(last) = self.history.undo.last()
            && last.group == group
            && let EditKind::State {
                before: original, ..
            } = &last.kind
        {
            cost = cost.max(crate::tiled_raster::history_bytes(original, &after));
        }
        self.admit_undo_cost(cost.saturating_add(256), after.allocation_bytes())?;
        after.restore(self);
        if group.is_some()
            && self.history.limits.group_ms > 0
            && self.history.last_group.is_some_and(|t| {
                t.elapsed().as_millis() <= u128::from(self.history.limits.group_ms)
            })
            && self.history.redo.is_empty()
            && self.history.saved != Some(self.history.current)
            && !self
                .history
                .inflight
                .values()
                .any(|t| *t == self.history.current)
            && let Some(last) = self.history.undo.last_mut()
            && last.group == group
            && last.to == self.history.current
            && let EditKind::State {
                before: original,
                after: previous,
            } = &mut last.kind
        {
            if **original == after {
                let edit = self.history.undo.pop().unwrap();
                self.history.current = edit.from;
            } else {
                **previous = after;
                last.cached = None;
                last.cached = Some(last.kind_bytes());
            }
            self.history.last_group = Some(std::time::Instant::now());
            self.history_changed();
            return Ok(true);
        }
        self.history.push(EditKind::State {
            before: Box::new(before),
            after: Box::new(after),
        });
        if let Some(last) = self.history.undo.last_mut() {
            last.group = group;
            self.history.last_group = group.map(|_| std::time::Instant::now());
        }
        self.history_changed();
        Ok(true)
    }
    pub(crate) fn history_token(&self) -> u64 {
        self.history.current
    }
    pub fn history_group(&self, redo: bool) -> Option<u64> {
        if redo {
            self.history.redo.last()
        } else {
            self.history.undo.last()
        }
        .and_then(|e| e.group)
    }
    pub fn take_resource_error(&mut self) -> Option<String> {
        self.resource_error.take()
    }
    pub fn history_counts(&self) -> (usize, usize) {
        (self.history.undo.len(), self.history.redo.len())
    }
    pub fn resident_bytes(&self) -> usize {
        self.allocation_bytes()
            + self
                .history
                .undo
                .iter()
                .chain(&self.history.redo)
                .map(Edit::bytes)
                .sum::<usize>()
    }
    /// A creation preference is document metadata, independent of drawing Undo.
    pub fn set_layer_comp_policy(
        &mut self,
        policy: crate::layer_comps::Policy,
    ) -> Result<(), String> {
        if policy == crate::layer_comps::Policy::Current
            && !self.scene.layer_comps.comps.is_empty()
            && self.scene.layer_comps.current.is_none()
        {
            return Err("Apply a saved comp before choosing Current-comp-only".into());
        }
        if self.scene.layer_comps.policy != policy {
            self.scene.layer_comps.policy = policy;
            self.history.saved = None;
            self.history.meta = self.history.meta.wrapping_add(1);
            self.revision = self.revision.wrapping_add(1);
            self.refresh_dirty();
        }
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        self.history_barrier();
        let Some(edit) = self.history.undo.pop() else {
            return false;
        };
        let mut tiled = match staged_history_tiles(self, &edit, true) {
            Ok(tiled) => tiled,
            Err(e) => {
                self.history.undo.push(edit);
                self.resource_error = Some(e);
                return false;
            }
        };
        let comp_policy = self.scene.layer_comps.policy;
        match &edit.kind {
            EditKind::Multi { layers, scene } => {
                for (n, changes) in layers {
                    if let Some(image) = tiled.remove(n) {
                        self.layers[*n].tiled = Some(image);
                    } else {
                        for p in changes {
                            self.layers[*n].pixels[p.offset] = p.before;
                        }
                    }
                }
                if let Some((before, _)) = scene {
                    self.scene = before.as_ref().clone();
                }
            }
            EditKind::Pixels {
                layer,
                changes,
                scene,
            } => {
                if let Some(image) = tiled.remove(layer) {
                    self.layers[*layer].tiled = Some(image);
                } else {
                    for p in changes {
                        self.layers[*layer].pixels[p.offset] = p.before;
                    }
                }
                if let Some((before, _)) = scene {
                    self.scene = before.as_ref().clone();
                }
            }
            EditKind::Pixel {
                layer,
                offset,
                before,
                ..
            } => {
                if let Some(image) = tiled.remove(layer) {
                    self.layers[*layer].tiled = Some(image);
                } else {
                    self.layers[*layer].pixels[*offset] = *before;
                }
            }
            EditKind::State { before, .. } => before.restore(self),
            EditKind::Scene { before, .. } => self.scene = before.as_ref().clone(),
        }
        self.scene.layer_comps.policy = comp_policy;
        self.history.current = edit.from;
        self.history.redo.push(edit);
        self.history_changed();
        true
    }
    pub fn redo(&mut self) -> bool {
        self.history_barrier();
        let Some(edit) = self.history.redo.pop() else {
            return false;
        };
        let mut tiled = match staged_history_tiles(self, &edit, false) {
            Ok(tiled) => tiled,
            Err(e) => {
                self.history.redo.push(edit);
                self.resource_error = Some(e);
                return false;
            }
        };
        let comp_policy = self.scene.layer_comps.policy;
        match &edit.kind {
            EditKind::Multi { layers, scene } => {
                for (n, changes) in layers {
                    if let Some(image) = tiled.remove(n) {
                        self.layers[*n].tiled = Some(image);
                    } else {
                        for p in changes {
                            self.layers[*n].pixels[p.offset] = p.after;
                        }
                    }
                }
                if let Some((_, after)) = scene {
                    self.scene = after.as_ref().clone();
                }
            }
            EditKind::Pixels {
                layer,
                changes,
                scene,
            } => {
                if let Some(image) = tiled.remove(layer) {
                    self.layers[*layer].tiled = Some(image);
                } else {
                    for p in changes {
                        self.layers[*layer].pixels[p.offset] = p.after;
                    }
                }
                if let Some((_, after)) = scene {
                    self.scene = after.as_ref().clone();
                }
            }
            EditKind::Pixel {
                layer,
                offset,
                after,
                ..
            } => {
                if let Some(image) = tiled.remove(layer) {
                    self.layers[*layer].tiled = Some(image);
                } else {
                    self.layers[*layer].pixels[*offset] = *after;
                }
            }
            EditKind::State { after, .. } => after.restore(self),
            EditKind::Scene { after, .. } => self.scene = after.as_ref().clone(),
        }
        self.scene.layer_comps.policy = comp_policy;
        self.history.current = edit.to;
        self.history.undo.push(edit);
        self.history_changed();
        true
    }
}

impl Document {
    /// Physical file management is outside drawing Undo. Rebase retained snapshots
    /// too, so a later Undo cannot resurrect an obsolete managed link.
    pub fn remap_managed_files(&mut self, old: &std::path::Path, new: &std::path::Path) -> usize {
        let count = crate::file_browser::remap_scene(&mut self.scene, old, new);
        let mut historical = 0;
        for edit in self.history.undo.iter_mut().chain(&mut self.history.redo) {
            edit.cached = None;
            match &mut edit.kind {
                EditKind::State { before, after } => {
                    historical += crate::file_browser::remap_scene(&mut before.scene, old, new);
                    historical += crate::file_browser::remap_scene(&mut after.scene, old, new);
                }
                EditKind::Scene { before, after } => {
                    historical += crate::file_browser::remap_scene(before, old, new);
                    historical += crate::file_browser::remap_scene(after, old, new);
                }
                EditKind::Multi { scene, .. } | EditKind::Pixels { scene, .. } => {
                    if let Some((before, after)) = scene {
                        historical += crate::file_browser::remap_scene(before, old, new);
                        historical += crate::file_browser::remap_scene(after, old, new);
                    }
                }
                EditKind::Pixel { .. } => {}
            }
        }
        for edit in self.history.undo.iter_mut().chain(&mut self.history.redo) {
            edit.cached = Some(edit.kind_bytes());
        }
        if count + historical > 0 {
            self.history.saved = None;
            self.history.meta = self.history.meta.wrapping_add(1);
            self.history_changed();
        }
        count
    }
}

#[path = "history_audit.rs"]
pub mod audit;
#[path = "history_roundtrip.rs"]
pub mod roundtrip;
#[path = "saved_revision.rs"]
pub mod saved_revision;

#[cfg(test)]
mod tests {
    use crate::document::{Background, CanvasSpec, Document};
    #[test]
    fn document_history_is_independent_and_tracks_saved_revision() {
        let spec = CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        };
        let mut a = Document::create(spec, 4).unwrap();
        let mut b = Document::create(spec, 4).unwrap();
        a.mark_saved("a.drawer".into());
        assert!(a.edit_pixel(0, 0, 0, [10, 20, 30, 255]));
        assert!(a.dirty);
        assert_eq!(b.history_counts(), (0, 0));
        assert_eq!(b.layers[0].pixels[0], [0; 4]);
        a.mark_saved("a.drawer".into());
        assert!(a.undo());
        assert!(a.dirty);
        assert!(a.redo());
        assert!(!a.dirty);
        assert!(a.undo());
        assert!(a.edit_pixel(0, 0, 0, [50, 60, 70, 255]));
        assert!(a.dirty);
        assert!(!a.redo());
        assert!(!b.undo());
        assert_ne!(a.id, b.id);
    }
}

#[path = "history_navigation.rs"]
pub mod navigation;

#[path = "history_limits.rs"]
pub mod limits;

impl Document {
    fn admit_undo_cost(&self, cost: usize, image: usize) -> Result<(), String> {
        let history = self.history.limits.bytes();
        if cost > history
            || image
                .checked_add(cost)
                .is_none_or(|n| n > self.working_budget)
        {
            return Err(format!(
                "New edit stopped: complete Undo needs {cost} bytes, history limit {history}, image {image}, working limit {}. Current picture preserved; free memory or raise limits",
                self.working_budget
            ));
        }
        Ok(())
    }
}
