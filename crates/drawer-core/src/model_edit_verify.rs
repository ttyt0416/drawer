use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    human_rig::{self, Kind, Rig},
    model_edit::*,
    model_library::{self as lib, Placement, Shape},
    world3d,
};
use crate::{pose_sequence::Skeleton, world3d::Matrix};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn p(d: &Document, e: Edit) -> Result<Prepared, String> {
    prepare(d, e, B, &AtomicBool::new(false))
}
pub fn fixture(kind: Kind) -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    human_rig::prepare(
        &d,
        human_rig::Edit::Create {
            rig: Rig::new(kind),
            position: [0.; 3],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    lib::prepare(
        &d,
        lib::Edit::Primitive {
            shape: Shape::Box,
            detail: 6,
            placement: Placement {
                position: [0.7, 0.3, 0.],
                fit: Some(0.3),
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let mut s = d.scene.clone();
    s.world3d.lighting = Some(crate::lighting::Settings {
        style: crate::lighting::Style::Unlit,
        ..Default::default()
    });
    d.edit_scene(s)?;
    Ok(d)
}
pub(crate) fn image(d: &Document) -> Vec<[u8; 4]> {
    crate::render::Renderer::new(d)
        .output(d.dpi, B / 8)
        .unwrap()
        .2
}
fn same(a: Matrix, b: Matrix) -> bool {
    a.0.iter()
        .flatten()
        .zip(b.0.iter().flatten())
        .all(|(a, b)| (a - b).abs() < 1e-7)
}
pub fn morphs() -> Result<(), String> {
    let mut d = fixture(Kind::Head)?;
    let start = d.snapshot();
    let base = d.scene.worlds[0].vertices.clone();
    p(
        &d,
        Edit::Expressions {
            rig: 1,
            weights: [1., 1., 1.],
            base: false,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.worlds[0].vertices != base
            && d.scene.world3d.rigs[&1].angles == start.scene.world3d.rigs[&1].angles,
        "Expression morph changed pose / did not deform face",
    )?;
    d.undo();
    check(d.snapshot() == start, "Expression morph not one Undo")?;
    d.redo();
    p(
        &d,
        Edit::Expressions {
            rig: 1,
            weights: [0.; 3],
            base: false,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.worlds[0].vertices == base,
        "Expression zero did not restore original topology/vertices",
    )?;
    for weights in [[-0.1, 0., 0.], [1.1, 0., 0.], [f64::NAN, 0., 0.]] {
        check(
            p(
                &d,
                Edit::Expressions {
                    rig: 1,
                    weights,
                    base: false,
                },
            )
            .is_err(),
            "Invalid expression weight accepted",
        )?;
    }
    let original = d.scene.worlds[1].clone();
    p(
        &d,
        Edit::AddKey {
            model: 2,
            name: "Stretch".into(),
            scale: [1., 2., 1.],
        },
    )?
    .commit(&mut d)?;
    let m = d.scene.world3d.editing.morphs[&2].clone();
    for w in [0., 0.5, 1.] {
        p(
            &d,
            Edit::Weight {
                model: 2,
                key: 0,
                value: w,
            },
        )?
        .commit(&mut d)?;
        let f = world3d::node(&d.scene, 2).unwrap().frame;
        for (i, v) in m.base.iter().enumerate() {
            let expected = f.map([v[0], v[1] * (1. + w), v[2]]);
            check(
                expected
                    .iter()
                    .zip(d.scene.worlds[1].vertices[i])
                    .all(|(a, b)| (a - b).abs() < 1e-7),
                "Shape weight interpolation wrong",
            )?;
        }
    }
    for value in [-0.01, 1.01, f64::NAN] {
        check(
            p(
                &d,
                Edit::Weight {
                    model: 2,
                    key: 0,
                    value,
                },
            )
            .is_err(),
            "Invalid shape weight accepted",
        )?;
    }
    lib::prepare(
        &d,
        lib::Edit::Duplicate {
            ids: vec![2],
            offset: [1., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.morphs.contains_key(&3),
        "Morph duplicate lost original base/keys",
    )?;
    p(
        &d,
        Edit::Weight {
            model: 3,
            key: 0,
            value: 0.,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.morphs[&2].keys[0].weight == 1.,
        "Morph duplicate aliases original",
    )?;
    p(
        &d,
        Edit::Weight {
            model: 3,
            key: 0,
            value: 0.5,
        },
    )?
    .commit(&mut d)?;
    p(
        &d,
        Edit::CaptureKey {
            model: 2,
            source: 3,
            name: "Captured".into(),
        },
    )?
    .commit(&mut d)?;
    let snapshot = d.scene.world3d.editing.morphs[&2].keys[1].delta.clone();
    p(
        &d,
        Edit::Weight {
            model: 3,
            key: 0,
            value: 1.,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.morphs[&2].keys[1].delta == snapshot,
        "Captured shape target changed with its source",
    )?;
    check(
        p(
            &d,
            Edit::CaptureKey {
                model: 2,
                source: 1,
                name: "Wrong".into(),
            },
        )
        .is_err()
            && p(
                &d,
                Edit::AddKey {
                    model: 1,
                    name: "Rig key".into(),
                    scale: [2.; 3],
                },
            )
            .is_err(),
        "Wrong shape topology / unskinned rig shape key accepted",
    )?;
    p(&d, Edit::DeleteKey { model: 2, key: 1 })?.commit(&mut d)?;
    p(&d, Edit::DeleteKey { model: 2, key: 0 })?.commit(&mut d)?;
    check(
        d.scene.worlds[1] == original && !d.scene.world3d.editing.morphs.contains_key(&2),
        "Deleting last key did not restore base mesh",
    )?;
    Ok(())
}
pub fn limits_and_sides() -> Result<(), String> {
    let mut d = fixture(Kind::Body)?;
    let joints = d.scene.world3d.rigs[&1].joints();
    let l = joints.iter().position(|j| j.name == "L/UpperArm").unwrap();
    let r = joints.iter().position(|j| j.name == "R/UpperArm").unwrap();
    p(
        &d,
        Edit::Joint {
            rig: 1,
            joint: l,
            angles: [15., 10., 20.],
            base: false,
        },
    )?
    .commit(&mut d)?;
    let before = d.snapshot();
    p(
        &d,
        Edit::Copy {
            rig: 1,
            side: Side::LeftToRight,
            base: false,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.rigs[&1].angles[r] == [15., -10., -20.]
            && d.scene.world3d.rigs[&1].angles[l] == [15., 10., 20.],
        "Verified side copy changed source / wrong reflection",
    )?;
    d.undo();
    check(d.snapshot() == before, "Side copy not one Undo")?;
    let mut c = controls(&d.scene.world3d.rigs[&1])[l].clone();
    c.min[0] = 10.;
    c.max[0] = 20.;
    c.locked[1] = true;
    c.hold = d.scene.world3d.rigs[&1].angles[l];
    p(
        &d,
        Edit::Control {
            rig: 1,
            joint: l,
            value: c.clone(),
            base: false,
        },
    )?
    .commit(&mut d)?;
    for x in [10., 20.] {
        p(
            &d,
            Edit::Joint {
                rig: 1,
                joint: l,
                angles: [x, 10., 20.],
                base: false,
            },
        )?
        .commit(&mut d)?;
    }
    let frozen = d.snapshot();
    for angles in [[9.9, 10., 20.], [20.1, 10., 20.], [15., 11., 20.]] {
        check(
            p(
                &d,
                Edit::Joint {
                    rig: 1,
                    joint: l,
                    angles,
                    base: false,
                },
            )
            .is_err()
                && d.snapshot() == frozen,
            "Joint min/max/lock bypassed",
        )?;
    }
    let mut direct = d.scene.world3d.rigs[&1].clone();
    direct.angles[l][1] = 11.;
    check(
        human_rig::prepare(
            &d,
            human_rig::Edit::Replace { id: 1, rig: direct },
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Existing rig editor bypassed lock",
    )?;
    let mut bad = c.clone();
    bad.min[0] = 30.;
    bad.max[0] = 20.;
    check(
        p(
            &d,
            Edit::Control {
                rig: 1,
                joint: l,
                value: bad,
                base: false,
            },
        )
        .is_err(),
        "Reversed angle range accepted",
    )?;
    let s = Skeleton::of(&d.scene.world3d.rigs[&1]);
    let map = s
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            b.name.strip_prefix("L/").and_then(|n| {
                s.bones
                    .iter()
                    .position(|b| b.name == format!("R/{n}"))
                    .map(|r| (i, r))
            })
        })
        .collect::<Vec<_>>();
    pairs(&s, &map)?;
    let mut fake = s.clone();
    fake.bones[l].parent = Some(0);
    check(
        pairs(&fake, &map).is_err() && pairs(&s, &map[..map.len() - 1]).is_err(),
        "Names-only incompatible / incomplete mapping accepted",
    )?;
    check(
        copy_side(&Rig::new(Kind::Hand), Side::Swap).is_err(),
        "Missing side map silently accepted",
    )?;
    let mut lock = controls(&d.scene.world3d.rigs[&1])[r].clone();
    lock.locked = [true; 3];
    lock.hold = d.scene.world3d.rigs[&1].angles[r];
    p(
        &d,
        Edit::Control {
            rig: 1,
            joint: r,
            value: lock,
            base: false,
        },
    )?
    .commit(&mut d)?;
    check(
        p(
            &d,
            Edit::Copy {
                rig: 1,
                side: Side::LeftToRight,
                base: false,
            },
        )
        .is_err(),
        "Side copy bypassed destination locks",
    )?;
    Ok(())
}
pub fn hierarchy() -> Result<(), String> {
    let mut d = fixture(Kind::Head)?;
    let original = world3d::node(&d.scene, 2).unwrap().frame;
    let before = d.snapshot();
    p(
        &d,
        Edit::Parent {
            child: 2,
            parent: Some(1),
            joint: Some(1),
            keep: Keep::World,
        },
    )?
    .commit(&mut d)?;
    check(
        same(world3d::node(&d.scene, 2).unwrap().frame, original),
        "World-preserving bone attach moved child",
    )?;
    let linked = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Bone attach not one Undo")?;
    d.redo();
    check(d.snapshot() == linked, "Bone attach Redo differs")?;
    let local = d.scene.world3d.editing.attachments[&2].local;
    p(
        &d,
        Edit::Joint {
            rig: 1,
            joint: 1,
            angles: [0., 0., 30.],
            base: false,
        },
    )?
    .commit(&mut d)?;
    let expected = local
        .then(d.scene.world3d.rigs[&1].frames()[1])
        .then(world3d::node(&d.scene, 1).unwrap().frame);
    check(
        same(world3d::node(&d.scene, 2).unwrap().frame, expected)
            && d.scene.worlds[1].vertices != linked.scene.worlds[1].vertices,
        "Bone child did not follow base joint",
    )?;
    world3d::prepare(
        &d,
        world3d::Edit::Transform {
            ids: vec![2],
            options: Default::default(),
            value: [0.2, 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.attachments[&2].local != local,
        "Direct attached-child movement snapped back / lost offset",
    )?;
    let attached = world3d::node(&d.scene, 2).unwrap().frame;
    let moved = crate::search_move::prepare(&d, crate::layer_search::Key::World(1), [3, 0])?;
    d.commit_state(moved)?;
    check(
        !same(world3d::node(&d.scene, 2).unwrap().frame, attached),
        "Layer movement did not carry bone child",
    )?;
    d.undo();
    let offset = d.scene.world3d.editing.attachments[&2].local;
    let moved = crate::search_move::prepare(&d, crate::layer_search::Key::World(2), [3, 0])?;
    d.commit_state(moved)?;
    check(
        !same(d.scene.world3d.editing.attachments[&2].local, offset),
        "Layer movement of bone child lost local offset",
    )?;
    d.undo();
    let frozen = d.snapshot();
    for edit in [
        Edit::Parent {
            child: 1,
            parent: Some(2),
            joint: None,
            keep: Keep::World,
        },
        Edit::Parent {
            child: 2,
            parent: Some(2),
            joint: None,
            keep: Keep::World,
        },
        Edit::Parent {
            child: 2,
            parent: Some(1),
            joint: Some(99),
            keep: Keep::World,
        },
    ] {
        check(
            p(&d, edit).is_err() && d.snapshot() == frozen,
            "Parent cycle/self/bone mismatch mutated state",
        )?;
    }
    let old_world = world3d::node(&d.scene, 2).unwrap().frame;
    let old_bone =
        d.scene.world3d.rigs[&1].frames()[1].then(world3d::node(&d.scene, 1).unwrap().frame);
    let local = old_world.then(old_bone.inverse());
    p(
        &d,
        Edit::Parent {
            child: 2,
            parent: None,
            joint: None,
            keep: Keep::Local,
        },
    )?
    .commit(&mut d)?;
    check(
        same(world3d::node(&d.scene, 2).unwrap().frame, local),
        "Local-preserving detach used world preservation",
    )?;
    d.undo();
    lib::prepare(
        &d,
        lib::Edit::Duplicate {
            ids: vec![2],
            offset: [1., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        same(
            world3d::node(&d.scene, 3).unwrap().frame,
            world3d::node(&d.scene, 2)
                .unwrap()
                .frame
                .then(Matrix::translation([1., 0., 0.])),
        ),
        "Duplicated bone child lost requested offset",
    )?;
    human_rig::prepare(
        &d,
        human_rig::Edit::Detach { id: 1 },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.attachments.is_empty()
            && world3d::node(&d.scene, 2).unwrap().parent.is_none(),
        "Detached rig retained orphan bone links",
    )?;
    Ok(())
}
pub fn timeline_native(root: &Path) -> Result<(), String> {
    let mut mixed = fixture(Kind::Head)?;
    p(
        &mixed,
        Edit::Expressions {
            rig: 1,
            weights: [0.4, 0.6, 0.2],
            base: false,
        },
    )?
    .commit(&mut mixed)?;
    let mut control = controls(&mixed.scene.world3d.rigs[&1])[1].clone();
    control.locked[0] = true;
    control.hold = mixed.scene.world3d.rigs[&1].angles[1];
    p(
        &mixed,
        Edit::Control {
            rig: 1,
            joint: 1,
            value: control,
            base: false,
        },
    )?
    .commit(&mut mixed)?;
    p(
        &mixed,
        Edit::AddKey {
            model: 2,
            name: "Local shape".into(),
            scale: [1., 1.5, 1.],
        },
    )?
    .commit(&mut mixed)?;
    p(
        &mixed,
        Edit::Weight {
            model: 2,
            key: 0,
            value: 0.5,
        },
    )?
    .commit(&mut mixed)?;
    p(
        &mixed,
        Edit::Parent {
            child: 2,
            parent: Some(1),
            joint: Some(1),
            keep: Keep::World,
        },
    )?
    .commit(&mut mixed)?;
    let snap = mixed.snapshot();
    let file = root.join("mixed.drawer");
    crate::files::save(&mut mixed, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == snap && image(&reopened) == image(&mixed),
        "Native117 weights/locks/expressions/hierarchy differ",
    )?;

    let (mut d, asset) = crate::pose_sequence_verify::fixture()?;
    lib::prepare(
        &d,
        lib::Edit::Primitive {
            shape: Shape::Box,
            detail: 6,
            placement: Placement {
                position: [0.7, 0.3, 0.],
                fit: Some(0.2),
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    crate::pose_sequence::prepare(
        &d,
        crate::pose_sequence::Edit::Bind {
            rig: 1,
            asset,
            start: 2,
            step: 3,
            repeats: 2,
            curve: crate::keyframes::Curve::Fixed,
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    p(
        &d,
        Edit::Parent {
            child: 2,
            parent: Some(1),
            joint: Some(0),
            keep: Keep::World,
        },
    )?
    .commit(&mut d)?;
    let state = d.snapshot();
    let first = image(&d);
    d.current_frame = 5;
    check(
        image(&d) != first && d.snapshot() == state && meshes(&d)?.contains_key(&2),
        "Timeline bone attachment did not animate read-only",
    )?;
    let current = sampled_frames(&d)?[&2];
    p(
        &d,
        Edit::Parent {
            child: 2,
            parent: None,
            joint: None,
            keep: Keep::World,
        },
    )?
    .commit(&mut d)?;
    check(
        same(sampled_frames(&d)?[&2], current),
        "World preservation ignored current animated pose",
    )?;
    d.undo();
    let mut c = controls(&d.scene.world3d.rigs[&1])[0].clone();
    c.locked[2] = true;
    c.hold = d.scene.world3d.rigs[&1].angles[0];
    check(
        p(
            &d,
            Edit::Control {
                rig: 1,
                joint: 0,
                value: c,
                base: true,
            },
        )
        .is_err(),
        "Joint lock silently invalidated stored pose clip",
    )?;
    let native = root.join("model-controls.drawer");
    let frozen = d.snapshot();
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    let mut reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
    reopened.current_frame = 5;
    check(
        reopened.snapshot() == frozen && image(&reopened) == image(&d),
        "Native117 bone/pose output differs",
    )?;
    let mut bytes = std::fs::read(&native).map_err(|e| e.to_string())?;
    crate::save_generation::verify::legacy_version(&mut bytes, 116);
    std::fs::write(root.join("old.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "Native116 accepted model controls",
    )?;
    let edit = p(
        &d,
        Edit::Weight {
            model: 2,
            key: 0,
            value: 0.,
        },
    );
    check(edit.is_err(), "Missing key accepted")?;
    let next = p(
        &d,
        Edit::Parent {
            child: 2,
            parent: None,
            joint: None,
            keep: Keep::World,
        },
    )?;
    d.current_frame = 6;
    check(next.commit(&mut d).is_err(), "Stale model frame committed")?;
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            prepare(
                &d,
                Edit::Parent {
                    child: 2,
                    parent: None,
                    joint: None,
                    keep: Keep::World,
                },
                budget,
                &AtomicBool::new(cancel),
            )
            .is_err(),
            "Model budget/cancel ignored",
        )?;
    }
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("model-edit-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        morphs()?;
        limits_and_sides()?;
        hierarchy()?;
        timeline_native(&dir)?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn expressions_shape_key_boundaries_capture_independent_copy_and_reset() {
        super::morphs().unwrap();
    }
    #[test]
    fn joint_axis_locks_ranges_verified_side_mapping_and_atomic_copy() {
        super::limits_and_sides().unwrap();
    }
    #[test]
    fn bone_parent_world_local_cycle_follow_offset_duplicate_and_detach() {
        super::hierarchy().unwrap();
    }
    #[test]
    fn animated_bone_parent_world_preservation_native117_budget_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::timeline_native(&t.0).unwrap();
    }
}
