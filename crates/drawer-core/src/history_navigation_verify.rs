use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn run() -> Result<(), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let mut other = d.current_copy();
    d.mark_saved("history-navigation.drawer".into());
    let start = d.snapshot();
    let base = d.history_entries()[0].token;
    for n in 1..=4 {
        d.edit_pixel(0, 0, 0, [n, 0, 0, 255]);
    }
    let end = d.snapshot();
    let tail = d.history_entries().last().unwrap().token;
    check(
        d.jump_history(base)? && d.snapshot() == start && !d.dirty,
        "Multi-step backward/saved state",
    )?;
    check(
        d.history_entries().iter().filter(|e| e.redo).count() == 4,
        "Redo branch labels",
    )?;
    check(
        d.jump_history(tail)? && d.snapshot() == end && d.dirty,
        "Multi-step forward state",
    )?;
    d.undo();
    let removed = tail;
    d.edit_pixel(0, 0, 0, [90, 0, 0, 255]);
    let state = d.snapshot();
    check(
        d.jump_history(removed).is_err() && d.snapshot() == state,
        "Removed branch accepted",
    )?;
    let checkpoint = d.begin_save_checkpoint()?;
    let frozen = d.current_copy();
    d.undo();
    let before = (d.snapshot(), d.dirty, d.path.clone(), d.revision);
    d.clear_session_history();
    check(
        d.snapshot() == before.0
            && d.dirty == before.1
            && d.path == before.2
            && d.revision == before.3
            && !d.undo()
            && !d.redo(),
        "Clear modified drawing or retained records",
    )?;
    // A save of the cleared-away state still marks only that captured identity.
    d.complete_save_checkpoint(
        checkpoint,
        std::path::Path::new("history-navigation.drawer"),
    )?;
    check(
        d.dirty && frozen.snapshot() != d.snapshot(),
        "Clear lost in-flight save identity",
    )?;
    check(d.jump_history(base).is_err(), "Pruned state accepted")?;
    let state = d.snapshot();
    d.edit_pixel(0, 1, 1, [3, 4, 5, 255]);
    check(d.undo() && d.snapshot() == state, "New Undo after clear")?;
    d.mark_saved("history-navigation.drawer".into());
    d.clear_session_history();
    check(!d.dirty, "Clear made saved state dirty")?;
    other.edit_pixel(0, 0, 0, [8, 8, 8, 255]);
    d.clear_session_history();
    check(
        other.history_counts() == (1, 0),
        "Other document history changed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn jumps_branches_clear_and_pending_save() {
        super::run().unwrap();
    }
}
