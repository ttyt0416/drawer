use crate::document::Document;

#[derive(Clone)]
struct PixelEdit {
    layer: usize,
    offset: usize,
    before: [u8; 4],
    after: [u8; 4],
    from: u64,
    to: u64,
}
#[derive(Default)]
pub(crate) struct History {
    undo: Vec<PixelEdit>,
    redo: Vec<PixelEdit>,
    current: u64,
    next: u64,
    saved: Option<u64>,
}
impl History {
    pub(crate) fn mark_saved(&mut self) {
        self.saved = Some(self.current);
    }
}
impl Document {
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
        self.history.next += 1;
        let edit = PixelEdit {
            layer,
            offset,
            before,
            after,
            from: self.history.current,
            to: self.history.next,
        };
        layer_data.pixels[offset] = after;
        self.history.current = edit.to;
        self.history.undo.push(edit);
        if self.history.undo.len() > 100 {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        self.dirty = self.history.saved != Some(self.history.current);
        self.revision += 1;
        true
    }
    pub fn history_counts(&self) -> (usize, usize) {
        (self.history.undo.len(), self.history.redo.len())
    }
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.history.undo.pop() else {
            return false;
        };
        self.layers[edit.layer].pixels[edit.offset] = edit.before;
        self.history.current = edit.from;
        self.history.redo.push(edit);
        self.dirty = self.history.saved != Some(self.history.current);
        self.revision += 1;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.history.redo.pop() else {
            return false;
        };
        self.layers[edit.layer].pixels[edit.offset] = edit.after;
        self.history.current = edit.to;
        self.history.undo.push(edit);
        self.dirty = self.history.saved != Some(self.history.current);
        self.revision += 1;
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
