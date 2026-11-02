use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_library::{self as lib, Placement, Shape},
    world_physics::{self as core, *},
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn prepare(d: &Document, e: Edit) -> Result<world3d::Prepared, String> {
    core::prepare(d, e, B, &AtomicBool::new(false))
}
fn add(d: &mut Document, shape: Shape, position: [f64; 3], scale: [f64; 3]) -> Result<(), String> {
    lib::prepare(
        d,
        lib::Edit::Primitive {
            shape,
            detail: 6,
            placement: Placement {
                position,
                scale,
                fit: None,
                angles: if shape == Shape::Plane {
                    [90., 0., 0.]
                } else {
                    [0.; 3]
                },
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(d)?;
    Ok(())
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    add(&mut d, Shape::Box, [0., 5., 0.], [0.5; 3])?;
    add(&mut d, Shape::Box, [0., 1., 0.], [2., 0.5, 2.])?;
    add(&mut d, Shape::Plane, [4., 2., 0.], [2., 1., 2.])?;
    prepare(
        &d,
        Edit::Collider {
            ids: vec![2],
            enabled: true,
        },
    )?
    .commit(&mut d)?;
    let mut s = d.scene.clone();
    s.catalog.world_info.entry(2).or_default().properties.locked = true;
    d.edit_scene(s)?;
    Ok(d)
}
fn y(d: &Document) -> f64 {
    world3d::center(&d.scene.worlds[0])[1]
}
fn settle(ids: Vec<u64>, timestep: f64) -> Edit {
    Edit::Settle {
        ids,
        floor: 0.,
        gravity: 9.81,
        seconds: 2.,
        timestep,
    }
}
pub fn placement() -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    let p = prepare(
        &d,
        Edit::Floor {
            ids: vec![1],
            height: 0.,
        },
    )?;
    check(d.snapshot() == before, "Floor preview mutated source")?;
    p.commit(&mut d)?;
    check(
        (y(&d) - 0.5).abs() < 1e-8 && d.scene.worlds[1] == before.scene.worlds[1],
        "Floor snap moved locked collider / incorrect bottom",
    )?;
    d.undo();
    check(d.snapshot() == before, "Floor Undo failed")?;
    d.redo();
    let counts = d.history_counts();
    check(
        !prepare(
            &d,
            Edit::Floor {
                ids: vec![1],
                height: 0.,
            },
        )?
        .commit(&mut d)?
            && d.history_counts() == counts,
        "Noop snap added history",
    )?;
    d.undo();
    world3d::prepare(
        &d,
        world3d::Edit::Transform {
            ids: vec![1],
            options: world3d::Preferences::default(),
            value: [4., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    prepare(
        &d,
        Edit::Collider {
            ids: vec![3],
            enabled: true,
        },
    )?
    .commit(&mut d)?;
    let mut s = d.scene.clone();
    s.world3d.surfaces.get_mut(&3).unwrap().materials[0].rgba[3] = 0.;
    s.catalog
        .world_info
        .entry(3)
        .or_default()
        .properties
        .opacity = 0;
    d.edit_scene(s)?;
    prepare(
        &d,
        Edit::Surface {
            ids: vec![1],
            distance: 10.,
        },
    )?
    .commit(&mut d)?;
    check(
        (y(&d) - 2.5).abs() < 1e-7,
        "Polygon snap failed without lights / on transparent collider",
    )?;
    let before = d.snapshot();
    check(
        prepare(
            &d,
            Edit::Surface {
                ids: vec![1],
                distance: 0.0001,
            },
        )
        .is_ok()
            && d.snapshot() == before,
        "Touching surface did not retain pose",
    )?;
    prepare(
        &d,
        Edit::Collider {
            ids: vec![3],
            enabled: false,
        },
    )?
    .commit(&mut d)?;
    check(
        prepare(
            &d,
            Edit::Surface {
                ids: vec![1],
                distance: 10.,
            },
        )
        .is_err(),
        "Missing polygon below anchor accepted",
    )?;
    Ok(())
}
pub fn simulation() -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    let a = prepare(&d, settle(vec![1], 1e6))?;
    let b = prepare(&d, settle(vec![1], 1e6))?;
    check(
        a.scene() == b.scene() && d.snapshot() == before && !a.warnings.is_empty(),
        "Large timestep not deterministic / preview changed source",
    )?;
    a.commit(&mut d)?;
    check(
        (y(&d) - 2.).abs() < 1e-7 && d.scene.worlds[1] == before.scene.worlds[1],
        "Gravity tunneled through locked collider",
    )?;
    let landed = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Settle Undo failed")?;
    d.redo();
    check(d.snapshot() == landed, "Settle Redo failed")?;
    let mut s = d.scene.clone();
    for p in &mut s.worlds[0].vertices {
        p[1] -= 0.8;
    }
    s.world3d.nodes.get_mut(&1).unwrap().frame = s.world3d.nodes[&1]
        .frame
        .then(world3d::Matrix::translation([0., -0.8, 0.]));
    d.edit_scene(s)?;
    let p = prepare(&d, settle(vec![1], 1. / 120.))?;
    check(
        p.warnings.iter().any(|w| w.contains("overlaps")),
        "Initial overlap not reported",
    )?;
    p.commit(&mut d)?;
    check((y(&d) - 2.).abs() < 1e-7, "Initial overlap unresolved")?;
    let mut s = d.scene.clone();
    s.catalog
        .world_info
        .entry(1)
        .or_default()
        .properties
        .position_locked = true;
    d.edit_scene(s)?;
    let before = d.snapshot();
    check(
        prepare(&d, settle(vec![1], 1. / 120.)).is_err() && d.snapshot() == before,
        "Locked moving body changed",
    )?;
    d.undo();
    let before = d.snapshot();
    check(
        core::prepare(&d, settle(vec![1], 1. / 120.), 1, &AtomicBool::new(false)).is_err()
            && core::prepare(&d, settle(vec![1], 1. / 120.), B, &AtomicBool::new(true)).is_err()
            && d.snapshot() == before,
        "Budget / cancellation changed scene",
    )?;
    for e in [
        Edit::Floor {
            ids: vec![],
            height: 0.,
        },
        settle(vec![1], f64::NAN),
        Edit::Array {
            ids: vec![1],
            counts: [0, 1, 1],
            spacing: [0.; 3],
        },
        Edit::Array {
            ids: vec![1],
            counts: [16; 3],
            spacing: [0.; 3],
        },
        Edit::Floor {
            ids: vec![1],
            height: f64::INFINITY,
        },
    ] {
        check(prepare(&d, e).is_err(), "Invalid placement accepted")?;
    }
    let p = prepare(
        &d,
        Edit::Floor {
            ids: vec![1],
            height: 0.,
        },
    )?;
    d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = d.snapshot();
    check(
        p.commit(&mut d).is_err() && d.snapshot() == before,
        "Stale placement committed",
    )?;
    Ok(())
}
pub fn arrays() -> Result<(), String> {
    let mut d = fixture()?;
    lib::prepare(
        &d,
        lib::Edit::Duplicate {
            ids: vec![1],
            offset: [0., 1., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    world3d::prepare(
        &d,
        world3d::Edit::Parent {
            ids: vec![4],
            parent: Some(1),
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let original = d.snapshot();
    prepare(
        &d,
        Edit::Floor {
            ids: vec![1, 4],
            height: 0.,
        },
    )?
    .commit(&mut d)?;
    check(
        (world3d::center(&d.scene.worlds[0])[1] - 0.5).abs() < 1e-8
            && (world3d::center(&d.scene.worlds[3])[1] - 1.5).abs() < 1e-8,
        "Parent / child moved twice",
    )?;
    d.undo();
    check(d.snapshot() == original, "Tree snap Undo failed")?;
    prepare(
        &d,
        Edit::Collider {
            ids: vec![1],
            enabled: true,
        },
    )?
    .commit(&mut d)?;
    let before = d.snapshot();
    prepare(
        &d,
        Edit::Array {
            ids: vec![1, 4],
            counts: [2, 2, 1],
            spacing: [-3., 2., 0.],
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.worlds.len() == 10 && d.scene.worlds[..4] == before.scene.worlds,
        "Array count / original preservation failed",
    )?;
    for (root, child, offset) in [
        (5, 6, [-3., 0., 0.]),
        (7, 8, [0., 2., 0.]),
        (9, 10, [-3., 2., 0.]),
    ] {
        check(
            d.scene.world3d.nodes[&child].parent == Some(root)
                && d.scene.world3d.colliders.contains(&root)
                && d.scene.world3d.colliders.contains(&child),
            "Array lost parent / collider mapping",
        )?;
        let center = world3d::center(&d.scene.worlds[(root - 1) as usize]);
        let old = world3d::center(&before.scene.worlds[0]);
        check(
            (0..3).all(|i| (center[i] - old[i] - offset[i]).abs() < 1e-8),
            "Grid spacing compounded",
        )?;
    }
    let result = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Array not one Undo")?;
    d.redo();
    check(d.snapshot() == result, "Array Redo failed")?;
    let history = d.history_counts();
    check(
        !prepare(
            &d,
            Edit::Array {
                ids: vec![1],
                counts: [1; 3],
                spacing: [0.; 3],
            },
        )?
        .commit(&mut d)?
            && d.history_counts() == history,
        "Zero-copy array added history",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    placement()?;
    simulation()?;
    arrays()?;
    let dir = root.join("world-physics-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture()?;
        prepare(&d, settle(vec![1], 1. / 120.))?.commit(&mut d)?;
        let before = d.snapshot();
        let path = dir.join("settled.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, B)
                .map_err(|e| e.to_string())?
                .snapshot()
                == before,
            "Native111 lost collider roles / settled positions",
        )?;
        let mut b = std::fs::read(&path).map_err(|e| e.to_string())?;
        b[8..12].copy_from_slice(&110u32.to_le_bytes());
        std::fs::write(&path, b).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, B).is_err(),
            "Native110 accepted collider roles",
        )?;
        let mut s = d.scene.clone();
        s.world3d.colliders.insert(999);
        check(!s.world3d.valid(&s), "Stale collider ID accepted")?;
        let mut s = d.scene.clone();
        s.worlds.clear();
        s.catalog.worlds = Default::default();
        crate::world3d::reconcile(&mut s);
        check(
            s.world3d.colliders.is_empty(),
            "Deleting models left collider roles",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn floor_polygon_surface_and_locked_reference() {
        super::placement().unwrap();
    }
    #[test]
    fn gravity_overlap_large_step_repeatability_lock_budget_cancel_stale() {
        super::simulation().unwrap();
    }
    #[test]
    fn grid_independent_trees_parent_child_spacing_and_one_undo() {
        super::arrays().unwrap();
    }
    #[test]
    fn native111_collider_roles_positions_and_removal() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
