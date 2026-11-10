use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_library::{self as lib, *},
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn prep(d: &Document, e: Edit) -> Result<world3d::Prepared, String> {
    lib::prepare(d, e, B, &AtomicBool::new(false))
}
pub fn blank() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())
}
pub fn write_fixture(root: &Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(root.join("materials")).map_err(|e| e.to_string())?;
    let obj = "mtllib ./materials/test.mtl\nv 0 0 0\nv 3 0 0\nv 3 3 0\nv 1 1 0\nv 0 3 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0.3 0.3\nvt 0 1\nvn 0 0 1\no Panel\nusemtl Red\ns 1\nf -5/1/1 -4/2/1 -3/3/1 -2/4/1 -1/5/1\no Wire\nl 1 3\n";
    let path = root.join("local.obj");
    std::fs::write(&path, obj).map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("materials/test.mtl"),
        "newmtl Red\nKd 1 0 0\nd 0.5\nmap_Kd ../textures/missing.png\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn primitives() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    for shape in Shape::ALL {
        for detail in [3, 64] {
            let a = lib::primitive(shape, detail, &cancel)?;
            check(
                a.valid() && !a.models[0].surface.triangles.is_empty(),
                "Primitive mesh / topology invalid",
            )?;
        }
    }
    let mut d = blank()?;
    let tight = d.memory_bytes() * 3 + 65536;
    check(
        lib::prepare(
            &d,
            Edit::Primitive {
                shape: Shape::Sphere,
                detail: 64,
                placement: Default::default(),
            },
            tight,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Large primitive ignored remaining geometry budget",
    )?;
    check(
        lib::prepare(
            &d,
            Edit::Primitive {
                shape: Shape::Box,
                detail: 12,
                placement: Default::default(),
            },
            tight,
            &AtomicBool::new(false),
        )
        .is_ok(),
        "Small primitive rejected despite fitting remaining budget",
    )?;
    let before = d.snapshot();
    let p = Placement {
        position: [3., 4., 0.],
        scale: [-1., 2., 0.5],
        ..Default::default()
    };
    let prepared = prep(
        &d,
        Edit::Primitive {
            shape: Shape::Box,
            detail: 12,
            placement: p,
        },
    )?;
    check(
        d.snapshot() == before && prepared.scene().worlds.len() == 1,
        "Primitive preview changed original",
    )?;
    prepared.commit(&mut d)?;
    check(
        world3d::node(&d.scene, 1).unwrap().frame.origin() == [3., 4., 0.]
            && d.scene.world3d.nodes[&1].frame.determinant() < 0.,
        "Placement center / negative scale lost",
    )?;
    let state = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Primitive Undo failed")?;
    d.redo();
    check(d.snapshot() == state, "Primitive Redo failed")?;
    let counts = d.history_counts();
    let same = prep(
        &d,
        Edit::Align {
            ids: vec![1],
            axis: 0,
            anchor: Anchor::Center,
            target: AlignTarget::First,
        },
    )?
    .commit(&mut d)?;
    check(
        !same && d.history_counts() == counts,
        "No-op alignment created Undo",
    )?;
    check(
        prep(
            &d,
            Edit::Primitive {
                shape: Shape::Box,
                detail: 12,
                placement: Placement {
                    scale: [0., 1., 1.],
                    ..Default::default()
                },
            },
        )
        .is_err(),
        "Zero placement scale accepted",
    )?;
    Ok(())
}
pub fn edit_trees() -> Result<(), String> {
    let mut d = crate::world3d_verify::fixture()?;
    prep(
        &d,
        Edit::Primitive {
            shape: Shape::Box,
            detail: 12,
            placement: Placement {
                position: [4., 0., 0.],
                ..Default::default()
            },
        },
    )?
    .commit(&mut d)?;
    let old = d.snapshot();
    prep(
        &d,
        Edit::Align {
            ids: vec![2, 4, 1],
            axis: 0,
            anchor: Anchor::Center,
            target: AlignTarget::First,
        },
    )?
    .commit(&mut d)?;
    check(
        world3d::center(&d.scene.worlds[3])[0].abs() < 1e-9
            && d.scene.worlds[..3] == old.scene.worlds[..3],
        "Alignment lost first selected tree / moved its children twice",
    )?;
    d.undo();
    check(d.snapshot() == old, "Alignment Undo failed")?;
    prep(
        &d,
        Edit::Duplicate {
            ids: vec![1, 2],
            offset: [2., 0., 0.],
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.worlds.len() == 7
            && d.scene.world3d.nodes[&6].parent == Some(5)
            && d.scene.world3d.nodes[&7].parent == Some(6),
        "Tree duplication count / parent remapping differs",
    )?;
    for i in 0..3 {
        for (a, b) in old.scene.worlds[i]
            .vertices
            .iter()
            .zip(&d.scene.worlds[i + 4].vertices)
        {
            check(
                (b[0] - a[0] - 2.).abs() < 1e-9 && b[1] == a[1] && b[2] == a[2],
                "Duplicate child received offset more than once",
            )?;
        }
    }
    prep(
        &d,
        Edit::Duplicate {
            ids: vec![4],
            offset: [0., 2., 0.],
        },
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.surfaces[&4] == d.scene.world3d.surfaces[&8],
        "Copied surface attributes differ",
    )?;
    let original = d.scene.world3d.surfaces[&4].clone();
    let mut s = d.scene.clone();
    s.world3d.surfaces.get_mut(&8).unwrap().materials[0].rgba = [0., 1., 0., 1.];
    d.edit_scene(s)?;
    check(
        d.scene.world3d.surfaces[&4] == original,
        "Copied surface aliases original",
    )?;
    let mut s = d.scene.clone();
    s.catalog
        .world_info
        .entry(3)
        .or_default()
        .properties
        .position_locked = true;
    d.edit_scene(s)?;
    let locked = d.snapshot();
    for e in [
        Edit::Duplicate {
            ids: vec![1],
            offset: [1., 0., 0.],
        },
        Edit::Align {
            ids: vec![1, 4],
            axis: 0,
            anchor: Anchor::Max,
            target: AlignTarget::Selection,
        },
    ] {
        check(
            prep(&d, e).is_err() && d.snapshot() == locked,
            "Locked child changed during model tree edit",
        )?;
    }
    check(
        prep(
            &d,
            Edit::Align {
                ids: vec![],
                axis: 0,
                anchor: Anchor::Min,
                target: AlignTarget::First,
            },
        )
        .is_err()
            && prep(
                &d,
                Edit::Align {
                    ids: vec![4],
                    axis: 3,
                    anchor: Anchor::Min,
                    target: AlignTarget::First,
                },
            )
            .is_err(),
        "Empty / invalid alignment accepted",
    )?;
    Ok(())
}
pub fn parsing() -> Result<(), String> {
    let c = AtomicBool::new(false);
    let simple = "v 0 0 0\nv 2 0 0\nv 2 1 0\nv 0 1 0\nf 1 2 3 4\n";
    let a = crate::model_obj::decode(simple.as_bytes(), "test.obj", &c)?.asset;
    check(
        a.models[0].surface.triangles.len() == 2 && a.models[0].mesh.edges.len() == 4,
        "Quad triangulation lost perimeter / added diagonal wire",
    )?;
    for text in [
        "v NaN 0 0\n",
        "v 0 0 0 0\n",
        "v 0 0 0\nf 0 1 1\n",
        "v 0 0 0\nf 1 2 3\n",
        "csh command\n",
        "call other.obj\n",
        "mtllib ../outside.mtl\n",
        "mtllib https://example.invalid/a.mtl\n",
        "vn 0 0 0\n",
        "v 0 0 0\\",
        "v 0 0 0\nv 1 1 0\nv 0 1 0\nv 1 0 0\nf 1 2 3 4\n",
        "v 0 0 0\nv 1 0 0\nv 1 1 1\nv 0 1 0\nf 1 2 3 4\n",
    ] {
        check(
            crate::model_obj::decode(text.as_bytes(), "bad.obj", &c).is_err(),
            "Malformed / unsafe OBJ accepted",
        )?;
    }
    check(
        crate::model_obj::decode(simple.as_bytes(), "test.obj", &AtomicBool::new(true)).is_err(),
        "Cancelled OBJ parsed",
    )?;
    let large = "v 0 0 0\n".repeat(65537);
    check(
        crate::model_obj::decode(large.as_bytes(), "large.obj", &c).is_err(),
        "Excessive mesh allocated past vertex limit",
    )?;
    check(
        crate::model_obj::decode(&[255], "bad.obj", &c).is_err()
            && crate::model_obj::decode(&vec![b' '; 8 * 1024 * 1024 + 1], "huge.obj", &c).is_err(),
        "Invalid text / size accepted",
    )?;
    Ok(())
}
pub fn wire_queries() -> Result<(), String> {
    let mut d = blank()?;
    prep(
        &d,
        Edit::Primitive {
            shape: Shape::Sphere,
            detail: 12,
            placement: Default::default(),
        },
    )?
    .commit(&mut d)?;
    for projection in [
        world3d::Projection::Perspective,
        world3d::Projection::Orthographic,
    ] {
        let mut scene = d.scene.clone();
        scene.world3d.lens.as_mut().unwrap().projection = projection;
        d.edit_scene(scene)?;
        let lines = world3d::segments(&d.scene, 0);
        let renderer = crate::render::Renderer::new(&d);
        for y in (0..64).step_by(2) {
            for x in (0..64).step_by(2) {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let hit = lines.iter().any(|l| {
                    let v = [l[1][0] - l[0][0], l[1][1] - l[0][1]];
                    let length = v[0] * v[0] + v[1] * v[1];
                    let t = if length == 0. {
                        0.
                    } else {
                        (((p[0] - l[0][0]) * v[0] + (p[1] - l[0][1]) * v[1]) / length).clamp(0., 1.)
                    };
                    let q = [l[0][0] + t * v[0], l[0][1] + t * v[1]];
                    (p[0] - q[0]).hypot(p[1] - q[1]) <= 0.5
                });
                let expected = if hit { d.scene.worlds[0].color } else { [0; 4] };
                check(
                    renderer.pixel(p) == expected,
                    "Spatial wire query changed projected line coverage",
                )?;
            }
        }
    }
    Ok(())
}
fn source_pixels(d: &Document) -> Vec<u8> {
    let r = crate::render::Renderer::new(d);
    let mut p = vec![];
    for y in 0..64 {
        for x in 0..64 {
            p.extend(r.pixel([x as f64 + 0.5, y as f64 + 0.5]));
        }
    }
    p
}
pub fn verify(root: &Path) -> Result<(), String> {
    primitives()?;
    edit_trees()?;
    parsing()?;
    wire_queries()?;
    let dir = root.join("models-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = write_fixture(&dir)?;
        let original = std::fs::read(&path).map_err(|e| e.to_string())?;
        let material = std::fs::read(dir.join("materials/test.mtl")).map_err(|e| e.to_string())?;
        let a = crate::model_obj::load(&path, B, &AtomicBool::new(false))?;
        check(
            a.models.len() == 2
                && a.models[0].surface.triangles.len() == 3
                && a.models[0].surface.texcoords.len() == 5
                && a.models[0].surface.normals.len() == 1
                && a.models[0].mesh.color == [255, 0, 0, 128],
            "OBJ polygon / UV / normal / material snapshot differs",
        )?;
        check(
            a.warnings.iter().any(|w| w.contains("Missing texture"))
                && a.models[0].surface.materials[1].texture.as_deref()
                    == Some("textures/missing.png"),
            "Missing sidecar texture / safe relative path not reported",
        )?;
        let mut d = blank()?;
        let before = d.snapshot();
        let p = prep(
            &d,
            Edit::Import {
                path: path.clone(),
                placement: Default::default(),
            },
        )?;
        check(
            !p.warnings.is_empty() && d.snapshot() == before,
            "Import warnings / unchanged original missing",
        )?;
        p.commit(&mut d)?;
        check(
            source_pixels(&d)
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0),
            "Imported OBJ did not reach renderer",
        )?;
        let png = dir.join("model.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        check(
            image::open(&png)
                .map_err(|e| e.to_string())?
                .to_rgba8()
                .into_raw()
                == source_pixels(&d),
            "Actual model PNG differs from preview pixels",
        )?;
        let snapshot = d.snapshot();
        let native = dir.join("model.drawer");
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, B)
                .map_err(|e| e.to_string())?
                .snapshot()
                == snapshot,
            "Topology / UV / normal / material did not survive native108",
        )?;
        let mut b = std::fs::read(&native).map_err(|e| e.to_string())?;
        crate::save_generation::verify::legacy_version(&mut b, 107);
        std::fs::write(&native, b).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, B).is_err(),
            "Native107 accepted new topology",
        )?;
        check(
            std::fs::read(&path).map_err(|e| e.to_string())? == original
                && std::fs::read(dir.join("materials/test.mtl")).map_err(|e| e.to_string())?
                    == material,
            "Model / material source was modified",
        )?;
        let mut hidden = blank()?;
        prep(
            &hidden,
            Edit::Primitive {
                shape: Shape::Chair,
                detail: 12,
                placement: Placement {
                    hidden: true,
                    ..Default::default()
                },
            },
        )?
        .commit(&mut hidden)?;
        check(
            source_pixels(&hidden).iter().all(|n| *n == 0),
            "Hidden model leaked into output",
        )?;
        let mut s = hidden.scene.clone();
        s.catalog.world_info.get_mut(&1).unwrap().properties.hidden = false;
        hidden.edit_scene(s)?;
        check(
            source_pixels(&hidden)
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0),
            "Revealed model stayed invisible",
        )?;
        let mut large = blank()?;
        prep(
            &large,
            Edit::Primitive {
                shape: Shape::Sphere,
                detail: 64,
                placement: Default::default(),
            },
        )?
        .commit(&mut large)?;
        let high = dir.join("large.drawer");
        crate::files::save(&mut large, &high).map_err(|e| e.to_string())?;
        let reopened = crate::files::open(&high, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == large.snapshot(),
            "Maximum-detail sphere lost native geometry / normals",
        )?;
        let before = d.snapshot();
        check(
            lib::prepare(
                &d,
                Edit::Import {
                    path: path.clone(),
                    placement: Default::default(),
                },
                1,
                &AtomicBool::new(false),
            )
            .is_err()
                && lib::prepare(
                    &d,
                    Edit::Import {
                        path: path.clone(),
                        placement: Default::default(),
                    },
                    B,
                    &AtomicBool::new(true),
                )
                .is_err()
                && d.snapshot() == before,
            "Budget / cancellation changed document",
        )?;
        let p = prep(
            &d,
            Edit::Primitive {
                shape: Shape::Plane,
                detail: 3,
                placement: Default::default(),
            },
        )?;
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let changed = d.snapshot();
        check(
            p.commit(&mut d).is_err() && d.snapshot() == changed,
            "Stale model preview replaced drawing",
        )?;
        std::fs::write(
            dir.join("materials/test.mtl"),
            "newmtl Red\nmap_Kd ../../outside.png\n",
        )
        .map_err(|e| e.to_string())?;
        check(
            crate::model_obj::load(&path, B, &AtomicBool::new(false)).is_err(),
            "Texture escaped model folder",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
#[cfg(test)]
mod tests {
    #[test]
    fn spatial_wire_queries_match_naive_perspective_and_orthographic() {
        super::wire_queries().unwrap();
    }

    #[test]
    fn original_catalog_min_max_placement_mirror_and_noop() {
        super::primitives().unwrap();
    }
    #[test]
    fn trees_duplicate_independent_surfaces_alignment_order_and_locks() {
        super::edit_trees().unwrap();
    }
    #[test]
    fn obj_quads_corrupt_polygons_indices_unsafe_paths_limits_and_cancel() {
        super::parsing().unwrap();
    }
    #[test]
    fn local_obj_missing_texture_native108_hidden_output_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
