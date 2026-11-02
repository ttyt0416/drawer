use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    human_rig::{self as rig, *},
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn prep(d: &Document, e: Edit) -> Result<world3d::Prepared, String> {
    rig::prepare(d, e, B, &AtomicBool::new(false))
}
fn blank() -> Result<Document, String> {
    crate::model_library_verify::blank()
}
fn create(d: &mut Document, r: Rig) -> Result<(), String> {
    prep(
        d,
        Edit::Create {
            rig: r,
            position: [0.; 3],
        },
    )?
    .commit(d)?;
    Ok(())
}
pub fn joints() -> Result<(), String> {
    for (kind, count) in [(Kind::Body, 52), (Kind::Head, 3), (Kind::Hand, 16)] {
        let mut r = Rig::new(kind);
        check(
            r.valid() && r.joints().len() == count,
            "Skeleton size / default invalid",
        )?;
        for (i, j) in r.joints().iter().enumerate() {
            for angle in [j.min, j.max] {
                r.angles[i] = angle;
                check(r.valid(), "Joint endpoint rejected")?;
            }
            r.angles[i] = [0.; 3];
            for axis in 0..3 {
                r.angles[i][axis] = j.max[axis] + 0.01;
                check(!r.valid(), "Joint limit bypass")?;
                r.angles[i][axis] = j.min[axis] - 0.01;
                check(!r.valid(), "Joint lower limit bypass")?;
                r.angles[i][axis] = f64::NAN;
                check(!r.valid(), "NaN joint accepted")?;
                r.angles[i][axis] = 0.;
            }
        }
        let mut d = blank()?;
        create(&mut d, r.clone())?;
        let before = d.snapshot();
        let j = r
            .joints()
            .iter()
            .position(|j| j.name.ends_with("Index1"))
            .unwrap_or(1);
        r.angles[j] = [45., 0., 0.];
        if kind == Kind::Head {
            r.angles[j] = [30., 15., 0.];
        }
        let original = r.clone();
        r.mirror();
        check(r.valid(), "Mirrored pose violates limits")?;
        r.mirror();
        check(r == original, "Mirror twice differs")?;
        let p = prep(
            &d,
            Edit::Replace {
                id: 1,
                rig: r.clone(),
            },
        )?;
        check(d.snapshot() == before, "Pose preview mutated original")?;
        p.commit(&mut d)?;
        let posed = d.snapshot();
        check(
            posed != before && d.scene.world3d.valid(&d.scene),
            "Finger / head pose did not change valid mesh",
        )?;
        d.undo();
        check(d.snapshot() == before, "Pose Undo failed")?;
        d.redo();
        check(d.snapshot() == posed, "Pose Redo failed")?;
        let oldframe = d.scene.world3d.nodes[&1];
        r.shape.bulk = 1.5;
        r.shape.face = 1.4;
        r.shape.fingers = 1.6;
        r.shape.height = 2.4;
        prep(&d, Edit::Replace { id: 1, rig: r })?.commit(&mut d)?;
        check(
            d.scene.world3d.nodes[&1] == oldframe && d.snapshot() != posed,
            "Shape lost root frame / did not change geometry",
        )?;
    }
    let mut r = Rig::new(Kind::Body);
    let joints = r.joints();
    let right = joints.iter().position(|j| j.name == "R/Elbow").unwrap();
    let left = joints.iter().position(|j| j.name == "L/Elbow").unwrap();
    r.angles[right] = [80., 20., 0.];
    let before = r.frames();
    r.mirror();
    let after = r.frames();
    check(
        r.angles[left] == [80., -20., 0.] && r.angles[right] == [0.; 3],
        "Bilateral angles did not swap",
    )?;
    for (i, j) in joints.iter().enumerate() {
        let name = if let Some(n) = j.name.strip_prefix("R/") {
            format!("L/{n}")
        } else if let Some(n) = j.name.strip_prefix("L/") {
            format!("R/{n}")
        } else {
            j.name.clone()
        };
        let k = joints.iter().position(|j| j.name == name).unwrap();
        let a = before[i].origin();
        let b = after[k].origin();
        check(
            (a[0] + b[0]).abs() < 1e-8 && (a[1] - b[1]).abs() < 1e-8 && (a[2] - b[2]).abs() < 1e-8,
            "Mirrored joint coordinates are not reflections",
        )?;
    }
    let mut r = Rig::new(Kind::Hand);
    r.angles[4] = [35., 10., 5.];
    let a = r.frames();
    r.mirror();
    for (a, b) in a.iter().zip(r.frames()) {
        let a = a.origin();
        let b = b.origin();
        check(
            (a[0] + b[0]).abs() < 1e-8 && (a[1] - b[1]).abs() < 1e-8 && (a[2] - b[2]).abs() < 1e-8,
            "Hand mirror coordinates differ",
        )?;
    }
    Ok(())
}
pub fn transforms() -> Result<(), String> {
    for kind in [Kind::Body, Kind::Head, Kind::Hand] {
        let d = blank()?;
        let a = Rig::new(kind);
        let mut b = a.clone();
        b.shape.height = 2.4;
        let first = prep(
            &d,
            Edit::Create {
                rig: a,
                position: [0.; 3],
            },
        )?;
        let second = prep(
            &d,
            Edit::Create {
                rig: b,
                position: [0.; 3],
            },
        )?;
        check(
            first.scene().world3d.nodes == second.scene().world3d.nodes
                && first.scene().worlds != second.scene().worlds,
            "New preview refitted away the shape change",
        )?;
    }

    let mut d = blank()?;
    create(&mut d, Rig::new(Kind::Body))?;
    for (mode, value) in [
        (world3d::Mode::Scale, [-1.2, 0.8, 1.6]),
        (world3d::Mode::Rotate, [18., 36., 24.]),
        (world3d::Mode::Move, [1., -0.5, 0.3]),
    ] {
        world3d::prepare(
            &d,
            world3d::Edit::Transform {
                ids: vec![1],
                options: world3d::Preferences {
                    mode,
                    ..Default::default()
                },
                value,
            },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
    }
    let frame = d.scene.world3d.nodes[&1];
    let mut r = d.scene.world3d.rigs[&1].clone();
    r.angles[1][0] = 20.;
    prep(&d, Edit::Replace { id: 1, rig: r })?.commit(&mut d)?;
    check(
        d.scene.world3d.nodes[&1] == frame,
        "Pose discarded nonuniform reflected root",
    )?;
    crate::model_library::prepare(
        &d,
        crate::model_library::Edit::Duplicate {
            ids: vec![1],
            offset: [2., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.rigs[&1] == d.scene.world3d.rigs[&2],
        "Duplicate omitted rig",
    )?;
    let old = d.scene.world3d.rigs[&1].clone();
    let mut r = old.clone();
    r.shape.hips = 1.4;
    prep(&d, Edit::Replace { id: 2, rig: r })?.commit(&mut d)?;
    check(
        d.scene.world3d.rigs[&1] == old,
        "Duplicate rig aliases original",
    )?;
    let before = d.snapshot();
    prep(&d, Edit::Detach { id: 2 })?.commit(&mut d)?;
    check(
        d.scene.worlds == before.scene.worlds && !d.scene.world3d.rigs.contains_key(&2),
        "Detach changed pose geometry",
    )?;
    check(
        prep(&d, Edit::Replace { id: 2, rig: old })
            .err()
            .unwrap_or_default()
            .contains("no rig"),
        "Rigless model error missing",
    )?;
    d.undo();
    check(d.snapshot() == before, "Detach Undo lost rig")?;
    let mut s = d.scene.clone();
    s.worlds[0].vertices[0][0] += 0.1;
    check(!s.world3d.valid(&s), "Rig / skin mismatch accepted")?;
    for flag in 0..4 {
        let mut s = d.scene.clone();
        let p = &mut s.catalog.world_info.entry(1).or_default().properties;
        match flag {
            0 => p.hidden = true,
            1 => p.locked = true,
            2 => p.pixels_locked = true,
            _ => p.position_locked = true,
        };
        d.edit_scene(s)?;
        let before = d.snapshot();
        check(
            prep(&d, Edit::Detach { id: 1 }).is_err() && d.snapshot() == before,
            "Locked / hidden rig edited",
        )?;
        d.undo();
    }
    let before = d.snapshot();
    check(
        rig::prepare(&d, Edit::Detach { id: 1 }, 1, &AtomicBool::new(false)).is_err()
            && rig::prepare(&d, Edit::Detach { id: 1 }, B, &AtomicBool::new(true)).is_err()
            && d.snapshot() == before,
        "Budget / cancel changed source",
    )?;
    let p = prep(&d, Edit::Detach { id: 1 })?;
    d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let changed = d.snapshot();
    check(
        p.commit(&mut d).is_err() && d.snapshot() == changed,
        "Stale preview committed",
    )?;
    Ok(())
}
fn pixels(d: &Document) -> Vec<u8> {
    let r = crate::render::Renderer::new(d);
    let mut out = vec![];
    for y in 0..64 {
        for x in 0..64 {
            out.extend(r.pixel([x as f64 + 0.5, y as f64 + 0.5]));
        }
    }
    out
}
pub fn verify(root: &Path) -> Result<(), String> {
    joints()?;
    transforms()?;
    let dir = root.join("human-rig-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = blank()?;
        for kind in [Kind::Body, Kind::Head, Kind::Hand] {
            let mut r = Rig::new(kind);
            r.angles[1][0] = 30.;
            create(&mut d, r)?;
        }
        let before = d.snapshot();
        let p = dir.join("rig.drawer");
        crate::files::save(&mut d, &p).map_err(|e| e.to_string())?;
        let opened = crate::files::open(&p, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == before && pixels(&opened) == pixels(&d),
            "Native109 rig / rendered pose differs",
        )?;
        check(
            pixels(&d).as_chunks::<4>().0.iter().any(|p| p[3] != 0),
            "Mannequin renders no pixels",
        )?;
        let png = dir.join("pose.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        check(
            image::open(png)
                .map_err(|e| e.to_string())?
                .to_rgba8()
                .into_raw()
                == pixels(&d),
            "Exported pose differs",
        )?;
        let mut b = std::fs::read(&p).map_err(|e| e.to_string())?;
        b[8..12].copy_from_slice(&108u32.to_le_bytes());
        std::fs::write(&p, b).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&p, B).is_err(),
            "Native108 accepted rig data",
        )?;
        let mut bad = Rig::new(Kind::Hand);
        bad.angles.pop();
        check(!bad.valid(), "Wrong joint count accepted")?;
        bad = Rig::new(Kind::Head);
        bad.shape.face = f64::INFINITY;
        check(!bad.valid(), "Infinite shape accepted")?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn joint_limits_fingers_shapes_and_mirror() {
        super::joints().unwrap();
    }
    #[test]
    fn root_transforms_copy_detach_locks_budget_and_stale() {
        super::transforms().unwrap();
    }
    #[test]
    fn native109_and_actual_pose_png() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
