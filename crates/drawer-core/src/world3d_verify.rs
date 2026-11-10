use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    scene::{Camera, Rect, WorldMesh},
    world_math::{add, sub},
    world3d::{self as w, *},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn near(a: V, b: V) -> bool {
    a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-8)
}
fn prepare(d: &Document, e: Edit) -> Result<Prepared, String> {
    w::prepare(d, e, B, &AtomicBool::new(false))
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    prepare(&d, Edit::Reference)?.commit(&mut d)?;
    Ok(d)
}
pub fn transforms() -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let ids = (1..=3).collect::<Vec<_>>();
    prepare(
        &d,
        Edit::Transform {
            ids: vec![1, 2],
            options: Preferences::default(),
            value: [1., 2., 3.],
        },
    )?
    .commit(&mut d)?;
    for (a, b) in original.scene.worlds.iter().zip(&d.scene.worlds) {
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            check(
                near(add(*a, [1., 2., 3.]), *b),
                "Selected parent / child moved twice or descendant missing",
            )?;
        }
    }
    d.undo();
    check(d.snapshot() == original, "Parent transform Undo differs")?;
    d.redo();
    d.undo();
    prepare(
        &d,
        Edit::Parent {
            ids: ids.clone(),
            parent: None,
        },
    )?
    .commit(&mut d)?;
    // Independent reference coordinates distinguish individual and common pivots.
    let detached = d.snapshot();
    check(
        w::pivot(&d.scene, &ids, Pivot::SelectionCenter)? == [0., 0.25, 0.],
        "Selection bounds center differs",
    )?;
    prepare(
        &d,
        Edit::Transform {
            ids: ids.clone(),
            options: Preferences {
                mode: Mode::Scale,
                pivot: Pivot::SelectionCenter,
                ..Default::default()
            },
            value: [2.; 3],
        },
    )?
    .commit(&mut d)?;
    for (a, b) in detached.scene.worlds.iter().zip(&d.scene.worlds) {
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            check(
                near([a[0] * 2., 0.25 + (a[1] - 0.25) * 2., a[2] * 2.], *b),
                "Common-center scale used individual pivot",
            )?;
        }
    }
    d.undo();
    prepare(
        &d,
        Edit::Transform {
            ids: ids.clone(),
            options: Preferences {
                mode: Mode::Scale,
                ..Default::default()
            },
            value: [2.; 3],
        },
    )?
    .commit(&mut d)?;
    for (i, (a, b)) in detached
        .scene
        .worlds
        .iter()
        .zip(&d.scene.worlds)
        .enumerate()
    {
        let py = [-0.65, -0.3, 0.7][i];
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            check(
                near([a[0] * 2., py + (a[1] - py) * 2., a[2] * 2.], *b),
                "Individual pivot moved joint",
            )?;
        }
    }
    d.undo();
    for (id, angles) in [
        (1, [0., 0., 90.]),
        (2, [30., 40., 0.]),
        (3, [0., -25., 15.]),
    ] {
        prepare(
            &d,
            Edit::Transform {
                ids: vec![id],
                options: Preferences {
                    mode: Mode::Rotate,
                    step: 0.,
                    ..Default::default()
                },
                value: angles,
            },
        )?
        .commit(&mut d)?;
    }
    let before = d.snapshot();
    let p = w::node(&d.scene, 3).unwrap().frame.origin();
    prepare(
        &d,
        Edit::Transform {
            ids: vec![3, 1, 2],
            options: Preferences {
                mode: Mode::Rotate,
                pivot: Pivot::FirstSelected,
                step: 15.,
                ..Default::default()
            },
            value: [0., 0., 91.],
        },
    )?
    .commit(&mut d)?;
    for (a, b) in before.scene.worlds.iter().zip(&d.scene.worlds) {
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            check(
                near([p[0] - (a[1] - p[1]), p[1] + a[0] - p[0], a[2]], *b),
                "First-selected pivot or rotation-step90 differs",
            )?;
        }
    }
    d.undo();
    check(
        d.snapshot() == before,
        "Multi-object rotated pose Undo differs",
    )?;
    let a = d.scene.worlds[0].vertices[0];
    prepare(
        &d,
        Edit::Transform {
            ids: vec![1],
            options: Preferences {
                axes: Axes::Object,
                ..Default::default()
            },
            value: [1., 0., 0.],
        },
    )?
    .commit(&mut d)?;
    check(
        near(sub(d.scene.worlds[0].vertices[0], a), [0., 1., 0.]),
        "Rotated object X axis did not move along world Y",
    )?;
    d.undo();
    prepare(
        &d,
        Edit::Transform {
            ids: vec![1],
            options: Preferences::default(),
            value: [1., 0., 0.],
        },
    )?
    .commit(&mut d)?;
    check(
        near(sub(d.scene.worlds[0].vertices[0], a), [1., 0., 0.]),
        "Floor X axis followed object rotation",
    )?;
    d.undo();
    for (options, value) in [
        (
            Preferences {
                mode: Mode::Scale,
                pivot: Pivot::SelectionCenter,
                ..Default::default()
            },
            [2., 0.5, -1.5],
        ),
        (
            Preferences {
                mode: Mode::Rotate,
                axes: Axes::Object,
                step: 0.,
                ..Default::default()
            },
            [15., 25., 35.],
        ),
        (
            Preferences {
                mode: Mode::Scale,
                axes: Axes::Object,
                ..Default::default()
            },
            [0.6, 1.8, 0.7],
        ),
    ] {
        prepare(
            &d,
            Edit::Transform {
                ids: vec![1, 2, 3],
                options,
                value,
            },
        )?
        .commit(&mut d)?;
    }
    for n in d.scene.world3d.nodes.values() {
        let identity = n.frame.then(n.frame.inverse());
        for i in 0..3 {
            for j in 0..4 {
                let expected = if i == j { 1. } else { 0. };
                check(
                    (identity.0[i][j] - expected).abs() < 1e-8,
                    "Nonuniform affine/shear inverse lost precision",
                )?;
            }
        }
    }
    let before = d.snapshot();
    let mut prefs = Preferences {
        pivot: Pivot::FirstSelected,
        ..Default::default()
    };
    prefs.pivot = Pivot::SelectionCenter;
    let _ = w::pivot(&d.scene, &ids, prefs.pivot)?;
    check(
        d.snapshot() == before,
        "Changing only gizmo pivot changed the model",
    )?;
    check(
        prepare(
            &d,
            Edit::Transform {
                ids: vec![1],
                options: Preferences {
                    mode: Mode::Scale,
                    ..Default::default()
                },
                value: [0., 1., 1.],
            },
        )
        .is_err(),
        "Zero scale accepted",
    )?;
    check(
        prepare(
            &d,
            Edit::Parent {
                ids: vec![1],
                parent: Some(1),
            },
        )
        .is_err(),
        "Cyclic parent accepted",
    )?;
    let prepared = prepare(
        &d,
        Edit::Transform {
            ids: vec![1],
            options: Preferences::default(),
            value: [1., 0., 0.],
        },
    )?;
    d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let changed = d.snapshot();
    check(
        prepared.commit(&mut d).is_err() && d.snapshot() == changed,
        "Stale3D preview overwrote edits",
    )?;
    Ok(())
}
pub fn boundaries() -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    check(
        w::prepare(&d, Edit::Reference, 1, &AtomicBool::new(false)).is_err()
            && w::prepare(&d, Edit::Reference, B, &AtomicBool::new(true)).is_err()
            && d.snapshot() == before,
        "3D cancellation / budget changed scene",
    )?;
    check(
        prepare(
            &d,
            Edit::Transform {
                ids: vec![],
                options: Preferences::default(),
                value: [1.; 3],
            },
        )
        .is_err(),
        "Empty 3D selection accepted",
    )?;
    let mut s = d.scene.clone();
    s.catalog.world_info.entry(3).or_default().properties.locked = true;
    d.edit_scene(s)?;
    let locked = d.snapshot();
    check(
        prepare(
            &d,
            Edit::Transform {
                ids: vec![1],
                options: Preferences::default(),
                value: [1., 0., 0.],
            },
        )
        .is_err()
            && d.snapshot() == locked,
        "Locked descendant moved",
    )?;
    d.undo();
    let mut s = d.snapshot();
    s.scene.catalog.worlds.remove(0, s.scene.worlds.len())?;
    s.scene.worlds.remove(0);
    d.commit_state(s)?;
    check(
        d.scene.world3d.nodes[&2].parent.is_none() && d.scene.worlds[0] == before.scene.worlds[1],
        "Deleting parent changed surviving child world pose",
    )?;
    d.undo();
    check(d.snapshot() == before, "Delete-parent Undo lost relation")?;
    // Visibility / position locks restrict movement, while the layer deletion
    // policy allows removal unless the whole layer is locked.
    let mut hidden = d.scene.clone();
    let flags = &mut hidden.catalog.world_info.entry(3).or_default().properties;
    flags.hidden = true;
    flags.position_locked = true;
    d.edit_scene(hidden)?;
    let mut removed = d.snapshot();
    removed
        .scene
        .catalog
        .worlds
        .remove(2, removed.scene.worlds.len())?;
    removed.scene.worlds.remove(2);
    d.commit_state(removed)?;
    check(
        d.scene.worlds.len() == 2,
        "3D metadata changed hidden-layer deletion policy",
    )?;
    d.undo();
    d.undo();
    let mut s = d.scene.clone();
    for _ in 3..17 {
        s.worlds.push(s.worlds[0].clone());
    }
    s.catalog.worlds.append(17)?;
    s.world3d.nodes.clear();
    for id in 1..=17 {
        s.world3d.nodes.insert(
            id,
            Node {
                frame: Matrix::default(),
                parent: (id > 1).then_some(id - 1),
            },
        );
    }
    check(s.world3d.valid(&s), "Sixteen parent links rejected")?;
    s.worlds.push(s.worlds[0].clone());
    s.catalog.worlds.append(18)?;
    s.world3d.nodes.insert(
        18,
        Node {
            frame: Matrix::default(),
            parent: Some(17),
        },
    );
    check(!s.world3d.valid(&s), "Parent depth17 accepted")?;
    let mut probe = fixture()?;
    probe.commit_state(crate::timeline_editor::create(
        &probe,
        10,
        Default::default(),
    )?)?;
    probe.commit_state(crate::camera2d::enable(&probe, true)?)?;
    let mut camera = probe.scene.camera.clone().unwrap();
    camera.position[0] += 1.;
    let lens = probe.scene.world3d.lens.unwrap();
    let protected = probe.snapshot();
    check(
        prepare(
            &probe,
            Edit::Camera {
                camera: camera.clone(),
                lens,
            },
        )
        .is_err()
            && probe.snapshot() == protected,
        "3D camera bypassed2D filming source guard",
    )?;
    crate::keyframes::edit_content(
        &mut probe,
        Some(crate::keyframes::Owner::Layer(
            crate::layer_search::Key::World(1),
        )),
    )?;
    prepare(&probe, Edit::Camera { camera, lens })?.commit(&mut probe)?;
    let mut blank = Document::create(
        CanvasSpec {
            width: 128,
            height: 96,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let mut scene = blank.scene.clone();
    scene.transform = crate::scene::Affine([2., 0., 0., 2., -30., -10.]);
    blank.edit_scene(scene)?;
    prepare(&blank, Edit::Reference)?.commit(&mut blank)?;
    let camera = blank.scene.camera.as_ref().unwrap();
    let center = [
        (camera.page_bounds.min[0] + camera.page_bounds.max[0]) / 2.,
        (camera.page_bounds.min[1] + camera.page_bounds.max[1]) / 2.,
    ];
    check(
        blank.scene.transform.map(center) == [64., 48.],
        "New camera did not respect preserved resized/cropped page coordinates",
    )?;

    Ok(())
}
pub fn layer_interop() -> Result<(), String> {
    let mut d = fixture()?;
    let c = d.scene.camera.clone().unwrap();
    let lens = Lens {
        angles: [10., 20., 0.],
        ..d.scene.world3d.lens.unwrap()
    };
    prepare(
        &d,
        Edit::Camera {
            camera: c.clone(),
            lens,
        },
    )?
    .commit(&mut d)?;
    let from = lens
        .project(&c, w::node(&d.scene, 2).unwrap().frame.origin())
        .unwrap();
    let moved = crate::search_move::prepare(&d, crate::layer_search::Key::World(2), [7, -5])?;
    d.commit_state(moved)?;
    let to = lens
        .project(&c, w::node(&d.scene, 2).unwrap().frame.origin())
        .unwrap();
    check(
        (to[0] - from[0] - 7.).abs() < 1e-8 && (to[1] - from[1] + 5.).abs() < 1e-8,
        "Perspective layer move lost camera plane / joint pivot",
    )?;
    let old = d.snapshot();
    let copied = crate::layer_move::copy_sources(
        &d,
        &[
            crate::layer_search::Key::World(1),
            crate::layer_search::Key::World(2),
            crate::layer_search::Key::World(3),
        ],
    )?;
    d.commit_state(copied.0)?;
    for (old, new) in [(1, 4), (2, 5), (3, 6)] {
        check(
            d.scene.world3d.nodes[&new].frame == d.scene.world3d.nodes[&old].frame,
            "Layer copy lost object axes / pivot",
        )?;
    }
    check(
        d.scene.world3d.nodes[&5].parent == Some(4) && d.scene.world3d.nodes[&6].parent == Some(5),
        "Layer copy kept original parent links",
    )?;
    d.undo();
    check(d.snapshot() == old, "Copied3D hierarchy Undo failed")?;
    Ok(())
}
pub fn projection() -> Result<(), String> {
    let c = Camera {
        page_bounds: Rect {
            min: [0., 0.],
            max: [64., 64.],
        },
        position: [0.; 3],
        pixels_per_unit: 8.,
    };
    let l = Lens {
        fov: 90.,
        near: 1.,
        far: 4.,
        ..Default::default()
    };
    let p = l.project(&c, [1., 0., -2.]).unwrap();
    check(
        (p[0] - 48.).abs() < 1e-8 && p[1] == 32.,
        "Perspective focal projection differs",
    )?;
    let ortho = Lens {
        projection: Projection::Orthographic,
        ..l
    };
    check(
        ortho.project(&c, [1., 0., -2.]) == Some([40., 32.]),
        "Orthographic projection differs",
    )?;
    check(
        l.project(&c, [0., 0., 0.]).is_none()
            && l.project(&c, [0., 0., -5.]).is_none()
            && l.segment(&c, [0., 0., 1.], [1., 0., 2.]).is_none(),
        "Behind/near/far depth leaked",
    )?;
    let line = l.segment(&c, [1., 0., 0.], [1., 0., -8.]).unwrap();
    check(
        (line[0][0] - 64.).abs() < 1e-8 && (line[1][0] - 40.).abs() < 1e-8,
        "Crossing edge dropped instead of depth-clipped",
    )?;
    let rotated = Lens {
        angles: [0., 90., 0.],
        ..l
    };
    check(
        rotated.project(&c, [-2., 0., 0.]).is_some(),
        "Camera orientation did not rotate view",
    )?;
    for lens in [
        Lens { near: 0., ..l },
        Lens { far: 1., ..l },
        Lens { fov: 180., ..l },
    ] {
        check(!lens.valid(), "Invalid depth / lens accepted")?;
    }
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    transforms()?;
    boundaries()?;
    projection()?;
    layer_interop()?;
    let dir = root.join("world3d-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture()?;
        prepare(
            &d,
            Edit::Transform {
                ids: vec![2],
                options: Preferences {
                    mode: Mode::Rotate,
                    ..Default::default()
                },
                value: [0., 0., 45.],
            },
        )?
        .commit(&mut d)?;
        let snapshot = d.snapshot();
        let path = dir.join("pose.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let loaded = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            loaded.snapshot() == snapshot,
            "Articulated pose / camera changed after107 reopen",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        crate::save_generation::verify::legacy_version(&mut bytes, 106);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, B).is_err(),
            "Version106 accepted world frame / lens",
        )?;
        let prefs = Preferences {
            axes: Axes::Object,
            pivot: Pivot::FirstSelected,
            mode: Mode::Rotate,
            step: 7.5,
            ..Default::default()
        };
        prefs.save(&dir)?;
        check(
            Preferences::load(&dir)? == prefs,
            "Gizmo defaults changed on restart",
        )?;
        let mut d = Document::create(
            CanvasSpec {
                width: 64,
                height: 64,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let mut s = d.scene.clone();
        s.worlds.push(WorldMesh {
            vertices: vec![[-1., 0., -2.], [1., 0., -2.]],
            edges: vec![[0, 1]],
            color: [255, 0, 0, 255],
        });
        s.camera = Some(Camera {
            page_bounds: Rect {
                min: [0., 0.],
                max: [64., 64.],
            },
            position: [0.; 3],
            pixels_per_unit: 8.,
        });
        s.world3d.lens = Some(Lens {
            near: 1.,
            far: 4.,
            fov: 90.,
            ..Default::default()
        });
        d.edit_scene(s)?;
        check(
            crate::render::Renderer::new(&d).pixel([32.5, 32.5]) == [255, 0, 0, 255]
                && crate::render::Renderer::new(&d).pixel([32.5, 30.5]) == [0; 4],
            "3D camera did not reach document renderer",
        )?;
        let png = dir.join("camera.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        let pixels = image::open(&png).map_err(|e| e.to_string())?.to_rgba8();
        check(
            pixels.get_pixel(32, 32).0 == [255, 0, 0, 255] && pixels.get_pixel(32, 30).0 == [0; 4],
            "Actual PNG differs from 3D renderer",
        )?;
        let old = d.snapshot();
        let c = d.scene.camera.clone().unwrap();
        prepare(
            &d,
            Edit::Camera {
                camera: c,
                lens: Lens {
                    near: 3.,
                    far: 4.,
                    ..d.scene.world3d.lens.unwrap()
                },
            },
        )?
        .commit(&mut d)?;
        check(
            crate::render::Renderer::new(&d).pixel([32.5, 32.5]) == [0; 4],
            "Clipped model leaked into output",
        )?;
        d.undo();
        check(d.snapshot() == old, "Camera depth Undo failed")?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
mod tests {
    #[test]
    fn existing_layer_move_and_copy_preserve_camera_plane_and_hierarchy() {
        super::layer_interop().unwrap();
    }

    #[test]
    fn multi_axes_pivots_nonuniform_affines_pose_and_stale() {
        super::transforms().unwrap();
    }
    #[test]
    fn parents_locks_zero_scale_empty_cancel_and_depth_limits() {
        super::boundaries().unwrap();
    }
    #[test]
    fn perspective_orthographic_rotated_camera_and_near_far_crossing_edges() {
        super::projection().unwrap();
    }
    #[test]
    fn native107_png_camera_undo_and_local_defaults() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
