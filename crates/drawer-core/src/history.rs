use crate::{document::Document, sizing::DocumentState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelChange {
    pub offset: usize,
    pub before: [u8; 4],
    pub after: [u8; 4],
}
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
struct Edit {
    group: Option<u64>,
    kind: EditKind,
    from: u64,
    to: u64,
}
impl Edit {
    fn bytes(&self) -> usize {
        match &self.kind {
            EditKind::Multi { layers, scene } => {
                layers
                    .iter()
                    .map(|(_, p)| p.len() * std::mem::size_of::<PixelChange>())
                    .sum::<usize>()
                    + std::mem::size_of::<Self>()
                    + scene
                        .as_ref()
                        .map_or(0, |(a, b)| a.size_bytes() + b.size_bytes())
            }
            EditKind::Pixels { changes, scene, .. } => {
                changes.len() * std::mem::size_of::<PixelChange>()
                    + std::mem::size_of::<Self>()
                    + scene
                        .as_ref()
                        .map_or(0, |(a, b)| a.size_bytes() + b.size_bytes())
            }
            EditKind::Pixel { .. } => std::mem::size_of::<Self>(),
            EditKind::State { before, after } => before.memory_bytes() + after.memory_bytes(),
            EditKind::Scene { before, after } => {
                before.size_bytes() + after.size_bytes() + std::mem::size_of::<Self>()
            }
        }
    }
}
#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    current: u64,
    next: u64,
    saved: Option<u64>,
}
impl History {
    pub(crate) fn mark_saved(&mut self) {
        self.saved = Some(self.current);
    }
    fn push(&mut self, kind: EditKind) {
        self.next += 1;
        let edit = Edit {
            group: None,
            kind,
            from: self.current,
            to: self.next,
        };
        self.current = edit.to;
        self.redo.clear();
        self.undo.push(edit);
        while self.undo.len() > 100
            || self.undo.iter().map(Edit::bytes).sum::<usize>()
                > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
        {
            self.undo.remove(0);
        }
    }
}
impl Document {
    pub(crate) fn commit_multi(
        &mut self,
        mut layers: Vec<(usize, Vec<PixelChange>)>,
        after: Option<crate::scene::Scene>,
    ) -> Result<bool, String> {
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
            changes.sort_by_key(|p| p.offset);
            if changes.windows(2).any(|p| p[0].offset == p[1].offset)
                || changes
                    .iter()
                    .any(|p| layer.pixels.get(p.offset) != Some(&p.before))
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
            group: None,
            kind,
            from: 0,
            to: 0,
        };
        if bytes.bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
            || bytes.bytes().saturating_add(self.resident_bytes())
                > crate::document::DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Multi-layer undo memory budget".into());
        }
        if let EditKind::Multi { layers, scene } = &bytes.kind {
            for (n, changes) in layers {
                for p in changes {
                    self.layers[*n].pixels[p.offset] = p.after;
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
        if bytes > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
            || bytes + self.resident_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Stroke exceeds the undo memory budget".into());
        }
        let raster = self
            .layers
            .get_mut(layer)
            .ok_or("Raster layer no longer exists")?;
        if raster.locked {
            return Err("The raster layer is locked".into());
        }
        if changes
            .iter()
            .any(|p| raster.pixels.get(p.offset) != Some(&p.before))
        {
            return Err("The layer changed during the stroke".into());
        }
        for p in &changes {
            raster.pixels[p.offset] = p.after;
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
    fn history_changed(&mut self) {
        self.dirty = self.history.saved != Some(self.history.current);
        self.revision += 1;
    }
    pub fn edit_pixel(&mut self, layer: usize, x: u32, y: u32, after: [u8; 4]) -> bool {
        if crate::layer_properties::paintable(self, layer).is_err() {
            return false;
        }
        if x >= self.width || y >= self.height {
            return false;
        }
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
        let before = layer_data.pixels[offset];
        let after = if protected {
            crate::clipping::preserve(before, after)
        } else {
            after
        };
        if before == after {
            return false;
        }
        layer_data.pixels[offset] = after;
        self.history.push(EditKind::Pixel {
            layer,
            offset,
            before,
            after,
        });
        self.history_changed();
        true
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
        crate::render_order::reconcile(&mut after);
        crate::file_objects::prune(&mut after);
        crate::layer_groups::prune(&mut after)?;
        crate::paper_background::prune(&mut after);
        crate::clipping::validate_state(self, &after)?;
        crate::layer_properties::validate_edit(self, &after)?;
        if !(1..=crate::document::MAX_DIMENSION).contains(&after.width)
            || !(1..=crate::document::MAX_DIMENSION).contains(&after.height)
            || !after.dpi.is_finite()
            || after.dpi <= 0.0
            || !(1..=256).contains(&after.layers.len())
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
        if self.memory_bytes().saturating_add(after.memory_bytes())
            > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
        {
            return Err("Change exceeds the undo memory budget.".into());
        }
        let before = self.snapshot();
        if before == after {
            return Ok(false);
        }
        after.restore(self);
        if group.is_some()
            && self.history.redo.is_empty()
            && self.history.saved != Some(self.history.current)
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
            }
            self.history_changed();
            return Ok(true);
        }
        self.history.push(EditKind::State {
            before: Box::new(before),
            after: Box::new(after),
        });
        if let Some(last) = self.history.undo.last_mut() {
            last.group = group;
        }
        self.history_changed();
        Ok(true)
    }
    pub fn history_counts(&self) -> (usize, usize) {
        (self.history.undo.len(), self.history.redo.len())
    }
    pub fn resident_bytes(&self) -> usize {
        self.memory_bytes()
            + self
                .history
                .undo
                .iter()
                .chain(&self.history.redo)
                .map(Edit::bytes)
                .sum::<usize>()
    }
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.history.undo.pop() else {
            return false;
        };
        match &edit.kind {
            EditKind::Multi { layers, scene } => {
                for (n, changes) in layers {
                    for p in changes {
                        self.layers[*n].pixels[p.offset] = p.before;
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
                for p in changes {
                    self.layers[*layer].pixels[p.offset] = p.before;
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
            } => self.layers[*layer].pixels[*offset] = *before,
            EditKind::State { before, .. } => before.restore(self),
            EditKind::Scene { before, .. } => self.scene = before.as_ref().clone(),
        }
        self.history.current = edit.from;
        self.history.redo.push(edit);
        self.history_changed();
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.history.redo.pop() else {
            return false;
        };
        match &edit.kind {
            EditKind::Multi { layers, scene } => {
                for (n, changes) in layers {
                    for p in changes {
                        self.layers[*n].pixels[p.offset] = p.after;
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
                for p in changes {
                    self.layers[*layer].pixels[p.offset] = p.after;
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
            } => self.layers[*layer].pixels[*offset] = *after,
            EditKind::State { after, .. } => after.restore(self),
            EditKind::Scene { after, .. } => self.scene = after.as_ref().clone(),
        }
        self.history.current = edit.to;
        self.history.undo.push(edit);
        self.history_changed();
        true
    }
}
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
