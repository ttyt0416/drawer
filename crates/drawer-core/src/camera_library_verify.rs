use crate::{
    camera_library::*,
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    world_math::{add, mul},
    world_views::{self, View},
    world3d::{Lens, Matrix, Projection},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn p(d: &Document, e: Edit) -> Result<Prepared, String> {
    prepare(d, e, B, &AtomicBool::new(false))
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::world_views_verify::fixture()?;
    crate::model_library::prepare(
        &d,
        crate::model_library::Edit::Duplicate {
            ids: vec![1],
            offset: [0., 0., -4.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let mut s = d.scene.clone();
    for (id, color) in [(1, [1., 0., 0., 1.]), (2, [0., 0., 1., 1.])] {
        s.world3d.surfaces.get_mut(&id).unwrap().materials[0].rgba = color;
    }
    s.world3d.lighting.as_mut().unwrap().shadows = false;
    d.edit_scene(s)?;
    Ok(d)
}
pub fn projection() -> Result<(), String> {
    let mut d = fixture()?;
    let mut view = View::scene(&d);
    view.camera.position = [0., 0., 5.];
    view.lens = Lens::default();
    p(&d, Edit::View(view.clone()))?.commit(&mut d)?;
    let original = crate::render::Renderer::new(&d).pixel([32.5, 32.5]);
    view.lens.shift = [0.125, 0.];
    p(&d, Edit::View(view.clone()))?.commit(&mut d)?;
    check(
        crate::render::Renderer::new(&d).pixel([40.5, 32.5]) == original && original[0] > 200,
        "Shifted ray render differs from shifted projection",
    )?;
    for projection in [Projection::Perspective, Projection::Orthographic] {
        view.lens.projection = projection;
        view.lens.shift = [0.2, -0.3];
        view.lens.angles = [-10., 25., 5.];
        let point = add(
            view.camera.position,
            Matrix::euler(view.lens.angles).vector([0.3, 0.1, -3.]),
        );
        let pixel = view
            .lens
            .project(&view.camera, point)
            .ok_or("Projection missing")?;
        let (o, dir) = view.lens.ray(&view.camera, pixel);
        let sample = add(o, mul(dir, 3.));
        check(
            sample.iter().zip(point).all(|(a, b)| (a - b).abs() < 1e-9),
            "Lens projection/ray inverse differs",
        )?;
    }
    view.lens.projection = Projection::Perspective;
    view.lens.angles = [-15., 25., 10.];
    let target = [0.; 3];
    let before = view.lens.project(&view.camera, target).unwrap();
    let corrected = straighten(&view, target)?;
    let after = corrected.lens.project(&corrected.camera, target).unwrap();
    check(
        (0..2).all(|i| (before[i] - after[i]).abs() < 1e-8) && corrected.lens.vertical,
        "Level correction moved target framing",
    )?;
    for x in [-1., 1.] {
        let a = corrected
            .lens
            .project(&corrected.camera, [x, -1., 0.])
            .unwrap();
        let b = corrected
            .lens
            .project(&corrected.camera, [x, 1., 0.])
            .unwrap();
        check(
            (a[0] - b[0]).abs() < 1e-8,
            "Corrected vertical pillars converge/slant",
        )?;
    }
    let mut moved = corrected.clone();
    moved.camera.position[0] += 0.2;
    moved.lens.angles[1] += 10.;
    moved.lens.fov = 50.;
    let next = moved
        .lens
        .project(&moved.camera, target)
        .ok_or("Level target clipped")?;
    check(
        moved.valid() && (0..2).all(|i| (next[i] - after[i]).abs() < 1e-8),
        "Vertical target did not follow camera position/yaw/FOV",
    )?;
    moved.lens.shift[0] += 0.1;
    check(
        (moved.lens.project(&moved.camera, target).unwrap()[0] - next[0] - 6.4).abs() < 1e-8,
        "Manual shift did not offset locked target framing",
    )?;
    let mut scene = d.scene.clone();
    scene.world3d.lens = Some(view.lens);
    let guides = crate::lighting::vanishing(&scene).unwrap();
    scene.world3d.lens.as_mut().unwrap().shift[0] += 0.125;
    scene.world3d.lens.as_mut().unwrap().shift[1] += 0.25;
    for (a, b) in guides
        .into_iter()
        .zip(crate::lighting::vanishing(&scene).unwrap())
    {
        if let (crate::lighting::Vanishing::Finite(a), crate::lighting::Vanishing::Finite(b)) =
            (a, b)
        {
            check(
                (b[0] - a[0] - 8.).abs() < 1e-8 && (b[1] - a[1] - 16.).abs() < 1e-8,
                "Lens shift missing from vanishing guides",
            )?;
        }
    }
    let mut invalid = corrected.clone();
    invalid.lens.angles[0] = 1.;
    check(!invalid.valid(), "Vertical mode accepted pitch")?;
    invalid.lens.angles[0] = 0.;
    invalid.lens.shift[0] = f64::NAN;
    check(!invalid.valid(), "Nonfinite shift accepted")?;
    Ok(())
}
pub fn fitting() -> Result<(), String> {
    let d = fixture()?;
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let mut v = View::scene(&d);
        v.camera.position = [0., 0., 5.];
        v.lens = Lens {
            projection,
            shift: [0.3, -0.2],
            near: 4.9,
            far: 5.1,
            ..Default::default()
        };
        let fitted = fit(&d, &v, &[1, 2], 0.12, &AtomicBool::new(false))?;
        check(
            fitted.lens.near < 4.9 && fitted.lens.far > fitted.lens.near,
            "Fit retained clipping interval",
        )?;
        for mesh in &d.scene.worlds {
            for point in &mesh.vertices {
                let p = fitted
                    .lens
                    .project(&fitted.camera, *point)
                    .ok_or("Fit clipped selected vertex")?;
                check(
                    (0..2).all(|i| p[i] >= 64. * 0.12 - 1e-6 && p[i] <= 64. * 0.88 + 1e-6),
                    "Fit violated requested margin",
                )?;
            }
        }
    }
    for point in [false, true] {
        let mut q = fixture()?;
        let mut s = q.scene.clone();
        for vertex in &mut s.worlds[0].vertices {
            vertex[2] = 0.;
            if point {
                *vertex = [0.; 3];
            }
        }
        q.edit_scene(s)?;
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let mut v = View::scene(&q);
            v.lens.projection = projection;
            check(
                fit(&q, &v, &[1], 0.1, &AtomicBool::new(false))?.valid(),
                "Point/plane fitting failed",
            )?;
        }
    }
    let (mut posed, asset) = crate::pose_sequence_verify::fixture()?;
    crate::pose_sequence::prepare(
        &posed,
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
    .commit(&mut posed)?;
    posed.current_frame = 5;
    let v = fit(
        &posed,
        &View::scene(&posed),
        &[1],
        0.1,
        &AtomicBool::new(false),
    )?;
    for point in &crate::pose_sequence::meshes(&posed)?[&1].vertices {
        let projected = v
            .lens
            .project(&v.camera, *point)
            .ok_or("Fit clipped posed rig vertex")?;
        check(
            projected
                .into_iter()
                .all(|x| (6.4 - 1e-6..=57.6 + 1e-6).contains(&x)),
            "Current posed bounds not fitted",
        )?;
    }
    let leveled = straighten(&View::scene(&d), world_views::target(&d))?;
    let fitted_level = fit(&d, &leveled, &[1, 2], 0.1, &AtomicBool::new(false))?;
    let anchor = fitted_level
        .lens
        .project(&fitted_level.camera, fitted_level.lens.level.unwrap().world)
        .unwrap();
    check(
        fitted_level.valid() && (0..2).all(|i| (anchor[i] - 32.).abs() < 1e-8),
        "Fit did not retain valid vertical target mode / centered anchor",
    )?;
    let v = View::scene(&d);
    check(
        fit(&d, &v, &[], 0.1, &AtomicBool::new(false)).is_err()
            && fit(&d, &v, &[999], 0.1, &AtomicBool::new(false)).is_err()
            && fit(&d, &v, &[1], f64::NAN, &AtomicBool::new(false)).is_err()
            && fit(&d, &v, &[1], 0.1, &AtomicBool::new(true)).is_err(),
        "Invalid/cancelled fitting accepted",
    )?;
    Ok(())
}
pub fn list_native(root: &Path) -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    p(&d, Edit::Enable)?.commit(&mut d)?;
    let enabled = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Camera list not one Undo")?;
    d.redo();
    check(d.snapshot() == enabled, "Camera list Redo differs")?;
    check(p(&d, Edit::Delete(1)).is_err(), "Deleted last camera")?;
    p(&d, Edit::Duplicate("Copy".into()))?.commit(&mut d)?;
    let original = View::scene(&d);
    p(&d, Edit::Switch(2))?.commit(&mut d)?;
    let mut copied = View::scene(&d);
    copied.lens.shift = [0.2, -0.1];
    copied = straighten(&copied, world_views::target(&d))?;
    p(&d, Edit::View(copied.clone()))?.commit(&mut d)?;
    p(&d, Edit::Switch(1))?.commit(&mut d)?;
    check(
        View::scene(&d) == original
            && d.scene
                .world3d
                .cameras
                .entries
                .iter()
                .find(|e| e.id == 2)
                .unwrap()
                .saved
                .as_ref()
                == Some(&copied),
        "Duplicated cameras share mutable lens state",
    )?;
    let state = d.snapshot();
    let path = root.join("cameras.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == state,
        "Native116 camera bank differs",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&115u32.to_le_bytes());
    std::fs::write(root.join("old.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "Native115 accepted named cameras",
    )?;
    let snap = d.snapshot();
    p(&d, Edit::Delete(1))?.commit(&mut d)?;
    check(
        d.scene.world3d.cameras.active == 2 && View::scene(&d) == copied,
        "Deleting active camera did not restore another camera",
    )?;
    d.undo();
    check(d.snapshot() == snap, "Camera deletion Undo lost full bank")?;
    let old = p(&d, Edit::Rename("Stale".into()))?;
    d.current_frame = 1;
    check(
        old.commit(&mut d).is_err(),
        "Old-frame camera preview committed",
    )?;
    d.current_frame = 0;
    check(
        prepare(&d, Edit::Enable, 1, &AtomicBool::new(false)).is_err()
            && prepare(&d, Edit::Enable, B, &AtomicBool::new(true)).is_err(),
        "Camera budget/cancel ignored",
    )?;
    let before = d.snapshot();
    let image = crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2;
    let flattened = crate::layer_merge::prepare(
        &d,
        crate::layer_merge::Operation::Flatten,
        Default::default(),
    )?;
    d.commit_state(flattened.state)?;
    check(
        d.scene.world3d.cameras.is_default()
            && crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2 == image,
        "Flatten retained camera bank / changed visible image",
    )?;
    d.undo();
    check(
        d.snapshot() == before,
        "Flatten Undo did not restore full camera bank",
    )?;
    let mut empty = crate::model_library_verify::blank()?;
    p(&empty, Edit::Enable)?.commit(&mut empty)?;
    check(
        empty.scene.world3d.cameras.valid(&empty.scene),
        "Empty-canvas camera list invalid",
    )?;
    Ok(())
}
pub fn clipping(root: &Path) -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let counts = d.history_counts();
    let mut p = world_views::Preferences::default();
    p.initialize(&d);
    p.camera_guides = false;
    p.edges = false;
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let mut view = View::scene(&d);
        view.camera.position = [0., 0., 5.];
        view.lens = Lens {
            projection,
            ..Default::default()
        };
        p.views.fill(view.clone());
        p.occlusion = Default::default();
        let baseline = world_views::render(&d, &p, B, &AtomicBool::new(false))?;
        let pixel = |frame: &world_views::Frame| {
            let offset = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
            frame.pixels[offset..offset + 4].to_vec()
        };
        check(
            pixel(&baseline[0])[0] > 200,
            "Cutaway fixture has no front wall",
        )?;
        p.occlusion[0] = world_views::Occlusion {
            enabled: true,
            point: [0., 0., -2.],
        };
        let clipped = world_views::render(&d, &p, B, &AtomicBool::new(false))?;
        check(
            pixel(&clipped[0])[2] > 200 && clipped[1].pixels == baseline[1].pixels,
            "Front plane did not expose back wall / changed another view",
        )?;
        p.occlusion[0].point = [0., 0., 0.];
        check(
            pixel(&world_views::render(&d, &p, B, &AtomicBool::new(false))?[0])[0] > 200,
            "Straddling object was removed instead of clipped",
        )?;
        view.camera.position = [0., 0., -8.];
        view.lens.angles = [0., 180., 0.];
        p.views.fill(view);
        p.occlusion[0].point = [0., 0., -2.];
        check(
            pixel(&world_views::render(&d, &p, B, &AtomicBool::new(false))?[0])[0] > 200,
            "Cut plane did not follow camera direction",
        )?;
        p.occlusion[0].point = [0., 0., 20000.];
        check(
            pixel(&world_views::render(&d, &p, B, &AtomicBool::new(false))?[0])[3] == 0,
            "Cut plane beyond far did not hide all geometry",
        )?;
    }
    p.save(root)?;
    let reopened = world_views::Preferences::load(root)?;
    check(
        reopened == p && d.snapshot() == before && d.history_counts() == counts,
        "Workspace cutaway changed document/history or did not reload",
    )?;
    let mut json = serde_json::to_value(&p).unwrap();
    json.as_object_mut().unwrap().remove("occlusion");
    let legacy: world_views::Preferences =
        serde_json::from_value(json).map_err(|e| e.to_string())?;
    check(
        legacy.occlusion.iter().all(|o| !o.enabled),
        "Old workspace did not default cutaway off",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("camera-library-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        projection()?;
        fitting()?;
        list_native(&dir)?;
        clipping(&dir)?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn shifted_rays_and_vertical_target_preserving_projection() {
        super::projection().unwrap();
    }
    #[test]
    fn posed_bounds_point_plane_empty_selection_and_clipping_fit() {
        super::fitting().unwrap();
    }
    #[test]
    fn named_cameras_independent_copy_delete_native116_and_undo() {
        let t = crate::test_support::TempDir::new();
        super::list_native(&t.0).unwrap();
    }
    #[test]
    fn per_view_front_cutaway_rotation_parallel_partial_and_workspace() {
        let t = crate::test_support::TempDir::new();
        super::clipping(&t.0).unwrap();
    }
}
