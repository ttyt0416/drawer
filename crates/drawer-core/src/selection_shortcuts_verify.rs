use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::Affine,
    selection_edit::Combine,
    selection_shortcuts::{self as core, Command, Source},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let make = || {
        Document::create(
            CanvasSpec {
                width: 16,
                height: 16,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())
    };
    let mut d = make()?;
    let layer = d.scene.lines.add_layer("Contour")?;
    let points = vec![[1.5, 0.0], [5.5, 0.0], [5.5, 6.0], [1.5, 6.0]];
    let path = d
        .scene
        .lines
        .add_path(layer, points.clone(), true, 7.0, [40, 50, 60, 128])?;
    d.scene.lines.layers[0].hidden = true;
    d.scene.lines.layers[0].locked = true;
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 2,
            length: 8,
            alpha: 128,
        }],
    });
    for (op, edge, middle) in [
        (Combine::Replace, 128, 255),
        (Combine::Add, 192, 255),
        (Combine::Subtract, 64, 0),
        (Combine::Intersect, 64, 128),
    ] {
        let before = d.snapshot();
        let history = d.history_counts();
        let scene = core::prepare(&d, &Source::Vector(path), op, false)?;
        check(
            scene.selection_coverage([1.5, 2.5]) == edge
                && scene.selection_coverage([3.5, 2.5]) == middle,
            "Contour combine/AA independent values",
        )?;
        check(d.snapshot() == before, "Contour preview mutated page")?;
        d.edit_scene(scene)?;
        check(
            d.layers == before.layers
                && d.scene.lines == before.scene.lines
                && d.history_counts().0 == history.0 + 1
                && d.undo()
                && d.snapshot() == before,
            "Contour source/read-only/one Undo",
        )?;
    }
    d.scene.selection_mask = None;
    d.scene.lines.layers[0].paths[0].closed = false;
    let source = Source::Vector(path);
    let before = d.snapshot();
    let p = core::preview(&d, &source, Combine::Replace)?;
    check(
        p.closure == Some([[1.5, 6.0], [1.5, 0.0]])
            && p.scene.selection_coverage([3.5, 2.5]) == 255,
        "Open path closing preview",
    )?;
    check(
        core::prepare(&d, &source, Combine::Replace, false).is_err() && d.snapshot() == before,
        "Open path silently closed",
    )?;
    let scene = core::prepare(&d, &source, Combine::Replace, true)?;
    d.edit_scene(scene)?;
    check(
        !d.scene.lines.layers[0].paths[0].closed && d.undo() && d.snapshot() == before,
        "Auto-close changed original path",
    )?;
    let mut ids = vec![];
    for (i, (a, b)) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(4)
        .enumerate()
    {
        ids.push(d.scene.rulers.add(
            &format!("Side{i}"),
            crate::rulers::Kind::Straight { a: *b, b: *a },
        )?);
    }
    ids.swap(0, 2);
    let ruler = Source::Rulers(ids.clone());
    let r = core::preview(&d, &ruler, Combine::Replace)?;
    check(
        r.closure.is_none()
            && r.scene.selection_coverage([3.5, 2.5]) == 255
            && r.scene.selection_coverage([1.5, 2.5]) == 128,
        "Unordered/reversed closed ruler contour",
    )?;
    let mut open = ids.clone();
    open.retain(|i| *i != ids[0]);
    let o = Source::Rulers(open);
    check(
        core::preview(&d, &o, Combine::Replace)?.closure.is_some()
            && core::prepare(&d, &o, Combine::Replace, false).is_err(),
        "Open ruler chain confirmation",
    )?;
    let branch = d.scene.rulers.add(
        "Branch",
        crate::rulers::Kind::Straight {
            a: points[0],
            b: [10.0, 10.0],
        },
    )?;
    let mut bad = ids.clone();
    bad.push(branch);
    check(
        core::preview(&d, &Source::Rulers(bad), Combine::Replace).is_err(),
        "Branched ruler accepted",
    )?;
    check(
        core::preview(&d, &Source::Rulers(vec![ids[0], ids[0]]), Combine::Replace).is_err()
            && core::preview(&d, &Source::Vector(u64::MAX), Combine::Replace).is_err(),
        "Duplicate/missing contour accepted",
    )?;
    d.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 15.0, 1.0]);
    let p = core::preview(&d, &source, Combine::Replace)?;
    check(
        p.points[0] == [15.0, 4.0] && p.scene.selection_coverage([10.0, 8.0]) == 255,
        "Contour page transform",
    )?;
    d.edit_scene(p.scene)?;
    let file = root.join("contour-selection.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Contour native reopen",
    )?;
    let mut workspace = crate::workspaces::Snapshot::default();
    workspace.selection_shortcuts.commands =
        vec![Command::Contour, Command::Outline, Command::Undo];
    workspace.selection_shortcuts.visible = false;
    let file = root.join("selection-shortcuts.json");
    workspace.export(&file).map_err(|e| e.to_string())?;
    let read = crate::workspaces::Snapshot::read(&file).map_err(|e| e.to_string())?;
    check(
        read.selection_shortcuts == workspace.selection_shortcuts,
        "Toolbar workspace reopen",
    )?;
    let base = crate::workspaces::Snapshot::default();
    let bars = crate::workspaces::Scope {
        bars: true,
        ..Default::default()
    };
    check(
        base.merge(&read, bars, &[])
            .map_err(|e| e.to_string())?
            .0
            .selection_shortcuts
            == read.selection_shortcuts,
        "Toolbar bars import scope",
    )?;
    let other = crate::workspaces::Scope {
        other: true,
        ..Default::default()
    };
    check(
        base.merge(&read, other, &[])
            .map_err(|e| e.to_string())?
            .0
            .selection_shortcuts
            == base.selection_shortcuts,
        "Toolbar leaked into other import scope",
    )?;
    workspace
        .selection_shortcuts
        .commands
        .push(Command::Contour);
    check(
        !workspace.valid() && workspace.export(&file).is_err(),
        "Duplicate shortcut validation",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn contours_confirmation_ruler_graph_combines_transform_storage_toolbar() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
