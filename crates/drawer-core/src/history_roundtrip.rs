//! Read-only audit of every retained transaction, on detached document/history copies.
use super::*;
use std::collections::BTreeMap;
#[derive(Default, Debug, serde::Serialize)]
pub struct Report {
    pub undo: usize,
    pub redo: usize,
    pub by_kind: BTreeMap<&'static str, usize>,
}
fn kind(edit: &Edit) -> &'static str {
    match edit.kind {
        EditKind::Multi { .. } => "Multi",
        EditKind::Pixels { .. } => "Pixels",
        EditKind::Pixel { .. } => "Pixel",
        EditKind::Scene { .. } => "Scene",
        EditKind::State { .. } => "State",
    }
}
fn set(
    state: &mut DocumentState,
    layer: usize,
    changes: &[PixelChange],
    back: bool,
) -> Result<(), String> {
    let layer = state
        .layers
        .get_mut(layer)
        .ok_or("History audit missing layer")?;
    if let Some(tiles) = &mut layer.tiled {
        tiles.prepare(
            changes.iter().map(|c| c.offset),
            crate::document::DEFAULT_DOCUMENT_BUDGET,
        )?;
        for c in changes {
            tiles.set_prepared(c.offset, if back { c.before } else { c.after });
        }
    } else {
        for c in changes {
            *layer
                .pixels
                .get_mut(c.offset)
                .ok_or("History audit missing original pixel")? =
                if back { c.before } else { c.after };
        }
    }
    Ok(())
}
fn expected(d: &Document, edit: &Edit, back: bool) -> Result<DocumentState, String> {
    let mut state = d.snapshot();
    match &edit.kind {
        EditKind::State { before, after } => {
            state = if back {
                before.as_ref().clone()
            } else {
                after.as_ref().clone()
            }
        }
        EditKind::Scene { before, after } => {
            state.scene = if back {
                before.as_ref().clone()
            } else {
                after.as_ref().clone()
            }
        }
        EditKind::Pixel {
            layer,
            offset,
            before,
            after,
        } => set(
            &mut state,
            *layer,
            &[PixelChange {
                offset: *offset,
                before: *before,
                after: *after,
            }],
            back,
        )?,
        EditKind::Pixels {
            layer,
            changes,
            scene,
        } => {
            set(&mut state, *layer, changes, back)?;
            if let Some((before, after)) = scene {
                state.scene = if back {
                    before.as_ref().clone()
                } else {
                    after.as_ref().clone()
                };
            }
        }
        EditKind::Multi { layers, scene } => {
            for (layer, changes) in layers {
                set(&mut state, *layer, changes, back)?;
            }
            if let Some((before, after)) = scene {
                state.scene = if back {
                    before.as_ref().clone()
                } else {
                    after.as_ref().clone()
                };
            }
        }
    }
    // Layer-comp recall policy is a local preference, deliberately outside drawing Undo.
    state.scene.layer_comps.policy = d.scene.layer_comps.policy;
    Ok(state)
}
impl Document {
    pub fn audit_history_roundtrip(&self) -> Result<Report, String> {
        if self.resident_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 4 {
            return Err("History audit copy memory limit".into());
        }
        let mut report = Report::default();
        for back in [true, false] {
            let mut probe = self.current_copy();
            probe.history = self.history.clone();
            loop {
                let edit = if back {
                    probe.history.undo.last()
                } else {
                    probe.history.redo.last()
                };
                let Some(edit) = edit.cloned() else { break };
                let original = probe.snapshot();
                let wanted = expected(&probe, &edit, back)?;
                let moved = if back { probe.undo() } else { probe.redo() };
                if !moved || probe.snapshot() != wanted {
                    return Err(format!(
                        "{} transaction failed its {} payload comparison",
                        kind(&edit),
                        if back { "Undo" } else { "Redo" }
                    ));
                }
                let restored = if back { probe.redo() } else { probe.undo() };
                if !restored || probe.snapshot() != original {
                    return Err(format!(
                        "{} transaction failed exact return to the original state",
                        kind(&edit)
                    ));
                }
                let continued = if back { probe.undo() } else { probe.redo() };
                if !continued || probe.snapshot() != wanted {
                    return Err("History audit second traversal differs".into());
                }
                if back {
                    report.undo += 1;
                } else {
                    report.redo += 1;
                }
                *report.by_kind.entry(kind(&edit)).or_default() += 1;
            }
        }
        Ok(report)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_all_transaction_formats_redo_branch_corruption_and_source_invariance() {
        crate::history::audit::run().unwrap();
        let mut d = crate::text_layout_verify::fixture().unwrap();
        d.edit_pixel(0, 0, 0, [10, 20, 30, 128]);
        let mut s = d.snapshot();
        s.layers[0].name = "일련 상태".into();
        d.commit_state(s).unwrap();
        let original = (
            d.snapshot(),
            d.revision,
            d.history_counts(),
            d.dirty,
            d.path.clone(),
        );
        let r = d.audit_history_roundtrip().unwrap();
        assert_eq!(r.undo, 2);
        assert_eq!(
            (
                d.snapshot(),
                d.revision,
                d.history_counts(),
                d.dirty,
                d.path.clone()
            ),
            original
        );
        assert!(d.undo());
        let r = d.audit_history_roundtrip().unwrap();
        assert_eq!((r.undo, r.redo), (1, 1));
        assert!(d.redo());
        if let EditKind::State { after, .. } = &mut d.history.undo.last_mut().unwrap().kind {
            after.layers[0].name = "corrupted retained after".into();
        } else {
            panic!("Expected state transaction");
        }
        assert!(d.audit_history_roundtrip().is_err());
        assert_eq!(d.snapshot(), original.0);
    }
}
