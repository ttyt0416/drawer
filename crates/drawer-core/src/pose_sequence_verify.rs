use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    human_rig::{self, Kind, Rig},
    keyframes::Curve,
    pose_sequence::*,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
fn p(d: &Document, e: Edit) -> Result<Prepared, String> {
    prepare(d, e, B, &AtomicBool::new(false))
}
pub fn fixture() -> Result<(Document, u64), String> {
    let mut d = crate::model_library_verify::blank()?;
    human_rig::prepare(
        &d,
        human_rig::Edit::Create {
            rig: Rig::new(Kind::Hand),
            position: [0.; 3],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let mut scene = d.scene.clone();
    scene.world3d.lighting = Some(crate::lighting::Settings {
        style: crate::lighting::Style::Unlit,
        ..Default::default()
    });
    d.edit_scene(scene)?;
    let state = crate::timeline_editor::create(&d, 24, crate::animation::Fps::default())?;
    d.commit_state(state)?;
    let a = p(
        &d,
        Edit::Capture {
            rig: 1,
            name: "Local hand poses".into(),
            step: 3,
        },
    )?;
    let id = a.asset.unwrap();
    a.commit(&mut d)?;
    p(
        &d,
        Edit::Append {
            asset: id,
            rig: 1,
            sampled: false,
        },
    )?
    .commit(&mut d)?;
    let mut second = Pose::of(&d.scene.world3d.rigs[&1]);
    second.angles[0][2] = 45.;
    p(
        &d,
        Edit::Pose {
            asset: id,
            index: 1,
            pose: second,
        },
    )?
    .commit(&mut d)?;
    Ok((d, id))
}
fn bind(d: &mut Document, id: u64, curve: Curve) -> Result<(), String> {
    p(
        d,
        Edit::Bind {
            rig: 1,
            asset: id,
            start: 2,
            step: 3,
            repeats: 2,
            curve,
        },
    )?
    .commit(d)?;
    Ok(())
}
fn image(d: &Document) -> Vec<[u8; 4]> {
    let r = crate::render::Renderer::new(d);
    (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .map(|(x, y)| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect()
}
pub fn sampling() -> Result<(), String> {
    let (mut d, id) = fixture()?;
    let base = d.snapshot();
    let a = image(&d);
    d.current_frame = 5;
    check(
        image(&d) == a && meshes(&d)?.is_empty(),
        "Registration implicitly animated a rig",
    )?;
    d.current_frame = 0;
    bind(&mut d, id, Curve::Fixed)?;
    let state = d.snapshot();
    let poses = d.scene.world3d.sequences.assets[&id].clip.poses.clone();
    for (frame, index) in [
        (0, 0),
        (1, 0),
        (2, 0),
        (3, 0),
        (4, 0),
        (5, 1),
        (6, 1),
        (7, 1),
        (8, 0),
        (10, 0),
        (11, 1),
        (13, 1),
        (14, 1),
        (23, 1),
    ] {
        d.current_frame = frame;
        check(
            Pose::of(&sampled_rig(&d, 1)?) == poses[index],
            "Two poses / three-frame exposure / repeat / endpoint hold differs",
        )?;
    }
    d.current_frame = 2;
    let first = image(&d);
    d.current_frame = 5;
    check(
        image(&d) != first
            && d.scene.worlds == base.scene.worlds
            && d.scene.world3d.rigs == base.scene.world3d.rigs
            && d.scene.keyframes.is_default(),
        "Pose rendering changed base geometry / generated hidden2D keys / did not animate",
    )?;
    d.current_frame = 0;
    d.undo();
    check(d.snapshot() == base, "Explicit link not one Undo")?;
    d.redo();
    check(d.snapshot() == state, "Link Redo differs")?;
    let poses_before = d.scene.world3d.sequences.clone();
    crate::world3d::prepare(
        &d,
        crate::world3d::Edit::Transform {
            ids: vec![1],
            options: Default::default(),
            value: [1., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.keyframes.is_default() && d.scene.world3d.sequences == poses_before,
        "Ordinary world movement created pose/2D keys",
    )?;
    d.undo();
    p(
        &d,
        Edit::Order {
            asset: id,
            from: 1,
            to: 0,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.sequences.assets[&id].clip.poses[0].angles[0][2] == 45.,
        "Reorder lost pose",
    )?;
    d.undo();
    p(&d, Edit::Timing { asset: id, step: 5 })?.commit(&mut d)?;
    check(
        active(&d.scene, 1).unwrap().step == 3,
        "Preview timing silently changed existing placement",
    )?;
    d.undo();
    bind(&mut d, id, Curve::Linear)?;
    d.current_frame = 3;
    check(
        (sampled_rig(&d, 1)?.angles[0][2] - 15.).abs() < 1e-9,
        "Explicit linear pose interpolation wrong",
    )?;
    d.current_frame = 5;
    check(
        sampled_rig(&d, 1)?.angles[0][2] == 45.,
        "Segment endpoint interpolation differs",
    )?;
    d.current_frame = 0;
    let mut scene = d.scene.clone();
    scene.world3d.lighting.as_mut().unwrap().style = crate::lighting::Style::Smooth;
    d.edit_scene(scene)?;
    bind(&mut d, id, Curve::Fixed)?;
    let mut reflected = d.scene.world3d.sequences.assets[&id].clip.poses[1].clone();
    reflected.mirrored = true;
    p(
        &d,
        Edit::Pose {
            asset: id,
            index: 1,
            pose: reflected.clone(),
        },
    )?
    .commit(&mut d)?;
    check(
        p(
            &d,
            Edit::Bind {
                rig: 1,
                asset: id,
                start: 2,
                step: 3,
                repeats: 2,
                curve: Curve::Linear,
            },
        )
        .is_err(),
        "Mirrored discontinuity accepted interpolation",
    )?;
    d.current_frame = 5;
    let animated = image(&d);
    p(&d, Edit::Unbind(1))?.commit(&mut d)?;
    p(
        &d,
        Edit::BasePose {
            rig: 1,
            pose: reflected,
        },
    )?
    .commit(&mut d)?;
    check(
        image(&d) == animated,
        "Mirrored sequence shading differs from reflected base surface",
    )?;
    Ok(())
}
pub fn scopes_files_native(root: &Path) -> Result<(), String> {
    let (mut d, id) = fixture()?;
    bind(&mut d, id, Curve::Fixed)?;
    let base = d.scene.world3d.rigs[&1].clone();
    let mut changed = d.scene.world3d.sequences.assets[&id].clip.poses[1].clone();
    changed.angles[0][2] = 60.;
    let before = d.snapshot();
    p(
        &d,
        Edit::Pose {
            asset: id,
            index: 1,
            pose: changed,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.rigs[&1] == base
            && d.undo_label()
                .is_some_and(|s| s.contains("selected sequence pose")),
        "Pose scope changed base / missing history label",
    )?;
    d.undo();
    check(
        d.snapshot() == before,
        "Selected-pose Undo changed other data",
    )?;
    d.redo();
    let clip = d.scene.world3d.sequences.assets[&id].clone();
    let mut pose = Pose::of(&base);
    pose.angles[0][2] = 10.;
    p(&d, Edit::BasePose { rig: 1, pose })?.commit(&mut d)?;
    check(
        d.scene.world3d.sequences.assets[&id] == clip
            && d.scene.world3d.rigs[&1].angles[0][2] == 10.
            && d.undo_label().is_some_and(|s| s.contains("base scene")),
        "Base scope changed sequence/missing history label",
    )?;
    check(
        human_rig::prepare(
            &d,
            human_rig::Edit::Replace {
                id: 1,
                rig: base.clone(),
            },
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Ambiguous mannequin pose edit accepted while bound",
    )?;
    let original = d.snapshot();
    let state = p(&d, Edit::Unbind(1))?;
    state.commit(&mut d)?;
    check(
        sampled_rig(&d, 1)?.angles[0][2] == 10.,
        "Detach did not restore explicit base pose",
    )?;
    d.undo();
    check(
        d.snapshot() == original,
        "Detach Undo lost timing/base pose",
    )?;
    let file = root.join("snapshot.drawerposes");
    export(&d, id, &file)?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let loaded = p(&d, Edit::Load(file.clone()))?;
    let loaded_id = loaded.asset.unwrap();
    loaded.commit(&mut d)?;
    check(
        export(&d, loaded_id, &file).is_err(),
        "Export overwrote imported source",
    )?;
    let mut external: Sequence = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    external.poses[0].angles[0][2] = 20.;
    std::fs::write(&file, serde_json::to_vec(&external).unwrap()).map_err(|e| e.to_string())?;
    check(
        d.scene.world3d.sequences.assets[&loaded_id].clip.poses[0].angles[0][2] == 0.,
        "Source file change automatically changed document",
    )?;
    p(
        &d,
        Edit::Reload {
            asset: loaded_id,
            path: file.clone(),
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.sequences.assets[&loaded_id].clip.poses[0].angles[0][2] == 20.,
        "Explicit reload did not update snapshot",
    )?;
    let native = root.join("poses.drawer");
    let state = d.snapshot();
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    std::fs::remove_file(&file).map_err(|e| e.to_string())?;
    check(
        export(&d, loaded_id, &file).is_err(),
        "Export recreated read-only missing source path",
    )?;
    let reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == state,
        "Native115 sequence/placement/base pose differs",
    )?;
    let mut raw = std::fs::read(&native).map_err(|e| e.to_string())?;
    crate::save_generation::verify::legacy_version(&mut raw, 114);
    std::fs::write(root.join("old.drawer"), raw).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "Native114 accepted pose sequences",
    )?;
    for frame in [0, 2, 5, 8, 13, 23] {
        let mut reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
        d.current_frame = frame;
        reopened.current_frame = frame;
        check(image(&d) == image(&reopened), "Native posed frame differs")?;
    }
    Ok(())
}
pub fn validation(root: &Path) -> Result<(), String> {
    let mut mirrored = Rig::new(Kind::Hand);
    mirrored.mirrored = true;
    check(
        Sequence::capture(&mirrored, "Mirrored hand".into(), 3)?.valid(),
        "Mirrored hand changed skeleton compatibility",
    )?;
    for kind in [Kind::Hand, Kind::Body, Kind::Head] {
        let mut rig = Rig::new(kind);
        rig.shape.height = 1.5;
        check(
            Sequence::capture(&rig, "Shaped rig".into(), 3)?.valid(),
            "Shape proportions changed skeleton axes",
        )?;
    }

    let (mut d, id) = fixture()?;
    let original = d.snapshot();
    for edit in [
        Edit::Bind {
            rig: 1,
            asset: id,
            start: u32::MAX,
            step: 3,
            repeats: 1,
            curve: Curve::Fixed,
        },
        Edit::Bind {
            rig: 1,
            asset: id,
            start: 0,
            step: 0,
            repeats: 1,
            curve: Curve::Fixed,
        },
        Edit::Bind {
            rig: 1,
            asset: id,
            start: 0,
            step: 3,
            repeats: 0,
            curve: Curve::Fixed,
        },
        Edit::Pose {
            asset: id,
            index: 99,
            pose: Pose::of(&d.scene.world3d.rigs[&1]),
        },
        Edit::Timing { asset: id, step: 0 },
    ] {
        check(
            p(&d, edit).is_err() && d.snapshot() == original,
            "Invalid pose/timing mutated state",
        )?;
    }
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            prepare(
                &d,
                Edit::Rename {
                    asset: id,
                    name: "Other".into(),
                },
                budget,
                &AtomicBool::new(cancel),
            )
            .is_err()
                && d.snapshot() == original,
            "Pose budget/cancel mutated document",
        )?;
    }
    let prepared = p(
        &d,
        Edit::Rename {
            asset: id,
            name: "Other".into(),
        },
    )?;
    d.current_frame = 1;
    check(
        prepared.commit(&mut d).is_err(),
        "Changed frame accepted old pose edit",
    )?;
    d.current_frame = 0;
    let file = root.join("invalid.drawerposes");
    let mut seq = d.scene.world3d.sequences.assets[&id].clip.clone();
    seq.skeleton.bones[0].parent = Some(0);
    std::fs::write(&file, serde_json::to_vec(&seq).unwrap()).map_err(|e| e.to_string())?;
    check(
        Sequence::load(&file).is_err(),
        "Equal names with incompatible ancestry auto-mapped",
    )?;
    let mut raw = serde_json::to_value(&d.scene.world3d.sequences.assets[&id].clip).unwrap();
    raw["step"] = serde_json::json!(-1);
    std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).map_err(|e| e.to_string())?;
    check(
        Sequence::load(&file).is_err(),
        "Negative local pose time accepted",
    )?;
    p(
        &d,
        Edit::RemovePose {
            asset: id,
            index: 1,
        },
    )?
    .commit(&mut d)?;
    bind(&mut d, id, Curve::Fixed)?;
    d.current_frame = 23;
    check(
        sampled_rig(&d, 1)?.angles == d.scene.world3d.rigs[&1].angles,
        "Single pose did not hold",
    )?;
    d.current_frame = 0;
    p(
        &d,
        Edit::RemovePose {
            asset: id,
            index: 0,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.sequences.assets[&id].clip.poses.is_empty()
            && d.scene.world3d.sequences.bindings.is_empty()
            && p(
                &d,
                Edit::Bind {
                    rig: 1,
                    asset: id,
                    start: 0,
                    step: 1,
                    repeats: 1,
                    curve: Curve::Fixed,
                },
            )
            .is_err(),
        "Empty sequence remained bound",
    )?;
    Ok(())
}
pub fn independent_copies() -> Result<(), String> {
    let (mut d, id) = fixture()?;
    bind(&mut d, id, Curve::Fixed)?;
    let original = d.snapshot();
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
    let copied = active(&d.scene, 2)
        .ok_or("Duplicated rig lost pose binding")?
        .asset;
    check(copied != id, "Rig duplicate shares mutable pose asset")?;
    let mut pose = d.scene.world3d.sequences.assets[&copied].clip.poses[0].clone();
    pose.angles[0][2] = 15.;
    p(
        &d,
        Edit::Pose {
            asset: copied,
            index: 0,
            pose,
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.sequences.assets[&id].clip.poses[0].angles[0][2] == 0.,
        "Copied pose aliases original",
    )?;
    d.undo();
    d.undo();
    check(d.snapshot() == original, "Rig copy Undo lost sequences")?;
    let prepared = crate::timelines::prepare(
        &d,
        crate::timelines::Edit::Duplicate {
            name: "Other timing".into(),
            independent: false,
        },
        B,
        &AtomicBool::new(false),
    )?;
    d.commit_state(prepared.state)?;
    let next = d.scene.timelines.next - 1;
    check(
        d.scene
            .world3d
            .sequences
            .bindings
            .iter()
            .any(|b| b.timeline == next),
        "Timeline duplicate lost explicit pose channels",
    )?;
    let switch = crate::timelines::prepare(
        &d,
        crate::timelines::Edit::Switch(next),
        B,
        &AtomicBool::new(false),
    )?;
    d.commit_state(switch.state)?;
    d.current_frame = 5;
    check(
        sampled_rig(&d, 1)?.angles[0][2] == 45.,
        "Copied timeline does not evaluate pose channels",
    )?;
    let delete = crate::timelines::prepare(
        &d,
        crate::timelines::Edit::Delete(next),
        B,
        &AtomicBool::new(false),
    )?;
    d.commit_state(delete.state)?;
    check(
        d.scene
            .world3d
            .sequences
            .bindings
            .iter()
            .all(|b| b.timeline != next),
        "Deleted arrangement retained pose binding",
    )?;
    Ok(())
}
pub fn output_camera(root: &Path) -> Result<(), String> {
    let (mut d, id) = fixture()?;
    bind(&mut d, id, Curve::Fixed)?;
    d.current_frame = 5;
    let before = d.snapshot();
    let image1 = image(&d);
    let mut scene = d.scene.clone();
    scene.keyframes.camera_output = true;
    scene.keyframes.tracks.push(crate::keyframes::Track {
        owner: crate::keyframes::Owner::Camera,
        enabled: true,
        parent_camera: false,
        basis: Default::default(),
        basis_opacity: 100,
        hold: Default::default(),
        keys: vec![crate::keyframes::Keyframe {
            frame: 5,
            pose: crate::keyframes::Pose {
                offset: [8., 0.],
                ..Default::default()
            },
            curve: Curve::Fixed,
        }],
    });
    d.edit_scene(scene)?;
    let renderer = crate::render::Renderer::new(&d);
    let mut hits = 0;
    for y in 0..64 {
        for x in 8..64 {
            let a = image1[y * 64 + x];
            if a[3] > 0 {
                check(
                    renderer.pixel([x as f64 - 7.5, y as f64 + 0.5]) == a,
                    "2D camera did not apply after posed3D render",
                )?;
                hits += 1;
            }
        }
    }
    check(
        hits > 10 && d.scene.worlds == before.scene.worlds,
        "Camera test lacks posed image / changed base geometry",
    )?;
    let png = root.join("posed.png");
    crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
    let exported = image::open(png).map_err(|e| e.to_string())?.to_rgba8();
    for y in 0..64 {
        for x in 0..64 {
            check(
                exported.get_pixel(x, y).0 == renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                "Posed PNG differs",
            )?;
        }
    }
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("pose-sequence-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        sampling()?;
        scopes_files_native(&dir)?;
        validation(&dir)?;
        independent_copies()?;
        output_camera(&dir)?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn registration_explicit_fixed_linear_repeat_and_endpoint_sampling() {
        super::sampling().unwrap();
    }
    #[test]
    fn scopes_history_native115_local_snapshot_and_source_protection() {
        let t = crate::test_support::TempDir::new();
        super::scopes_files_native(&t.0).unwrap();
    }
    #[test]
    fn skeleton_empty_single_negative_cancel_budget_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::validation(&t.0).unwrap();
    }
    #[test]
    fn independent_rig_copies_and_timeline_arrangements() {
        super::independent_copies().unwrap();
    }
    #[test]
    fn posed_output_and_2d_camera_order() {
        let t = crate::test_support::TempDir::new();
        super::output_camera(&t.0).unwrap();
    }
}
