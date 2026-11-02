use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_library::{self as lib, Edit, Shape},
    world_views::*,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    lib::prepare(
        &d,
        Edit::Primitive {
            shape: Shape::Box,
            detail: 6,
            placement: Default::default(),
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
pub fn cameras() -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let mut p = Preferences::default();
    p.initialize(&d);
    p.count = 4;
    check(p.valid(), "Initial views invalid")?;
    let first = p.views[0].clone();
    let mut right = p.views[1].clone();
    right.camera.position[0] += 1.;
    p.replace(1, right.clone())?;
    check(
        p.views[0] == first && p.views[1] == right && d.snapshot() == before,
        "Independent view changed scene/other camera",
    )?;
    p.link(1)?;
    check(
        p.views.iter().all(|v| *v == right) && p.reference == 1,
        "Link did not adopt explicit reference",
    )?;
    let mut next = right;
    next.camera.position[1] += 1.;
    p.replace(2, next.clone())?;
    check(
        p.views.iter().all(|v| *v == next),
        "Continuous link did not propagate editing camera",
    )?;
    p.linked = false;
    let old = p.views[0].clone();
    let mut next = p.views[1].clone();
    next.direction(Direction::Left, target(&d));
    p.replace(1, next)?;
    check(p.views[0] == old, "Independent transition kept linking")?;
    for direction in Direction::ALL {
        let mut v = View::scene(&d);
        v.direction(direction, [0.; 3]);
        check(v.valid(), "Direction camera invalid")?;
        let point = v
            .lens
            .project(&v.camera, [0.; 3])
            .ok_or("Direction lost target")?;
        check(
            (point[0] - 32.).abs() < 1e-8 && (point[1] - 32.).abs() < 1e-8,
            "Direction changed target position",
        )?;
        check(
            v.lens.projection
                == if direction == Direction::Perspective {
                    crate::world3d::Projection::Perspective
                } else {
                    crate::world3d::Projection::Orthographic
                },
            "Direction projection differs",
        )?;
    }
    for split in [0.2, 0.8] {
        p.split = split;
        check(p.valid(), "Valid split boundary rejected")?;
    }
    p.reset_split();
    check(
        p.count == 2 && p.split == 0.5 && d.snapshot() == before,
        "Reset split changed scene",
    )?;
    Ok(())
}
pub fn frames_and_copy(root: &Path) -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    let mut p = Preferences::default();
    p.initialize(&d);
    p.count = 4;
    p.edges = true;
    let first = render(&d, &p, B, &AtomicBool::new(false))?;
    check(
        first.len() == 4
            && first.iter().all(|f| {
                f.width > 0 && f.height > 0 && f.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0)
            })
            && first.iter().any(|f| !f.guides.is_empty())
            && d.snapshot() == before,
        "Shared scene previews/guides/source preservation failed",
    )?;
    let mut clean = p.clone();
    clean.camera_guides = false;
    clean.edges = false;
    let hidden = render(&d, &clean, B, &AtomicBool::new(false))?;
    check(
        first
            .iter()
            .zip(&hidden)
            .all(|(a, b)| a.pixels == b.pixels && b.guides.is_empty()),
        "Guide switches changed output pixels",
    )?;
    let original_render = crate::render::Renderer::new(&d).pixel([32., 32.]);
    let view = p.views[3].clone();
    copy_to_scene(&d, &view, B)?.commit(&mut d)?;
    let copied = d.snapshot();
    check(
        d.scene.camera == Some(view.camera.clone())
            && d.scene.world3d.lens == Some(view.lens)
            && d.scene.worlds == before.scene.worlds,
        "One-time view-to-scene copy changed geometry",
    )?;
    d.undo();
    check(d.snapshot() == before, "Scene camera copy not one Undo")?;
    d.redo();
    check(d.snapshot() == copied, "Scene camera copy Redo differs")?;
    d.undo();
    let mut one = p.clone();
    one.replace(1, View::scene(&d))?;
    let once = one.views[1].clone();
    copy_to_scene(&d, &p.views[2], B)?.commit(&mut d)?;
    check(
        one.views[1] == once && !one.linked,
        "Scene-to-view one-time copy started linking",
    )?;
    d.undo();
    check(
        crate::render::Renderer::new(&d).pixel([32., 32.]) == original_render,
        "View/guides changed final document pixels",
    )?;
    let path = root.join("scene-camera.drawer");
    copy_to_scene(&d, &view, B)?.commit(&mut d)?;
    let state = d.snapshot();
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == state,
        "Native camera reopen differs",
    )?;
    d.undo();
    lib::prepare(
        &d,
        Edit::Duplicate {
            ids: vec![1],
            offset: [1., 1., 1.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let after = render(&d, &p, B, &AtomicBool::new(false))?;
    check(
        first.iter().zip(&after).all(|(a, b)| a.pixels != b.pixels),
        "Object edit did not update every view",
    )?;
    let empty = crate::model_library_verify::blank()?;
    let mut e = Preferences::default();
    e.initialize(&empty);
    check(
        render(&empty, &e, B, &AtomicBool::new(false))?
            .iter()
            .all(|f| f.pixels.iter().all(|v| *v == 0)),
        "Empty scene preview is not empty",
    )?;
    check(
        render(&d, &p, 1, &AtomicBool::new(false)).is_err()
            && render(&d, &p, B, &AtomicBool::new(true)).is_err(),
        "Preview budget/cancel ignored",
    )?;
    Ok(())
}
pub fn storage(root: &Path) -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let mut p = Preferences::default();
    p.initialize(&d);
    p.count = 4;
    p.link(2)?;
    p.save(root)?;
    check(
        Preferences::load(root)? == p,
        "View workspace restart differs",
    )?;
    let first = std::fs::read(root.join("settings/world-views.json")).map_err(|e| e.to_string())?;
    p.split = 0.8;
    p.edges = true;
    p.save(root)?;
    check(
        std::fs::read(root.join("settings/world-views.previous.json"))
            .map_err(|e| e.to_string())?
            == first,
        "Previous workspace not backed up",
    )?;
    let valid = std::fs::read(root.join("settings/world-views.json")).map_err(|e| e.to_string())?;
    p.split = f32::NAN;
    check(
        p.save(root).is_err()
            && std::fs::read(root.join("settings/world-views.json")).map_err(|e| e.to_string())?
                == valid,
        "Invalid workspace overwrote settings",
    )?;
    p.split = 0.5;
    std::fs::write(root.join("settings/world-views.json"), b"broken").map_err(|e| e.to_string())?;
    check(
        Preferences::load(root).is_err(),
        "Damaged workspace accepted",
    )?;
    p.save(root)?;
    check(
        std::fs::read(root.join("settings/world-views.damaged.json")).map_err(|e| e.to_string())?
            == b"broken"
            && d.snapshot() == before,
        "Damaged workspace/source not preserved",
    )?;
    p.views[0].lens.fov = f64::NAN;
    check(!p.valid(), "Invalid view camera accepted")?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("world-views-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        cameras()?;
        frames_and_copy(&dir)?;
        storage(&dir)?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_link_directions_and_split() {
        super::cameras().unwrap();
    }
    #[test]
    fn shared_pixels_guides_copy_undo_native_and_empty() {
        let t = crate::test_support::TempDir::new();
        super::frames_and_copy(&t.0).unwrap();
    }
    #[test]
    fn local_restart_backup_corruption_and_invalid_values() {
        let t = crate::test_support::TempDir::new();
        super::storage(&t.0).unwrap();
    }
}
