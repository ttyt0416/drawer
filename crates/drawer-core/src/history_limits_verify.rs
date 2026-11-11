use super::*;
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn run() -> Result<(), String> {
    let mut d = Document::create(
        crate::document::CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: crate::document::Background::Transparent,
        },
        crate::document::DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.configure_history(Settings {
        steps: 3,
        memory_mib: 1,
        group_ms: 1000,
    })?;
    for n in 1..=5 {
        d.edit_pixel(0, 0, 0, [n, 0, 0, 255]);
    }
    check(
        d.history_counts() == (3, 0) && d.history_pruned() == 2,
        "Step limit did not remove oldest records",
    )?;
    for n in (2..=4).rev() {
        check(
            d.undo() && d.layers[0].pixels[0][0] == n,
            "Retained history chain broken",
        )?;
    }
    let before = d.snapshot();
    check(
        !d.undo()
            && d.can_configure_history(Settings {
                steps: 1,
                ..Default::default()
            })
            .is_err()
            && d.snapshot() == before
            && d.history_counts() == (0, 3),
        "Limit destroyed Redo or changed picture",
    )?;
    while d.redo() {}
    d.clear_session_history();
    let mut state = d.snapshot();
    state.dpi = 200.;
    d.commit_state_group(state, Some(77))?;
    let mut state = d.snapshot();
    state.dpi = 201.;
    d.commit_state_group(state, Some(77))?;
    check(
        d.history_counts() == (1, 0),
        "Explicit continuous group did not merge",
    )?;
    d.history.last_group = Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
    let mut state = d.snapshot();
    state.dpi = 202.;
    d.commit_state_group(state, Some(77))?;
    check(d.history_counts() == (2, 0), "Expired group merged")?;
    d.history_barrier();
    let mut state = d.snapshot();
    state.dpi = 203.;
    d.commit_state_group(state, Some(77))?;
    check(
        d.history_counts() == (3, 0),
        "Document-switch barrier ignored",
    )?;
    d.clear_session_history();
    d.configure_history(Settings {
        steps: 100,
        group_ms: 0,
        ..Default::default()
    })?;
    for n in 1..=2 {
        let mut state = d.snapshot();
        state.dpi = 300. + f64::from(n);
        d.commit_state_group(state, Some(77))?;
    }
    d.edit_pixel(0, 0, 1, [1, 1, 1, 255]);
    d.edit_pixel(0, 0, 2, [2, 2, 2, 255]);
    check(
        d.history_counts() == (4, 0),
        "Separate strokes/disabled grouping merged",
    )?;
    // Full-state edits exceed1MiB in aggregate; only the oldest records may disappear.
    d.clear_session_history();
    d.configure_history(Settings {
        steps: 1000,
        memory_mib: 1,
        group_ms: 1000,
    })?;
    let mut expected = vec![];
    for n in 0..100 {
        let mut state = d.snapshot();
        state.dpi = 400. + f64::from(n);
        d.commit_state_named(state, "Layer-like independent state")?;
        expected.push(d.snapshot());
    }
    let current = d.snapshot();
    check(
        d.history_bytes() <= 1024 * 1024 && d.history_counts().0 < 100 && d.snapshot() == current,
        "History memory ceiling",
    )?;
    for state in expected.iter().rev().skip(1) {
        if !d.undo() {
            break;
        }
        check(
            d.snapshot() == *state,
            "Memory pruning broke remaining chain",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn step_memory_grouping_boundaries_and_redo_preservation() {
        super::run().unwrap();
    }
}
