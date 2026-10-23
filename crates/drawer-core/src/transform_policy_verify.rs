//! Independent coordinate/width, depth overlap, immutable-source and native checks.
use crate::{
    affine_transform,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    interpolation::Filter,
    layer_properties::Target,
    layer_search::Key,
    mesh_deform::{Mesh, Pin},
    mesh_edit::{self, Settings},
    scene::{Point, Rect},
    vector_scope::Transform,
};
fn check(ok: bool, m: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(m.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() < 1e-8)
}
pub fn width_pivots(root: &std::path::Path) -> Result<(), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.scene.worlds.push(crate::scene::WorldMesh {
        vertices: vec![[0., 0., 0.], [1., 1., 1.]],
        edges: vec![[0, 1]],
        color: [1, 2, 3, 255],
    });
    d.scene.camera = Some(crate::scene::Camera {
        page_bounds: Rect {
            min: [0.; 2],
            max: [64., 48.],
        },
        position: [0., 0., -2.],
        pixels_per_unit: 10.,
    });
    let mut keys = vec![];
    for (n, x) in [8., 40.].into_iter().enumerate() {
        let id = d.scene.lines.add_layer(&format!("Path {n}"))?;
        d.scene.lines.add_path(
            id,
            vec![[x, 12.], [x + 8., 12.], [x + 8., 20.], [x, 20.]],
            true,
            10.,
            [120, 50, 30, 255],
        )?;
        keys.push(Key::Layer(Target::Vector(id)));
    }
    crate::layer_groups_verify::save(&mut d, keys.clone())?;
    let before = d.snapshot();
    let counts = d.history_counts();
    for preserve_width in [false, true] {
        for scale in [[2., 0.5], [-2., 0.5], [2., 2.], [-2., 2.]] {
            let t = Transform {
                scale,
                preserve_width,
                pivot: [28., 16.],
                ..Default::default()
            };
            let out =
                affine_transform::prepare(&d, keys[0], false, t, false, Filter::Nearest)?.state;
            let width = if preserve_width {
                10.
            } else {
                10. * (scale[0] * scale[1]).abs().sqrt()
            };
            check(
                out.scene
                    .lines
                    .layers
                    .iter()
                    .all(|l| (l.paths[0].width - width).abs() < 1e-9),
                "Geometric mean or preserve-width policy",
            )?;
            check(
                out.scene.worlds == before.scene.worlds && out.scene.camera == before.scene.camera,
                "Object transform changed world/camera",
            )?;
        }
    }
    for individual in [false, true] {
        let out = affine_transform::prepare(
            &d,
            keys[0],
            false,
            Transform {
                pivot: [28., 16.],
                degrees: 90.,
                translation: [3., -2.],
                individual,
                ..Default::default()
            },
            false,
            Filter::Nearest,
        )?
        .state;
        for (n, l) in out.scene.lines.layers.iter().enumerate() {
            let actual = l.paths[0].points[0];
            let expected = if individual {
                [19. + 32. * n as f64, 10.]
            } else {
                [35., -6. + 32. * n as f64]
            };
            check(
                near(actual, expected),
                "Group versus individual path center coordinates",
            )?;
        }
    }
    let out = affine_transform::prepare(
        &d,
        keys[0],
        false,
        Transform {
            degrees: 37.,
            pivot: [28., 16.],
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?
    .state;
    check(
        d.snapshot() == before && d.history_counts() == counts,
        "Pivot preview mutated source",
    )?;
    d.commit_state(out)?;
    let (r, axes) = crate::transform_policy::oriented(&d, keys[0])?;
    check(
        (axes - 37.).abs() < 1e-9,
        "Rotated bounds lost object orientation",
    )?;
    let t = Transform {
        axes,
        pivot: [12., 16.],
        ..Default::default()
    };
    let q = crate::transform_handles::box_corners(r, t)?;
    check(
        ((q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]).to_degrees() - 37.).abs() < 1e-9,
        "Box does not follow object axes",
    )?;
    let source = Transform {
        axes,
        pivot: [12., 16.],
        scale: [-1., 2.],
        degrees: 20.,
        ..Default::default()
    };
    let q = crate::transform_handles::box_corners(r, source)?;
    let end = [q[2][0] + 2., q[2][1] + 4.];
    let moved = crate::transform_handles::Drag {
        handle: crate::transform_handles::Handle::Corner(2),
        bounds: r,
        source,
        start: q[2],
    }
    .update(end, false, Default::default())?;
    let result = crate::transform_handles::box_corners(r, moved)?;
    check(
        near(result[0], q[0]) && near(result[2], end),
        "Oriented negative-scale handle lost opposite anchor",
    )?;
    let rotated = crate::sizing::prepare(
        &d.snapshot(),
        crate::sizing::Change::Rotate90 { clockwise: true },
        DEFAULT_DOCUMENT_BUDGET,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    let rotated = crate::layer_merge::preview(&rotated, 0)?;
    check(
        (crate::transform_policy::oriented(&rotated, keys[0])?.1 - 127.).abs() < 1e-9,
        "Document rotation lost the stored object axes",
    )?;
    let file = root.join("transform-policy.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && (crate::transform_policy::oriented(&reopened, keys[0])?.1 - 37.).abs() < 1e-9
            && d.undo()
            && d.snapshot() == before,
        "Oriented native/one Undo",
    )?;
    Ok(())
}
pub fn depth_and_selection() -> Result<(), String> {
    let area = Rect {
        min: [0.; 2],
        max: [16.; 2],
    };
    let mut mesh = Mesh::new(3, 3)?;
    let old = mesh.clone();
    mesh.move_selected(&[0, 1], [0.05, 0.02])?;
    check(
        mesh.points[0] == [0.05, 0.02]
            && mesh.points[1] == [0.55, 0.02]
            && (2..9).all(|n| mesh.points[n] == old.points[n]),
        "Group mesh move changed unselected vertices",
    )?;
    let snapshot = mesh.clone();
    check(
        mesh.move_selected(&[0, 0], [0.1, 0.]).is_err() && mesh == snapshot,
        "Invalid selection partially moved mesh",
    )?;
    let mut pin = Pin {
        vertex: 4,
        target: [0.5; 2],
        degrees: 90.,
        ..Default::default()
    };
    let rotated = Mesh::puppet(3, 3, &[pin], area)?;
    pin.keep_angle = true;
    let kept = Mesh::puppet(3, 3, &[pin], area)?;
    check(
        near(rotated.points[0], [1., 0.]) && kept.identity(),
        "Keep reference angle versus rotate with pin",
    )?;
    pin.reference_degrees = 45.;
    let kept = Mesh::puppet(3, 3, &[pin], area)?;
    pin.degrees = 180.;
    check(
        Mesh::puppet(3, 3, &[pin], area)?.points == kept.points,
        "Reference angle drifted with current rotation",
    )?;
    // Two cells cover exactly the same square; their source x coordinates differ.
    let mut overlap = Mesh::new(3, 2)?;
    overlap.points = vec![[0., 0.], [1., 0.], [0., 0.], [0., 1.], [1., 1.], [0., 1.]];
    overlap.order = vec![[0, 0], [0, 0], [0, 1], [0, 1]];
    check(
        overlap.valid() && near(overlap.inverse([0.25, 0.5]).unwrap(), [0.875, 0.5]),
        "Equal-depth creation order does not select later pin region",
    )?;
    overlap.order[0][0] = 1;
    overlap.order[1][0] = 1;
    check(
        near(overlap.inverse([0.25, 0.5]).unwrap(), [0.125, 0.5]),
        "Pin depth does not override creation order",
    )?;
    // The real pin solver assigns those orders from immutable source triangle centers.
    let pins = [
        Pin {
            vertex: 0,
            target: [0.8, 0.1],
            depth: 7,
            ..Default::default()
        },
        Pin {
            vertex: 2,
            target: [0.2, 0.1],
            depth: -3,
            ..Default::default()
        },
    ];
    let posed = Mesh::puppet(3, 3, &pins, area)?;
    check(
        posed.order.contains(&[7, 0]) && posed.order.contains(&[-3, 1]),
        "Solver discarded per-pin depth/creation ownership",
    )?;
    let mut d = crate::deform_verify::fixture()?;
    for y in 0..16 {
        for x in 0..16 {
            d.layers[0].pixels[y * 16 + x] = if x < 8 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
        }
    }
    let targets = [[0., 0.], [1., 0.], [0., 0.], [0., 1.], [1., 1.], [0., 1.]];
    let mut pins: Vec<_> = [0, 3, 1, 4, 2, 5]
        .into_iter()
        .map(|vertex| Pin {
            vertex,
            target: targets[vertex],
            ..Default::default()
        })
        .collect();
    for front in [false, true] {
        if front {
            for pin in &mut pins {
                if pin.vertex == 3 {
                    pin.depth = 5;
                }
            }
        }
        let mesh = Mesh::puppet(3, 2, &pins, area)?;
        check(
            mesh.valid() && mesh.points == targets,
            "Actual overlapping pin solver geometry",
        )?;
        let out = crate::deform::prepare(&d, &[0], false, crate::deform::Change::Mesh(&mesh))?;
        check(
            out.layers[0].pixel(16, 16, 4, 8)
                == if front {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                },
            "Actual pin depth/tie output colors",
        )?;
    }
    let mut strict = overlap.clone();
    strict.order.clear();
    check(!strict.valid(), "Ordinary mesh accepted folded geometry")?;
    Ok(())
}
pub fn editable_native(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::deform_verify::fixture()?;
    let before = d.snapshot();
    let counts = d.history_counts();
    let mut settings = Settings {
        mesh: Mesh::new(3, 3)?,
        puppet: false,
        pins: vec![],
        selected: vec![0, 1],
    };
    settings.mesh.move_selected(&[0, 1], [0.1, 0.05])?;
    let out = mesh_edit::prepare(&d, &[0], false, &settings, false, &mut || Ok(()))?;
    check(
        d.snapshot() == before && d.history_counts() == counts && out.scene.mesh_edit.is_some(),
        "Editable mesh preview isolation/archive",
    )?;
    d.commit_state(out)?;
    let first = d.snapshot();
    let path = root.join("editable-mesh.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let mut reopened =
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == first
            && reopened
                .scene
                .mesh_edit
                .as_ref()
                .unwrap()
                .matches(&reopened),
        "Native mesh editing state/source payload mismatch",
    )?;
    check(
        mesh_edit::prepare(&reopened, &[0], false, &settings, true, &mut || Ok(()))? == first,
        "Unchanged resume resampled baked pixels",
    )?;
    settings.mesh.move_selected(&[0, 1], [0.04, 0.02])?;
    let resumed = mesh_edit::prepare(&reopened, &[0], false, &settings, true, &mut || Ok(()))?;
    let direct = mesh_edit::prepare(
        &crate::deform_verify::fixture()?,
        &[0],
        false,
        &settings,
        false,
        &mut || Ok(()),
    )?;
    check(
        resumed.layers == direct.layers
            && resumed.scene.mesh_edit.as_ref().unwrap().settings == settings,
        "Resume accumulated resampling instead of original pixels",
    )?;
    reopened.layers[0].pixels[0] = [7, 8, 9, 255];
    check(
        !reopened
            .scene
            .mesh_edit
            .as_ref()
            .unwrap()
            .matches(&reopened)
            && mesh_edit::prepare(&reopened, &[0], false, &settings, true, &mut || Ok(())).is_err(),
        "Painted source accepted stale saved mesh",
    )?;
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == first,
        "Mesh archive/pixels one Undo/Redo",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&80u32.to_le_bytes());
    let old = root.join("fake-v80-mesh.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native header accepted editable mesh source",
    )?;
    let original = crate::deform_verify::fixture()?;
    let identity = Settings {
        mesh: Mesh::new(3, 3)?,
        puppet: true,
        pins: vec![Pin {
            vertex: 4,
            target: [0.5; 2],
            depth: 3,
            keep_angle: true,
            degrees: 90.,
            ..Default::default()
        }],
        selected: vec![4],
    };
    let state = mesh_edit::prepare(&original, &[0], false, &identity, false, &mut || Ok(()))?;
    check(
        state.layers == original.layers
            && state
                .scene
                .mesh_edit
                .as_ref()
                .is_some_and(|e| e.settings == identity),
        "Unmoved pin editing state was discarded",
    )?;
    Ok(())
}
pub fn precise_selection(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::deform_verify::fixture()?;
    d.color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        ..Default::default()
    };
    let mut high = vec![[0u16; 4]; 256];
    high[8 * 16 + 8] = [12345, 23456, 34567, 1];
    high[12 * 16 + 12] = [4444, 3333, 2222, 45678];
    d.layers[0].pixels = high
        .iter()
        .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
        .collect();
    d.layers[0].precise = Some(high.clone());
    d.scene.selection = Some(Rect {
        min: [4.; 2],
        max: [10.; 2],
    });
    let settings = Settings {
        mesh: Mesh::new(3, 3)?,
        puppet: true,
        pins: vec![Pin {
            vertex: 4,
            target: [0.6, 0.5],
            degrees: 20.,
            depth: 12,
            keep_angle: true,
            reference_degrees: 0.,
            ..Default::default()
        }],
        selected: vec![4],
    };
    d.scene.layer_ids.initialize(d.layers.len())?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let out = mesh_edit::prepare(&d, &[0], true, &settings, false, &mut || Ok(()))?;
    let index = out.layers[0].pixel_index(16, 16, 12, 12).unwrap();
    check(
        out.layers[0].precise.as_ref().unwrap()[index] == high[12 * 16 + 12],
        "Mesh selected-pixel edit changed unselected16-bit sample",
    )?;
    d.commit_state(out)?;
    let path = root.join("puppet-precision-selection.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let saved = reopened
        .scene
        .mesh_edit
        .as_ref()
        .ok_or("Missing puppet recipe")?;
    let restored = saved.original(&reopened)?.snapshot();
    check(
        saved.settings == settings
            && restored.layers == original.layers
            && restored.color == original.color
            && restored.scene.selection == original.scene.selection
            && restored.scene.selection_mask == original.scene.selection_mask
            && restored.scene.selection_translation == original.scene.selection_translation,
        "Puppet depth/angle/precise source/selection native roundtrip",
    )?;
    check(
        d.history_counts().0 == counts.0 + 1 && d.undo() && d.snapshot() == original,
        "Puppet precise source Undo",
    )?;
    let failed = mesh_edit::prepare(&d, &[0], true, &settings, false, &mut || {
        Err("Cancelled".into())
    });
    check(
        failed.is_err() && d.snapshot() == original,
        "Cancelled editable mesh changed source",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes.truncate(bytes.len() / 2);
    let corrupt = root.join("mesh-truncated.drawer");
    std::fs::write(&corrupt, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&corrupt, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Truncated mesh source accepted",
    )?;
    Ok(())
}
pub fn effect_coordinates(root: &std::path::Path) -> Result<(), String> {
    use crate::surface_effects::{Entry, Operation, Stack};
    let mut d = Document::create(
        CanvasSpec {
            width: 40,
            height: 20,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for y in 5..8 {
        for x in 3..6 {
            d.layers[0].pixels[y * 40 + x] = [255, 0, 0, 255];
        }
    }
    d.scene.surface_effects.rasters.insert(
        1,
        Stack {
            effects: vec![Entry {
                operation: Operation::Shadow {
                    blur: 0,
                    direction: 0.,
                    distance: 10.,
                    color: [0, 0, 0],
                    opacity: 1.,
                },
                enabled: true,
                scope: None,
            }],
            ..Default::default()
        },
    );
    let key = Key::Layer(Target::Raster(1));
    let original = d.snapshot();
    let object = affine_transform::prepare(
        &d,
        key,
        false,
        Transform {
            scale: [2.; 2],
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?
    .state;
    check(
        object.scene.surface_effects == original.scene.surface_effects,
        "Object scaling changed effect parameters/coordinate frame",
    )?;
    let object_doc = crate::layer_merge::preview(&object, 0)?;
    let render = crate::float_render::Renderer::new(&object_doc)?;
    check(
        render.surface_sample(0, [16, 10])?.rgba8()[3] > 0
            && render.surface_sample(0, [31, 10])?.rgba8()[3] == 0,
        "Object scaling also scaled10px shadow distance",
    )?;
    let resized = crate::sizing::prepare(
        &original,
        crate::sizing::Change::Resample {
            width: 80,
            height: 40,
            dpi: 300.,
            filter: Filter::Nearest,
        },
        DEFAULT_DOCUMENT_BUDGET,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    let mut scaled = crate::layer_merge::preview(&resized, 0)?;
    let render = crate::float_render::Renderer::new(&scaled)?;
    check(
        render.surface_sample(0, [26, 10])?.rgba8()[3] > 0
            && render.surface_sample(0, [16, 10])?.rgba8()[3] == 0,
        "Document resizing did not scale effect output to20px",
    )?;
    check(
        scaled.scene.surface_effects.rasters[&1].effects
            == original.scene.surface_effects.rasters[&1].effects,
        "Document resizing rewrote original effect parameters",
    )?;
    drop(render);
    let path = root.join("effect-coordinate-frame.drawer");
    crate::files::save(&mut scaled, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == scaled.snapshot(),
        "Effect coordinate frame/filter native roundtrip",
    )?;
    check(d.snapshot() == original, "Coordinate review changed source")?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    width_pivots(root)?;
    depth_and_selection()?;
    editable_native(root)?;
    effect_coordinates(root)?;
    precise_selection(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn puppet_precise_source_selection_cancel_truncated_native() {
        let root = crate::test_support::TempDir::new();
        super::precise_selection(&root.0).unwrap();
    }
    #[test]
    fn object_document_effect_coordinate_frames() {
        let root = crate::test_support::TempDir::new();
        super::effect_coordinates(&root.0).unwrap();
    }
    #[test]
    fn width_group_individual_pivots_oriented_handles_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::width_pivots(&root.0).unwrap();
    }
    #[test]
    fn mesh_group_selection_puppet_depth_ties_reference_angles() {
        super::depth_and_selection().unwrap();
    }
    #[test]
    fn editable_mesh_original_source_payload_resume_stale_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::editable_native(&root.0).unwrap();
    }
}
