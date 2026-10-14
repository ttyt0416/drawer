use crate::{document::Document, sizing::DocumentState};

enum EditKind {
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
}
struct Edit {
    kind: EditKind,
    from: u64,
    to: u64,
}
impl Edit {
    fn bytes(&self) -> usize {
        match &self.kind {
            EditKind::Pixel { .. } => std::mem::size_of::<Self>(),
            EditKind::State { before, after } => before.memory_bytes() + after.memory_bytes(),
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
    fn history_changed(&mut self) {
        self.dirty = self.history.saved != Some(self.history.current);
        self.revision += 1;
    }
    pub fn edit_pixel(&mut self, layer: usize, x: u32, y: u32, after: [u8; 4]) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let Some(layer_data) = self.layers.get_mut(layer) else {
            return false;
        };
        let offset = (y * self.width + x) as usize;
        let before = layer_data.pixels[offset];
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
        if !(1..=crate::document::MAX_DIMENSION).contains(&after.width)
            || !(1..=crate::document::MAX_DIMENSION).contains(&after.height)
            || !after.dpi.is_finite()
            || after.dpi <= 0.0
            || !(1..=256).contains(&after.layers.len())
            || !after.scene.valid()
            || after
                .layers
                .iter()
                .any(|l| l.pixels.len() != after.width as usize * after.height as usize)
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
        self.history.push(EditKind::State {
            before: Box::new(before),
            after: Box::new(after),
        });
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
            EditKind::Pixel {
                layer,
                offset,
                before,
                ..
            } => self.layers[*layer].pixels[*offset] = *before,
            EditKind::State { before, .. } => before.restore(self),
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
            EditKind::Pixel {
                layer,
                offset,
                after,
                ..
            } => self.layers[*layer].pixels[*offset] = *after,
            EditKind::State { after, .. } => after.restore(self),
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
