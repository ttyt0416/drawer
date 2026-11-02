use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    human_rig::{self, Kind, Rig},
    model_presets::*,
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn blank() -> Result<Document, String> {
    crate::model_library_verify::blank()
}
fn rig(d: &mut Document, kind: Kind) -> Result<(), String> {
    human_rig::prepare(
        d,
        human_rig::Edit::Create {
            rig: Rig::new(kind),
            position: [0.; 3],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(d)?;
    Ok(())
}
fn replace(d: &mut Document, r: Rig) -> Result<(), String> {
    human_rig::prepare(
        d,
        human_rig::Edit::Replace { id: 1, rig: r },
        B,
        &AtomicBool::new(false),
    )?
    .commit(d)?;
    Ok(())
}
pub fn adoption() -> Result<(), String> {
    for kind in [Kind::Body, Kind::Head, Kind::Hand] {
        let mut d = blank()?;
        rig(&mut d, kind)?;
        let mut r = d.scene.world3d.rigs[&1].clone();
        r.angles[1][0] = 40.;
        r.shape.height = 2.1;
        if kind == Kind::Hand {
            r.mirror();
        }
        replace(&mut d, r.clone())?;
        let pose = Preset::capture(&d, Some(1), Scope::Pose, "Pose".into())?;
        let shape = Preset::capture(&d, Some(1), Scope::Shape, "Shape".into())?;
        let mut target = r.clone();
        target.reset_pose();
        target.shape.height = 1.4;
        replace(&mut d, target.clone())?;
        let before = d.snapshot();
        let counts = d.history_counts();
        let preview = pose.prepare(&d, Some(1), B, &AtomicBool::new(false))?;
        check(
            d.snapshot() == before && d.history_counts() == counts,
            "Pose preview changed source / history",
        )?;
        drop(preview);
        check(d.snapshot() == before, "Cancel changed pose")?;
        pose.prepare(&d, Some(1), B, &AtomicBool::new(false))?
            .commit(&mut d)?;
        check(
            d.scene.world3d.rigs[&1].angles == r.angles
                && d.scene.world3d.rigs[&1].shape == target.shape
                && d.scene.world3d.rigs[&1].mirrored == r.mirrored,
            "Pose altered shape / lost hand side",
        )?;
        let posed = d.snapshot();
        d.undo();
        check(d.snapshot() == before, "Preset Undo failed")?;
        d.redo();
        check(d.snapshot() == posed, "Preset Redo failed")?;
        let frame = d.scene.world3d.nodes[&1];
        shape
            .prepare(&d, Some(1), B, &AtomicBool::new(false))?
            .commit(&mut d)?;
        check(
            d.scene.world3d.rigs[&1] == r && d.scene.world3d.nodes[&1] == frame,
            "Shape changed pose / placement",
        )?;
        let camera = Preset::capture(&d, None, Scope::Camera, "Camera".into())?;
        let (c, mut l) = world3d::default_camera(&d);
        l.fov = 62.;
        l.angles = [12., 18., 0.];
        world3d::prepare(
            &d,
            world3d::Edit::Camera { camera: c, lens: l },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
        let before = d.snapshot();
        let p = camera.prepare(&d, None, B, &AtomicBool::new(false))?;
        check(d.snapshot() == before, "Camera preview changed source")?;
        p.commit(&mut d)?;
        check(
            d.scene.world3d.lens.unwrap().fov == 45. && d.scene.world3d.rigs[&1] == r,
            "Camera preset changed rig / failed",
        )?;
    }
    Ok(())
}
pub fn guards() -> Result<(), String> {
    let mut d = blank()?;
    check(
        Preset::capture(&d, None, Scope::Camera, "Empty".into()).is_err(),
        "Missing camera capture accepted",
    )?;
    rig(&mut d, Kind::Body)?;
    let p = Preset::capture(&d, Some(1), Scope::Pose, "Pose".into())?;
    let mut target = blank()?;
    rig(&mut target, Kind::Hand)?;
    let before = target.snapshot();
    check(
        p.prepare(&target, Some(1), B, &AtomicBool::new(false))
            .err()
            .unwrap_or_default()
            .contains("Skeleton mismatch")
            && target.snapshot() == before,
        "Skeleton mismatch did not reject / preserve source",
    )?;
    human_rig::prepare(
        &target,
        human_rig::Edit::Detach { id: 1 },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut target)?;
    check(
        p.prepare(&target, Some(1), B, &AtomicBool::new(false))
            .is_err()
            && Preset::capture(&target, Some(1), Scope::Shape, "No rig".into()).is_err(),
        "Rigless model accepted preset",
    )?;
    let before = d.snapshot();
    check(
        p.prepare(&d, Some(1), 1, &AtomicBool::new(false)).is_err()
            && p.prepare(&d, Some(1), B, &AtomicBool::new(true)).is_err()
            && d.snapshot() == before,
        "Budget / cancel changed source",
    )?;
    let stale = p.prepare(&d, Some(1), B, &AtomicBool::new(false))?;
    d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = d.snapshot();
    check(
        stale.commit(&mut d).is_err() && d.snapshot() == before,
        "Stale preset committed",
    )?;
    let mut bad = p.clone();
    if let Value::Pose(v) = &mut bad.value {
        v.skeleton.joints[1].1 = None;
    }
    check(!bad.valid(), "Incorrect skeleton structure accepted")?;
    bad = p.clone();
    if let Value::Pose(v) = &mut bad.value {
        v.angles[0][0] = f64::NAN;
    }
    check(!bad.valid(), "NaN preset accepted")?;
    for name in ["", " ", "Invalid\nName"] {
        check(
            Preset::capture(&d, Some(1), Scope::Pose, name.into()).is_err(),
            "Invalid asset name accepted",
        )?;
    }
    let mut lib = Library::default();
    for _ in 0..128 {
        lib.add(p.clone())?;
    }
    let before = lib.clone();
    check(
        lib.add(p.clone()).is_err() && lib == before,
        "Preset count overflow mutated library",
    )?;
    lib.entries[1].id = lib.entries[0].id;
    check(!lib.valid(), "Duplicate preset IDs accepted")?;
    let mut scene = d.scene.clone();
    scene
        .catalog
        .world_info
        .entry(1)
        .or_default()
        .properties
        .locked = true;
    d.edit_scene(scene)?;
    check(
        p.prepare(&d, Some(1), B, &AtomicBool::new(false)).is_err(),
        "Locked rig accepted preset",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    adoption()?;
    guards()?;
    let dir = root.join("model-preset-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = blank()?;
        rig(&mut d, Kind::Body)?;
        let mut lib = Library::load(&dir)?;
        let before = d.snapshot();
        let counts = d.history_counts();
        for scope in [Scope::Pose, Scope::Shape, Scope::Camera] {
            lib.add(Preset::capture(&d, Some(1), scope, "로컬 프리셋".into())?)?;
        }
        lib.save(&dir)?;
        check(
            Library::load(&dir)? == lib && d.snapshot() == before && d.history_counts() == counts,
            "Library restart / capture independence failed",
        )?;
        let path = dir.join("settings/model-presets.json");
        let original = std::fs::read(&path).map_err(|e| e.to_string())?;
        let mut bad = lib.clone();
        bad.version = 2;
        check(
            bad.save(&dir).is_err() && std::fs::read(&path).map_err(|e| e.to_string())? == original,
            "Invalid save destroyed library",
        )?;
        lib.remove(2)?;
        lib.save(&dir)?;
        check(
            Library::load(&dir)? == lib && lib.next == 4,
            "Delete reused IDs / did not persist",
        )?;
        std::fs::write(&path, b"{bad").map_err(|e| e.to_string())?;
        check(
            Library::load(&dir).is_err()
                && std::fs::read(&path).map_err(|e| e.to_string())? == b"{bad",
            "Corrupt load destroyed asset",
        )?;
        std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).map_err(|e| e.to_string())?;
        check(
            Library::load(&dir).is_err(),
            "Oversize preset library accepted",
        )?;
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        check(
            lib.save(&dir).is_err() && path.is_dir(),
            "Failed publication replaced existing directory",
        )?;
        check(
            std::fs::read_dir(dir.join("settings"))
                .map_err(|e| e.to_string())?
                .count()
                == 1,
            "Preset write left staging files",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn pose_shape_camera_preview_cancel_adopt_and_undo() {
        super::adoption().unwrap();
    }
    #[test]
    fn skeleton_rigless_limits_lock_budget_cancel_and_stale() {
        super::guards().unwrap();
    }
    #[test]
    fn local_library_restart_delete_atomic_failure_corrupt_and_limits() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
