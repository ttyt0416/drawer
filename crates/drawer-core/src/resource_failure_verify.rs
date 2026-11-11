use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document};
pub fn run() -> Result<(), String> {
    for modern in [false, true] {
        let spec = CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        };
        let mut d = if modern {
            Document::create_with_color(spec, B, Default::default())
        } else {
            Document::create(spec, B)
        }
        .map_err(|e| e.to_string())?;
        d.mark_saved("resource-failure.drawer".into());
        d.set_working_budget(d.allocation_bytes());
        let before = (d.snapshot(), d.revision, d.dirty, d.history_counts());
        if d.edit_pixel(0, 0, 0, [5, 6, 7, 255])
            || d.take_resource_error().is_none()
            || d.snapshot() != before.0
            || d.revision != before.1
            || d.dirty != before.2
            || d.history_counts() != before.3
        {
            return Err("Exhausted memory allowed a silent/unrecorded edit".into());
        }
        let mut after = d.snapshot();
        after.dpi = 72.;
        if d.commit_state(after).is_ok() || d.snapshot() != before.0 {
            return Err("Exhausted state edit changed drawing".into());
        }
        d.set_working_budget(B);
        if !d.edit_pixel(0, 0, 0, [5, 6, 7, 255]) || !d.undo() || d.snapshot() != before.0 {
            return Err("Memory recovery did not restore editing/Undo".into());
        }
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 512,
            height: 512,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.configure_history(crate::history_limits::Settings {
        memory_mib: 1,
        ..Default::default()
    })?;
    let before = d.snapshot();
    let mut after = before.clone();
    after.dpi = 72.;
    if d.commit_state(after).is_ok() || d.snapshot() != before || d.history_counts() != (0, 0) {
        return Err("Oversized new Undo was silently dropped after editing".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn exhausted_memory_stops_edits_without_silent_history_loss() {
        super::run().unwrap();
    }
}
