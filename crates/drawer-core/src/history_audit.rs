//! Cross-command transaction and history invariants, without session-history persistence.
use super::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
    layers,
    scene::Rect,
};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())
}
fn stable(d: &Document, before: &(DocumentState, u64, (usize, usize), bool)) -> Result<(), String> {
    check(
        d.snapshot() == before.0
            && d.revision == before.1
            && d.history_counts() == before.2
            && d.dirty == before.3,
        "Failed/no-op command changed pixels, scene, revision, dirty state or redo",
    )
}
pub fn run() -> Result<(), String> {
    let mut d = page()?;
    let other = page()?;
    let other_before = other.snapshot();
    d.mark_saved("history-audit.drawer".into());
    let mut states = vec![d.snapshot()];
    d.commit_pixel_changes(
        0,
        vec![
            PixelChange {
                offset: 0,
                before: [0; 4],
                after: [240, 20, 10, 255],
            },
            PixelChange {
                offset: 1,
                before: [0; 4],
                after: [40, 120, 220, 128],
            },
        ],
    )?;
    states.push(d.snapshot());
    let p = layers::prepare(
        &d,
        layers::Command::Create {
            kind: layers::Kind::Raster,
            name: "Audit layer".into(),
            parent: None,
        },
    )?;
    d.commit_state_named(p.state, "Create layer")?;
    states.push(d.snapshot());
    let mut scene = d.scene.clone();
    scene.selection = Some(Rect {
        min: [0., 0.],
        max: [4., 4.],
    });
    scene.selection_mask = None;
    d.edit_scene(scene)?;
    states.push(d.snapshot());
    let p = crate::affine_transform::prepare(
        &d,
        crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(
            d.scene.layer_ids.id(0),
        )),
        true,
        crate::vector_scope::Transform {
            translation: [2., 1.],
            ..Default::default()
        },
        false,
        crate::interpolation::Filter::Nearest,
    )?;
    d.commit_state_named(p.state, "Transform selection")?;
    states.push(d.snapshot());
    check(
        d.history_counts() == (4, 0),
        "Drawing/layer/selection/transform were not four atomic commands",
    )?;
    for state in states[..4].iter().rev() {
        check(
            d.undo() && d.snapshot() == *state,
            "Cross-command Undo did not restore exact state",
        )?;
    }
    check(
        !d.dirty && !d.undo(),
        "Undo to saved baseline / empty history",
    )?;
    for state in &states[1..] {
        check(
            d.redo() && d.snapshot() == *state,
            "Cross-command Redo did not restore exact state",
        )?;
    }
    check(!d.redo() && d.dirty, "Redo endpoint / dirty status")?;
    d.mark_saved("history-audit.drawer".into());
    check(
        !d.dirty && d.undo() && d.dirty && d.redo() && !d.dirty,
        "Saved revision after Undo/Redo",
    )?;
    check(d.undo() && d.undo(), "Prepare nonempty redo branch")?;
    let checkpoint = (d.snapshot(), d.revision, d.history_counts(), d.dirty);
    let before = d.layers[0].pixels[0];
    check(
        !d.commit_pixel_changes(
            0,
            vec![PixelChange {
                offset: 0,
                before,
                after: before,
            }],
        )?,
        "No-op stroke reported an edit",
    )?;
    check(
        !d.commit_state_named(d.snapshot(), "No operation")?,
        "No-op state reported an edit",
    )?;
    stable(&d, &checkpoint)?;
    let duplicate = vec![
        PixelChange {
            offset: 0,
            before,
            after: [10, 200, 30, 255],
        },
        PixelChange {
            offset: 0,
            before,
            after: [30, 40, 220, 255],
        },
    ];
    check(
        d.commit_pixel_changes(0, duplicate).is_err(),
        "Duplicate pixel deltas entered history",
    )?;
    stable(&d, &checkpoint)?;
    let mut invalid_scene = d.scene.clone();
    invalid_scene.paper.roots = vec![crate::paper::Node::Raster(999)];
    check(
        d.commit_pixel_scene(
            0,
            vec![PixelChange {
                offset: 0,
                before,
                after: [80, 90, 100, 255],
            }],
            Some(invalid_scene),
        )
        .is_err(),
        "Invalid stroke scene partially committed",
    )?;
    stable(&d, &checkpoint)?;
    for change in [
        PixelChange {
            offset: 0,
            before: [1; 4],
            after: [2; 4],
        },
        PixelChange {
            offset: 9999,
            before: [0; 4],
            after: [255; 4],
        },
    ] {
        check(
            d.commit_pixel_changes(0, vec![change]).is_err(),
            "Stale/out-of-range stroke delta committed",
        )?;
        stable(&d, &checkpoint)?;
    }
    let mut invalid = d.snapshot();
    invalid.width = 0;
    check(
        d.commit_state(invalid).is_err(),
        "Invalid whole state committed",
    )?;
    stable(&d, &checkpoint)?;
    check(
        d.commit_multi(
            vec![
                (
                    0,
                    vec![PixelChange {
                        offset: 0,
                        before,
                        after: [50; 4],
                    }],
                ),
                (
                    1,
                    vec![PixelChange {
                        offset: 0,
                        before: [99; 4],
                        after: [20; 4],
                    }],
                ),
            ],
            None,
        )
        .is_err(),
        "Second-layer failure allowed partial first-layer edits",
    )?;
    stable(&d, &checkpoint)?;
    d.layers[0].locked = true;
    let locked = (d.snapshot(), d.revision, d.history_counts(), d.dirty);
    check(
        d.commit_pixel_changes(
            0,
            vec![PixelChange {
                offset: 0,
                before,
                after: [10; 4],
            }],
        )
        .is_err(),
        "Locked layer accepted stroke",
    )?;
    stable(&d, &locked)?;
    d.layers[0].locked = false;
    d.view.zoom = 2.;
    d.view.angle = 45.;
    d.view.pan = [3., 7.];
    stable(&d, &checkpoint)?;
    check(
        d.edit_pixel(0, 2, 2, [20, 180, 60, 255]) && !d.redo() && d.history_counts() == (3, 0),
        "New edit did not discard redo branch",
    )?;
    let baseline = d.snapshot();
    let count = d.history_counts();
    d.commit_multi(
        vec![
            (
                0,
                vec![PixelChange {
                    offset: 0,
                    before: d.layers[0].pixels[0],
                    after: [10, 100, 200, 255],
                }],
            ),
            (
                1,
                vec![PixelChange {
                    offset: 0,
                    before: d.layers[1].pixels[0],
                    after: [200, 80, 20, 128],
                }],
            ),
        ],
        None,
    )?;
    let after = d.snapshot();
    check(
        d.history_counts() == (count.0 + 1, 0)
            && d.undo()
            && d.snapshot() == baseline
            && d.redo()
            && d.snapshot() == after,
        "Multi-layer paint did not remain one transaction",
    )?;
    check(
        other.snapshot() == other_before && other.history_counts() == (0, 0),
        "Document histories were coupled",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn cross_command_undo_redo_saved_branch_failures_duplicates_and_atomic_multilayer() {
        super::run().unwrap();
    }
}
