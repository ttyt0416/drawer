use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    lighting::*,
    model_mesh::{Material, Surface, Triangle},
    scene::WorldMesh,
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    let mut s = d.scene.clone();
    for (x0, x1, y0, y1, z) in [(-2., 2., -2., 2., 0.), (0.4, 1.3, -0.5, 0.5, 1.)] {
        s.worlds.push(WorldMesh {
            vertices: vec![[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
            edges: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
            color: [255; 4],
        });
    }
    s.catalog.worlds.initialize(2)?;
    for id in [1, 2] {
        s.world3d.surfaces.insert(
            id,
            Surface {
                triangles: vec![
                    Triangle {
                        v: [0, 1, 2],
                        t: None,
                        n: Some([0; 3]),
                        material: 0,
                        smoothing: 1,
                    },
                    Triangle {
                        v: [0, 2, 3],
                        t: None,
                        n: Some([0; 3]),
                        material: 0,
                        smoothing: 1,
                    },
                ],
                normals: vec![[0., 0., 1.]],
                materials: vec![Material {
                    rgba: [1.; 4],
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
    }
    let (c, l) = world3d::default_camera(&d);
    s.camera = Some(c);
    s.world3d.lens = Some(l);
    d.edit_scene(s)?;
    Ok(d)
}
fn apply(d: &mut Document, s: Settings) -> Result<(), String> {
    prepare(d, s, B, &AtomicBool::new(false))?.commit(d)?;
    Ok(())
}
fn pixel(d: &Document) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel([32., 32.])
}
pub fn samples() -> Result<(), String> {
    let mut d = fixture()?;
    let mut s = Settings {
        style: Style::Flat,
        ambient: 0.,
        shadows: false,
        lights: vec![Light {
            kind: Kind::Directional {
                direction: [-1., 0., -1.],
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    apply(&mut d, s.clone())?;
    let lit = pixel(&d);
    check(
        (218..=220).contains(&lit[0]) && lit[0] == lit[1] && lit[3] == 255,
        "Linear diffuse shading incorrect",
    )?;
    s.shadows = true;
    apply(&mut d, s.clone())?;
    check(
        pixel(&d) == [0, 0, 0, 255],
        "Opaque blocker did not cast hard shadow",
    )?;
    let mut scene = d.scene.clone();
    scene
        .catalog
        .world_info
        .entry(2)
        .or_default()
        .properties
        .hidden = true;
    d.edit_scene(scene)?;
    check(pixel(&d) == lit, "Hidden model still cast shadow")?;
    d.undo();
    s.lights.clear();
    apply(&mut d, s.clone())?;
    check(
        pixel(&d) == [0, 0, 0, 255],
        "Zero lights produced transparency / light",
    )?;
    s.style = Style::Unlit;
    apply(&mut d, s.clone())?;
    check(pixel(&d) == [255; 4], "Unlit material depends on lights")?;
    s.style = Style::Flat;
    s.shadows = false;
    s.lights = vec![Light {
        kind: Kind::Point {
            position: [0., 0., 3.],
        },
        intensity: 4.,
        ..Default::default()
    }];
    apply(&mut d, s.clone())?;
    check(
        (169..=171).contains(&pixel(&d)[0]),
        "Point light falloff incorrect",
    )?;
    s.lights[0].kind = Kind::Spot {
        position: [0., 0., 3.],
        direction: [1., 0., 0.],
        angle: 30.,
    };
    apply(&mut d, s.clone())?;
    check(
        pixel(&d) == [0, 0, 0, 255],
        "Outside spotlight cone illuminated",
    )?;
    s.lights[0].kind = Kind::Spot {
        position: [0., 0., 3.],
        direction: [0., 0., -1.],
        angle: 30.,
    };
    apply(&mut d, s.clone())?;
    check(pixel(&d)[0] > 160, "Spotlight center dark")?;
    s.lights = vec![Light {
        color: [1., 0., 0.],
        kind: Kind::Directional {
            direction: [0., 0., -1.],
        },
        ..Default::default()
    }];
    apply(&mut d, s.clone())?;
    check(pixel(&d) == [255, 0, 0, 255], "Light RGB incorrect")?;
    let mut scene = d.scene.clone();
    scene.world3d.surfaces.get_mut(&1).unwrap().normals[0] = [1., 0., 1.];
    d.edit_scene(scene)?;
    s.lights[0].color = [1.; 3];
    s.style = Style::Smooth;
    apply(&mut d, s.clone())?;
    let smooth = pixel(&d);
    s.style = Style::Flat;
    apply(&mut d, s.clone())?;
    check(smooth[0] < pixel(&d)[0], "Smooth normals ignored")?;
    s.style = Style::Toon;
    s.bands = 2;
    apply(&mut d, s.clone())?;
    check(pixel(&d)[0] > smooth[0], "Toon bands ignored")?;
    // Near clipping uses camera-depth ray parameters, not distance to the origin.
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().far = 5.;
    d.edit_scene(scene)?;
    check(pixel(&d) == [0; 4], "Far clipping failed")?;
    d.undo();
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().near = 7.;
    d.edit_scene(scene)?;
    check(pixel(&d) == [0; 4], "Near clipping failed")?;
    d.undo();
    for projection in [
        world3d::Projection::Perspective,
        world3d::Projection::Orthographic,
    ] {
        let mut scene = d.scene.clone();
        scene.world3d.lens.as_mut().unwrap().projection = projection;
        d.edit_scene(scene)?;
        check(
            pixel(&d)[3] == 255,
            "Parallel / perspective face projection failed",
        )?;
    }
    // An opaque front surface wins regardless of world-layer order.
    let mut scene = d.scene.clone();
    for p in &mut scene.worlds[1].vertices {
        p[0] -= 0.85;
        p[2] = 2.;
    }
    scene.world3d.surfaces.get_mut(&2).unwrap().materials[0].rgba = [0., 1., 0., 1.];
    d.edit_scene(scene)?;
    s.style = Style::Unlit;
    apply(&mut d, s)?;
    check(
        crate::render::Renderer::new(&d)
            .source_pixel(crate::layer_search::Key::World(1), [32., 32.])
            == [0; 4]
            && pixel(&d) == [0, 255, 0, 255],
        "Opaque 3D depth / material color failed",
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
    samples()?;
    let mut old = fixture()?;
    let mut scene = old.scene.clone();
    scene.world3d.lens = None;
    old.edit_scene(scene)?;
    let before = pixels(&old);
    apply(
        &mut old,
        Settings {
            style: Style::Wire,
            ..Default::default()
        },
    )?;
    check(
        old.scene.world3d.lens.unwrap().projection == world3d::Projection::Orthographic
            && pixels(&old) == before,
        "Lighting changed legacy orthographic framing",
    )?;
    let dir = root.join("scene-lighting-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture()?;
        let mut settings = Settings {
            guides: true,
            ..Default::default()
        };
        let before = d.snapshot();
        let p = prepare(&d, settings.clone(), B, &AtomicBool::new(false))?;
        check(d.snapshot() == before, "Lighting preview mutated original")?;
        drop(p);
        check(d.snapshot() == before, "Cancel changed lighting")?;
        apply(&mut d, settings.clone())?;
        let saved = d.snapshot();
        d.undo();
        check(d.snapshot() == before, "Lighting Undo failed")?;
        d.redo();
        check(d.snapshot() == saved, "Lighting Redo failed")?;
        let with = pixels(&d);
        let lines = guide_lines(&d.scene);
        check(
            !lines.is_empty()
                && lines
                    .iter()
                    .all(|(l, _)| l.iter().flatten().all(|v| v.is_finite())),
            "Finite guide clipping failed",
        )?;
        settings.guides = false;
        apply(&mut d, settings.clone())?;
        check(
            pixels(&d) == with && guide_lines(&d.scene).is_empty(),
            "Guides changed rendered pixels",
        )?;
        settings.guides = true;
        apply(&mut d, settings.clone())?;
        let mut scene = d.scene.clone();
        scene.world3d.lens.as_mut().unwrap().projection = world3d::Projection::Orthographic;
        scene.world3d.lens.as_mut().unwrap().angles = [15., 30., 5.];
        d.edit_scene(scene)?;
        check(
            vanishing(&d.scene)
                .unwrap()
                .into_iter()
                .all(|v| matches!(v, Vanishing::Parallel(_))),
            "Orthographic has finite vanishing point",
        )?;
        let native = dir.join("lights.drawer");
        let before = d.snapshot();
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        let reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == before && pixels(&reopened) == pixels(&d),
            "Native110 changed lighting / pixels",
        )?;
        let png = dir.join("lit.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        check(
            image::open(png)
                .map_err(|e| e.to_string())?
                .to_rgba8()
                .into_raw()
                == pixels(&d),
            "PNG differs from lit render",
        )?;
        let mut bytes = std::fs::read(&native).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&109u32.to_le_bytes());
        std::fs::write(&native, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, B).is_err(),
            "Native109 accepted lighting",
        )?;
        for invalid in [
            Settings {
                ambient: f64::NAN,
                ..Default::default()
            },
            Settings {
                lights: vec![Light::default(); 9],
                ..Default::default()
            },
            Settings {
                lights: vec![Light {
                    kind: Kind::Directional { direction: [0.; 3] },
                    ..Default::default()
                }],
                ..Default::default()
            },
        ] {
            check(
                prepare(&d, invalid, B, &AtomicBool::new(false)).is_err(),
                "Invalid lighting accepted",
            )?;
        }
        let before = d.snapshot();
        check(
            prepare(&d, settings.clone(), 1, &AtomicBool::new(false)).is_err()
                && prepare(&d, settings.clone(), B, &AtomicBool::new(true)).is_err()
                && d.snapshot() == before,
            "Budget / cancel changed source",
        )?;
        let p = prepare(&d, settings, B, &AtomicBool::new(false))?;
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let before = d.snapshot();
        check(
            p.commit(&mut d).is_err() && d.snapshot() == before,
            "Stale lighting committed",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_lights_shadows_dark_faces_normals_toon_depth_and_clip() {
        super::samples().unwrap();
    }
    #[test]
    fn native110_guides_projection_png_cancel_and_guards() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
    #[test]
    fn bvh_matches_naive_rays() {
        let mut d = super::fixture().unwrap();
        crate::model_library::prepare(
            &d,
            crate::model_library::Edit::Primitive {
                shape: crate::model_library::Shape::Sphere,
                detail: 32,
                placement: Default::default(),
            },
            crate::document::DEFAULT_DOCUMENT_BUDGET,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap()
        .commit(&mut d)
        .unwrap();
        super::apply(&mut d, Default::default()).unwrap();
        for projection in [
            crate::world3d::Projection::Perspective,
            crate::world3d::Projection::Orthographic,
        ] {
            let mut s = d.scene.clone();
            s.world3d.lens.as_mut().unwrap().projection = projection;
            s.world3d.lens.as_mut().unwrap().angles = [15., 25., 10.];
            d.edit_scene(s).unwrap();
            assert!(
                crate::world_render::Trace::new(&d)
                    .unwrap()
                    .compare_queries()
            );
        }
    }
}
