use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_masks::{self as core, Command, Policy},
    layer_properties::{self as props, Target},
    layer_search::Key,
    selection::{Mask, Run},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].placement = None;
    d.layers[0].pixels = vec![[100, 150, 200, 128]; 3];
    d.scene.selection_mask = Some(Mask {
        runs: vec![
            Run {
                x: 1,
                y: 0,
                length: 1,
                alpha: 128,
            },
            Run {
                x: 2,
                y: 0,
                length: 1,
                alpha: 255,
            },
        ],
    });
    Ok(d)
}
fn apply(d: &mut Document, t: Target, c: Command) -> Result<Target, String> {
    let p = core::prepare(d, t, c)?;
    d.commit_state(p.state)?;
    Ok(p.selected)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let r = Target::Raster(1);
    let mut d = fixture()?;
    let before = d.snapshot();
    let counts = d.history_counts();
    let preview = core::prepare(
        &d,
        r,
        Command::Selection {
            hide_inside: false,
            policy: Policy::Create,
        },
    )?;
    check(d.snapshot() == before, "Mask creation preview changed page")?;
    d.commit_state(preview.state)?;
    for (x, a) in [0, 64, 128].into_iter().enumerate() {
        check(
            d.composite_pixel(x as u32, 0)[3] == a,
            "Selection 0/128/255 mask strength",
        )?;
    }
    check(
        d.layers == before.layers && d.history_counts().0 == counts.0 + 1,
        "Mask changed source pixels or Undo count",
    )?;
    let with_mask = d.snapshot();
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == with_mask,
        "Mask create Undo/Redo",
    )?;
    let p = core::prepare(&d, r, Command::Apply)?;
    check(
        !p.notes.is_empty() && d.snapshot() == with_mask,
        "Mask application preflight",
    )?;
    d.commit_state(p.state)?;
    check(
        core::get(&d, r).is_none()
            && d.layers[0].pixels.iter().map(|p| p[3]).collect::<Vec<_>>() == [0, 64, 128],
        "Applied alpha mask or mask removal",
    )?;
    for (x, a) in [0, 64, 128].into_iter().enumerate() {
        check(
            d.composite_pixel(x as u32, 0)[3] == a,
            "Applied mask changed appearance",
        )?;
    }
    check(
        d.undo() && d.snapshot() == with_mask,
        "Apply mask Undo did not restore editable mask",
    )?;
    apply(&mut d, r, Command::Invert)?;
    for (x, a) in [128, 64, 0].into_iter().enumerate() {
        check(
            d.composite_pixel(x as u32, 0)[3] == a,
            "Mask inversion strengths",
        )?;
    }
    check(
        core::get(&d, r).unwrap().factor([999., 999.]) == 1.,
        "Inverted mask outside sparse bounds",
    )?;
    apply(&mut d, r, Command::Enabled(false))?;
    check(
        (0..3).all(|x| d.composite_pixel(x, 0)[3] == 128)
            && core::prepare(&d, r, Command::Apply).is_err(),
        "Disabled mask altered appearance or applied silently",
    )?;
    apply(&mut d, r, Command::Enabled(true))?;
    apply(&mut d, r, Command::Linked(false))?;
    let m = core::get(&d, r).unwrap().clone();
    let state = crate::search_move::prepare(&d, Key::Layer(r), [2, 0])?;
    d.commit_state(state)?;
    check(
        core::get(&d, r).unwrap().offset == m.offset,
        "Unlinked mask followed layer",
    )?;
    let placement = d.layers[0].placement;
    apply(&mut d, r, Command::Move([1., 0.]))?;
    check(
        core::get(&d, r).unwrap().offset == [1., 0.] && d.layers[0].placement == placement,
        "Independent mask movement changed layer position",
    )?;
    apply(&mut d, r, Command::Move([-1., 0.]))?;
    apply(&mut d, r, Command::Linked(true))?;
    let state = crate::search_move::prepare(&d, Key::Layer(r), [2, 0])?;
    d.commit_state(state)?;
    check(
        core::get(&d, r).unwrap().offset == [2., 0.],
        "Linked raster mask did not follow layer",
    )?;
    let state = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Copy destination".into(),
            parent: None,
        },
    )?
    .state;
    d.commit_state(state)?;
    let dest = props::raster(&d, 1);
    apply(
        &mut d,
        dest,
        Command::Copy {
            source: r,
            replace: false,
        },
    )?;
    check(
        core::get(&d, r) == core::get(&d, dest),
        "Mask copy lost flags or original page position",
    )?;
    check(
        core::prepare(
            &d,
            dest,
            Command::Copy {
                source: r,
                replace: false,
            },
        )
        .is_err(),
        "Mask copy silently replaced existing mask",
    )?;
    apply(&mut d, dest, Command::Invert)?;
    check(
        core::get(&d, r).unwrap().inverted != core::get(&d, dest).unwrap().inverted,
        "Copied masks share mutable state",
    )?;
    apply(
        &mut d,
        dest,
        Command::Copy {
            source: r,
            replace: true,
        },
    )?;
    let path = root.join("layer-mask-controls.drawer");
    let expected = d.snapshot();
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == expected,
        "Mask flags/copy/offset native roundtrip",
    )?;
    let state = props::prepare_patch(
        &d,
        &[dest],
        props::Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    let locked = d.snapshot();
    for c in [
        Command::Invert,
        Command::Enabled(false),
        Command::Linked(false),
        Command::Copy {
            source: r,
            replace: true,
        },
        Command::Remove,
        Command::Selection {
            hide_inside: true,
            policy: Policy::Replace,
        },
        Command::Apply,
    ] {
        check(
            core::prepare(&d, dest, c).is_err() && d.snapshot() == locked,
            "Locked mask edit was not atomic",
        )?;
    }
    let mut d = fixture()?;
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: true,
            policy: Policy::Create,
        },
    )?;
    for (x, a) in [128, 64, 0].into_iter().enumerate() {
        check(
            d.composite_pixel(x as u32, 0)[3] == a,
            "Hide inside selection alpha",
        )?;
    }
    check(
        core::prepare(
            &d,
            r,
            Command::Selection {
                hide_inside: false,
                policy: Policy::Create,
            },
        )
        .is_err(),
        "Existing mask overwrite lacked explicit policy",
    )?;
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: false,
            policy: Policy::Replace,
        },
    )?;
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: false,
            policy: Policy::Intersect,
        },
    )?;
    check(
        (core::get(&d, r).unwrap().factor([1.5, 0.5]) * 255.).round() as u8 == 64,
        "Mask intersection did not multiply partial strengths",
    )?;
    // Two inverted masks keep the unbounded visible exterior.
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: true,
            policy: Policy::Replace,
        },
    )?;
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: true,
            policy: Policy::Intersect,
        },
    )?;
    check(
        core::get(&d, r).unwrap().factor([99., 99.]) == 1.
            && (core::get(&d, r).unwrap().factor([1.5, 0.5]) * 255.).round() as u8 == 63,
        "Inverted intersection exterior/partial coverage",
    )?;
    apply(&mut d, r, Command::Enabled(false))?;
    check(
        core::prepare(
            &d,
            r,
            Command::Selection {
                hide_inside: false,
                policy: Policy::Intersect,
            },
        )
        .is_err(),
        "Disabled intersection ambiguous",
    )?;
    apply(&mut d, r, Command::Remove)?;
    d.scene.selection_mask = None;
    d.scene.selection = None;
    check(
        core::prepare(
            &d,
            r,
            Command::Selection {
                hide_inside: false,
                policy: Policy::Create,
            },
        )
        .is_err(),
        "Missing selection accepted",
    )?;
    d.scene.selection_mask = Some(Mask::default());
    check(
        core::prepare(
            &d,
            r,
            Command::Selection {
                hide_inside: false,
                policy: Policy::Create,
            },
        )
        .is_err(),
        "Explicit empty selection accepted",
    )?;
    let mut d = fixture()?;
    d.scene.transform = crate::scene::Affine([2., 0., 0., 2., -3., 4.]);
    d.scene.selection_translation = [3., -2.];
    apply(
        &mut d,
        r,
        Command::Selection {
            hide_inside: false,
            policy: Policy::Create,
        },
    )?;
    check(
        core::get(&d, r).unwrap().offset == [1.5, -1.]
            && (core::get(&d, r).unwrap().factor([3., -0.5]) * 255.).round() as u8 == 128,
        "Moved selection mask lost inverse transform",
    )?;
    let mut d = crate::line_tools_verify::fixture()?;
    let vector = Target::Vector(d.scene.lines.layers[0].id);
    let mask = crate::layer_folders::rectangle_mask(
        &d,
        crate::scene::Rect {
            min: [0.; 2],
            max: [32., 48.],
        },
        128,
    )?;
    d.commit_state(
        crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Mask {
                target: vector,
                mask: Some(mask),
            },
        )?
        .state,
    )?;
    let state = crate::search_move::prepare(&d, Key::Layer(vector), [2, 1])?;
    d.commit_state(state)?;
    check(
        core::get(&d, vector).unwrap().offset == [2., 1.],
        "Linked vector mask movement",
    )?;
    let original = d.snapshot();
    check(
        core::prepare(&d, vector, Command::Apply).is_err(),
        "Vector alpha apply must offer explicit bake",
    )?;
    let p = core::prepare(&d, vector, Command::Bake)?;
    d.commit_state(p.state)?;
    check(
        d.scene.lines.layers.is_empty() && d.undo() && d.snapshot() == original,
        "Apply vector mask rasterization and Undo",
    )?;
    let mut d = fixture()?;
    d.scene.selection_mask = None;
    let folder = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "Masked folder".into(),
            targets: vec![r],
        },
    )?;
    let f = folder.selected;
    d.commit_state(folder.state)?;
    let mask = crate::layer_folders::rectangle_mask(
        &d,
        crate::scene::Rect {
            min: [0.; 2],
            max: [3., 1.],
        },
        128,
    )?;
    d.commit_state(
        crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Mask {
                target: f,
                mask: Some(mask),
            },
        )?
        .state,
    )?;
    let state = crate::search_move::prepare(&d, Key::Layer(f), [1, 0])?;
    d.commit_state(state)?;
    check(
        core::get(&d, f).unwrap().offset == [1., 0.],
        "Linked folder mask movement",
    )?;
    let original = d.snapshot();
    check(
        core::prepare(&d, f, Command::Apply).is_err(),
        "Folder alpha apply must offer explicit bake",
    )?;
    let p = core::prepare(&d, f, Command::Bake)?;
    d.commit_state(p.state)?;
    check(
        d.scene.layer_effects.get(f).is_none() && d.undo() && d.snapshot() == original,
        "Apply folder mask and Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mask_operations_selection_copy_link_apply_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
