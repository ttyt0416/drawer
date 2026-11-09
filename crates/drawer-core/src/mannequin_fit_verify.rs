use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    human_rig::{self, Kind, Rig, Shape},
    mannequin_fit::*,
    model_edit,
    world3d::{self, Matrix},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn edit(d: &mut Document, c: Change) -> Result<bool, String> {
    prepare(d, 1, c, false, B, &AtomicBool::new(false))?.commit(d)
}
fn fixture(kind: Kind) -> Result<Document, String> {
    crate::model_edit_verify::fixture(kind)
}
pub fn forms() -> Result<(), String> {
    for kind in [Kind::Body, Kind::Head, Kind::Hand] {
        let mut d = fixture(kind)?;
        let original = d.snapshot();
        let mut form = Form {
            parts: vec![[2., 0.5, 2.]; d.scene.world3d.rigs[&1].angles.len()],
            strength: 2.,
            toward: [0., 1., 0.],
        };
        edit(&mut d, Change::Form(form.clone()))?;
        check(
            d.scene.world3d.valid(&d.scene),
            "Extreme part scales/perspective invalid",
        )?;
        let modified = d.snapshot();
        check(original != modified, "Part edit did not affect geometry")?;
        d.undo();
        check(
            d.snapshot() == original,
            "Part/perspective not restored in one Undo",
        )?;
        d.redo();
        check(d.snapshot() == modified, "Part redo differs")?;
        let r = &d.scene.world3d.rigs[&1];
        let mesh = human_rig::generate(r, &AtomicBool::new(false))?.0;
        check(
            mesh.vertices
                .iter()
                .flatten()
                .all(|x| x.is_finite() && x.abs() < 100.),
            "Extreme mannequin exploded",
        )?;
        let skeleton = crate::pose_sequence::Skeleton::of(r);
        check(
            skeleton == crate::pose_sequence::Skeleton::of(&Rig::new(kind)),
            "Scale/perspective changed skeleton compatibility",
        )?;
        let node = d.scene.world3d.nodes[&1];
        check(
            human_rig::binding_valid(r, &d.scene.worlds[0], node.frame),
            "Deformation mesh and joint metadata differ",
        )?;
        for strength in [0., 2.] {
            form.strength = strength;
            edit(&mut d, Change::Form(form.clone()))?;
        }
        edit(&mut d, Change::Form(Form::default()))?;
        check(
            d.snapshot().scene == original.scene,
            "Reset did not restore undeformed mannequin",
        )?;
        for bad in [f64::NAN, -0.001, 2.001] {
            form.strength = bad;
            let before = d.snapshot();
            check(
                prepare(
                    &d,
                    1,
                    Change::Form(form.clone()),
                    false,
                    B,
                    &AtomicBool::new(false),
                )
                .is_err()
                    && d.snapshot() == before,
                "Invalid perspective mutated state",
            )?;
        }
        form.strength = 0.;
        form.parts[0][0] = 0.49;
        check(
            prepare(
                &d,
                1,
                Change::Form(form.clone()),
                false,
                B,
                &AtomicBool::new(false),
            )
            .is_err(),
            "Part minimum bypass",
        )?;
        form.parts[0][0] = 2.01;
        check(!form.valid(form.parts.len()), "Part maximum bypass")?;
        form.parts.clear();
        form.toward = [0.; 3];
        check(!form.valid(52), "Zero perspective axis accepted")?;
    }
    let f = Form {
        strength: 1.,
        ..Default::default()
    };
    let a = f.point([0.2, 0., 0.8], 1.8);
    let b = f.point([0.2, 0., -0.8], 1.8);
    check(
        a[0] > 0.2 && b[0] < 0.2,
        "Near/far manga exaggeration absent",
    )?;
    let m = Matrix::euler([10., 20., 30.]).then(Matrix::translation([0.2, 0.1, 0.3]));
    let frame = f.frame(m, 1.8);
    for axis in 0..3 {
        let mut q = [0.; 3];
        q[axis] = 1e-7;
        let v = f.point(m.map(q), 1.8);
        let base = f.point(m.origin(), 1.8);
        for i in 0..3 {
            check(
                ((v[i] - base[i]) / 1e-7 - frame.0[i][axis]).abs() < 1e-6,
                "Bone attachment tangent differs from deformed mesh",
            )?;
        }
    }
    Ok(())
}
pub fn mixing() -> Result<(), String> {
    let a = Shape::default();
    let b = Shape {
        height: 3.,
        shoulders: 2.,
        hips: 0.5,
        bulk: 2.,
        head: 2.,
        hands: 0.5,
        face: 0.5,
        jaw: 2.,
        palm: 2.,
        fingers: 0.5,
    };
    check(
        blend(a, b, 0., Region::Body)? == a && blend(a, b, 1., Region::Body)? == b,
        "Body blend endpoints differ",
    )?;
    let h = blend(a, b, 1., Region::Head)?;
    check(
        h.head == 2. && h.face == 0.5 && h.jaw == 2. && h.hands == 1. && h.height == 1.8,
        "Head blend leaked outside region",
    )?;
    let hand = blend(a, b, 0.5, Region::Hand)?;
    check(
        hand.hands == 0.75 && hand.fingers == 0.75 && hand.palm == 1.5 && hand.head == 1.,
        "Hand blend not independent",
    )?;
    for (kind, region) in [
        (Kind::Body, Region::Body),
        (Kind::Head, Region::Head),
        (Kind::Hand, Region::Hand),
    ] {
        let mut d = fixture(kind)?;
        let before = d.snapshot();
        edit(
            &mut d,
            Change::Blend {
                target: b,
                weight: 0.5,
                region,
            },
        )?;
        check(d.scene.world3d.valid(&d.scene), "Blended geometry invalid")?;
        d.undo();
        check(d.snapshot() == before, "Regional blend Undo differs")?;
    }
    for weight in [-0.01, 1.01, f64::NAN] {
        check(
            blend(a, b, weight, Region::Head).is_err(),
            "Invalid blend weight accepted",
        )?;
    }
    Ok(())
}
pub fn assistance() -> Result<(), String> {
    let mut d = fixture(Kind::Body)?;
    let mut r = d.scene.world3d.rigs[&1].clone();
    let joints = r.joints();
    let spine = joints.iter().position(|j| j.name == "Spine").unwrap();
    r.angles[spine] = [25., 10., 20.];
    let knee = joints.iter().position(|j| j.name == "R/Knee").unwrap();
    r.angles[knee][0] = 90.;
    let mut controls = model_edit::controls(&r);
    controls[spine].locked[1] = true;
    controls[spine].hold = r.angles[spine];
    r.controls = controls;
    human_rig::prepare(
        &d,
        human_rig::Edit::Replace { id: 1, rig: r },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    world3d::prepare(
        &d,
        world3d::Edit::Transform {
            ids: vec![1],
            options: world3d::Preferences {
                mode: world3d::Mode::Rotate,
                ..Default::default()
            },
            value: [0., 0., 30.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let before = d.snapshot();
    let p = prepare(
        &d,
        1,
        Change::Assist {
            collision: false,
            upright: true,
            ground: Some(-0.25),
        },
        false,
        B,
        &AtomicBool::new(false),
    )?;
    check(d.snapshot() == before, "Assist preview changed original")?;
    p.commit(&mut d)?;
    let rig = &d.scene.world3d.rigs[&1];
    check(
        rig.angles[spine] == [0., 10., 0.] && rig.angles[knee][0] == 0.,
        "Upright ignored locks or did not straighten",
    )?;
    let frame = d.scene.world3d.nodes[&1].frame;
    check(
        (frame.axis(1)[1] - 1.).abs() < 1e-9,
        "Upright did not align world up",
    )?;
    let low = human_rig::feet(rig, &AtomicBool::new(false))?
        .into_iter()
        .map(|p| frame.map(p)[1])
        .fold(f64::INFINITY, f64::min);
    check(
        (low + 0.25).abs() < 1e-8,
        "Ground foot geometry differs from selected floor",
    )?;
    check(d.scene.world3d.valid(&d.scene), "Assisted scene invalid")?;
    d.undo();
    check(d.snapshot() == before, "Combined stand/ground not one Undo")?;
    let mut r = Rig::new(Kind::Body);
    r.shape.shoulders = 0.5;
    r.shape.hips = 2.;
    r.shape.bulk = 1.5;
    for (i, j) in r.joints().iter().enumerate() {
        if j.name == "R/UpperArm" {
            r.angles[i][2] = -90.;
        }
        if j.name == "L/UpperArm" {
            r.angles[i][2] = 90.;
        }
    }
    check(penetration(&r)? > 1e-8, "Collision fixture has no overlap")?;
    let fixed = separate(&r, &AtomicBool::new(false))?;
    check(
        penetration(&fixed)? <= 1e-10 && fixed.valid(),
        "Limb separation left an overlap",
    )?;
    let mut locked = r.clone();
    locked.controls = model_edit::controls(&locked);
    for c in &mut locked.controls {
        c.locked = [true; 3];
    }
    check(
        separate(&locked, &AtomicBool::new(false)).is_err(),
        "Impossible locked collision silently accepted",
    )?;
    check(
        separate(&r, &AtomicBool::new(true)).is_err(),
        "Collision ignored cancellation",
    )?;
    Ok(())
}
pub fn persistence(root: &Path) -> Result<(), String> {
    let mut d = fixture(Kind::Body)?;
    let form = Form {
        parts: vec![[1.1, 0.8, 1.2]; 52],
        strength: 0.9,
        toward: [0., 0., -1.],
    };
    edit(&mut d, Change::Form(form))?;
    model_edit::prepare(
        &d,
        model_edit::Edit::Parent {
            child: 2,
            parent: Some(1),
            joint: Some(8),
            keep: model_edit::Keep::World,
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let before = d.snapshot();
    let mut form = d.scene.world3d.rigs[&1].form.clone();
    form.parts[8] = [1.5, 1.5, 0.8];
    edit(&mut d, Change::Form(form))?;
    check(
        d.scene.world3d.valid(&d.scene) && d.scene.worlds[1] != before.scene.worlds[1],
        "Attached prop failed to follow part changes",
    )?;
    let snap = d.snapshot();
    let image = crate::model_edit_verify::image(&d);
    let file = root.join("mannequin-fit.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == snap && crate::model_edit_verify::image(&reopened) == image,
        "Native118 mannequin geometry or pixels differ",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&117u32.to_le_bytes());
    std::fs::write(root.join("old-fit.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old-fit.drawer"), B).is_err(),
        "Native117 accepted new mannequin metadata",
    )?;
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            prepare(
                &d,
                1,
                Change::Form(Form::default()),
                false,
                budget,
                &AtomicBool::new(cancel),
            )
            .is_err(),
            "Assist budget/cancel bypass",
        )?;
    }
    for id in [0, 999, 2] {
        check(
            prepare(
                &d,
                id,
                Change::Form(Form::default()),
                false,
                B,
                &AtomicBool::new(false),
            )
            .is_err(),
            "Missing/unrigged actor accepted",
        )?;
    }
    let pending = model_edit::prepare(
        &d,
        model_edit::Edit::Fit {
            rig: 1,
            change: Change::Form(Form::default()),
            base: false,
        },
        B,
        &AtomicBool::new(false),
    )?;
    d.current_frame += 1;
    check(
        pending.commit(&mut d).is_err(),
        "Stale frame form committed",
    )?;
    Ok(())
}
pub fn timelines() -> Result<(), String> {
    use crate::{
        keyframes::Curve,
        pose_sequence::{self, Edit},
    };
    let (mut d, asset) = crate::pose_sequence_verify::fixture()?;
    pose_sequence::prepare(
        &d,
        Edit::Bind {
            rig: 1,
            asset,
            start: 2,
            step: 3,
            repeats: 2,
            curve: Curve::Fixed,
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let old = d.snapshot();
    let form = Form {
        strength: 1.,
        parts: vec![[1.2, 0.8, 1.1]; 16],
        ..Default::default()
    };
    check(
        prepare(
            &d,
            1,
            Change::Form(form.clone()),
            false,
            B,
            &AtomicBool::new(false),
        )
        .is_err()
            && d.snapshot() == old,
        "Implicit base form edit replaced a bound pose",
    )?;
    prepare(&d, 1, Change::Form(form), true, B, &AtomicBool::new(false))?.commit(&mut d)?;
    check(
        d.scene.world3d.sequences == old.scene.world3d.sequences,
        "Form change rewrote stored pose keys",
    )?;
    let frozen = d.snapshot();
    d.current_frame = 2;
    let a = pose_sequence::meshes(&d)?;
    d.current_frame = 5;
    let b = pose_sequence::meshes(&d)?;
    check(
        a != b && d.snapshot() == frozen,
        "Deformed pose playback static or mutated base",
    )?;
    d.undo();
    check(
        d.snapshot() == old,
        "Base form Undo failed with pose sequence",
    )?;
    let head = fixture(Kind::Head)?;
    check(
        prepare(
            &head,
            1,
            Change::Assist {
                collision: false,
                upright: false,
                ground: Some(0.),
            },
            false,
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Head silently accepted foot assist",
    )?;
    let body = fixture(Kind::Body)?;
    for floor in [-1e6, 1e6] {
        let mut copy = fixture(Kind::Body)?;
        edit(
            &mut copy,
            Change::Assist {
                collision: false,
                upright: false,
                ground: Some(floor),
            },
        )?;
        check(
            copy.scene.world3d.valid(&copy.scene),
            "Floor endpoint invalid",
        )?;
    }
    for floor in [f64::NAN, 1e6 + 1.] {
        check(
            prepare(
                &body,
                1,
                Change::Assist {
                    collision: false,
                    upright: false,
                    ground: Some(floor),
                },
                false,
                B,
                &AtomicBool::new(false),
            )
            .is_err(),
            "Invalid floor accepted",
        )?;
    }
    check(
        prepare(
            &body,
            1,
            Change::Assist {
                collision: false,
                upright: false,
                ground: None,
            },
            false,
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Empty assistance accepted",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("mannequin-fit-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        forms()?;
        mixing()?;
        assistance()?;
        timelines()?;
        persistence(&dir)
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn explicit_pose_clip_scope_playback_undo_floor_boundaries_and_missing_parts() {
        super::timelines().unwrap();
    }

    #[test]
    fn part_scales_manga_perspective_extremes_reset_and_attachment_derivatives() {
        super::forms().unwrap();
    }
    #[test]
    fn independent_body_head_hand_mixing_boundaries_and_undo() {
        super::mixing().unwrap();
    }
    #[test]
    fn constrained_limb_separation_world_upright_actual_foot_ground_one_undo() {
        super::assistance().unwrap();
    }
    #[test]
    fn native118_form_pixels_bone_follow_budget_cancel_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::persistence(&t.0).unwrap();
    }
}
