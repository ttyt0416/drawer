use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_import::Wrap,
    model_library::{self as lib, Shape},
    surface_edit::*,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn edit(d: &mut Document, c: Change) -> Result<(), String> {
    prepare(d, 1, c, B, &AtomicBool::new(false))?.commit(d)?;
    Ok(())
}
pub fn fixture(shape: Shape) -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    lib::prepare(
        &d,
        lib::Edit::Primitive {
            shape,
            detail: 6,
            placement: lib::Placement {
                fit: None,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let mut s = d.scene.clone();
    s.world3d.surfaces.get_mut(&1).unwrap().materials[0].rgba = [1.; 4];
    d.edit_scene(s)?;
    Ok(d)
}
pub fn image(path: &Path, pixels: &[u8], w: u32, h: u32) -> Result<(), String> {
    use image::ImageEncoder;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(pixels, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}
fn pixels(d: &Document) -> Result<Vec<[u8; 4]>, String> {
    Ok(crate::render::Renderer::new(d).output(d.dpi, B / 8)?.2)
}
fn sample(d: &Document, p: [f64; 2]) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel(p)
}
pub fn divisions() -> Result<(), String> {
    for shape in Shape::ALL {
        let mut d = fixture(shape)?;
        let before = d.snapshot();
        let old_frame = d.scene.world3d.nodes[&1].frame;
        let detail = if matches!(shape, Shape::Sphere | Shape::Cylinder | Shape::Cone) {
            12
        } else {
            3
        };
        edit(&mut d, Change::Subdivide(detail))?;
        check(
            d.scene.worlds[0].vertices.len() > before.scene.worlds[0].vertices.len()
                && d.scene.world3d.nodes[&1].frame == old_frame
                && d.scene.world3d.valid(&d.scene),
            "Primitive subdivisions did not preserve valid axes / increase geometry",
        )?;
        let result = d.snapshot();
        d.undo();
        check(d.snapshot() == before, "Subdivision Undo differs")?;
        d.redo();
        check(d.snapshot() == result, "Subdivision Redo differs")?;
        for value in [0, 65535] {
            let snap = d.snapshot();
            check(
                prepare(&d, 1, Change::Subdivide(value), B, &AtomicBool::new(false)).is_err()
                    && d.snapshot() == snap,
                "Invalid/oversized subdivision changed input",
            )?;
        }
    }
    let mut d = fixture(Shape::Chair)?;
    check(
        prepare(&d, 1, Change::Subdivide(32), B, &AtomicBool::new(false)).is_err(),
        "Oversized edge subdivision accepted",
    )?;
    edit(&mut d, Change::GenerateUv)?;
    check(
        d.scene.world3d.surfaces[&1]
            .triangles
            .iter()
            .all(|t| t.t.is_some()),
        "Box-projected UV chart missing",
    )?;
    let mut imported = d.scene.clone();
    imported
        .world3d
        .surfaces
        .get_mut(&1)
        .unwrap()
        .tools
        .primitive = None;
    d.edit_scene(imported)?;
    check(
        prepare(&d, 1, Change::Subdivide(3), B, &AtomicBool::new(false)).is_err(),
        "Imported topology silently rebuilt",
    )?;
    Ok(())
}
pub fn mapping(root: &Path) -> Result<(), String> {
    let path = root.join("color.png");
    image(
        &path,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ],
        2,
        2,
    )?;
    let source = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut d = fixture(Shape::Plane)?;
    let before = d.snapshot();
    check(
        prepare(
            &d,
            1,
            Change::Load {
                material: 0,
                normal: false,
                path: path.clone(),
            },
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Map with missing UVs accepted",
    )?;
    edit(&mut d, Change::GenerateUv)?;
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: false,
            path: path.clone(),
        },
    )?;
    check(
        std::fs::read(&path).map_err(|e| e.to_string())? == source,
        "Texture load wrote original",
    )?;
    check(
        sample(&d, [41., 23.]) == [0, 255, 0, 255],
        "Canonical diffuse UV / vertical image orientation incorrect",
    )?;
    let original = pixels(&d)?;
    let mut uv = Uv {
        offset: [0.5, 0.],
        ..Default::default()
    };
    edit(
        &mut d,
        Change::Uv {
            material: 0,
            value: uv.clone(),
        },
    )?;
    check(
        sample(&d, [41., 23.]) == [255, 0, 0, 255],
        "UV translation did not repeat",
    )?;
    uv.offset = [0.; 2];
    uv.scale = [3., 1.];
    uv.wrap = [Some(Wrap::Mirror), Some(Wrap::Clamp)];
    edit(
        &mut d,
        Change::Uv {
            material: 0,
            value: uv.clone(),
        },
    )?;
    check(
        pixels(&d)? != original,
        "Mirror/scale did not affect actual texture",
    )?;
    uv.angle = 90.;
    edit(
        &mut d,
        Change::Uv {
            material: 0,
            value: uv.clone(),
        },
    )?;
    check(pixels(&d)? != original, "UV rotation ignored")?;
    uv.scale = [-1., 1.];
    uv.angle = 0.;
    edit(
        &mut d,
        Change::Uv {
            material: 0,
            value: uv.clone(),
        },
    )?;
    check(
        sample(&d, [41., 23.]) == [255, 0, 0, 255],
        "Negative UV scale did not flip image",
    )?;
    for invalid in [0., f64::NAN, 129.] {
        uv.scale[0] = invalid;
        check(
            prepare(
                &d,
                1,
                Change::Uv {
                    material: 0,
                    value: uv.clone(),
                },
                B,
                &AtomicBool::new(false),
            )
            .is_err(),
            "Invalid UV scale accepted",
        )?;
    }
    let cached_pixels = pixels(&d)?;
    image(&path, &[255, 255, 0, 255], 1, 1)?;
    let cached = d.snapshot();
    check(
        pixels(&d)? == cached_pixels,
        "External change implicitly reloaded",
    )?;
    edit(
        &mut d,
        Change::Reload {
            material: 0,
            normal: false,
        },
    )?;
    check(
        sample(&d, [32., 32.]) == [255, 255, 0, 255],
        "Manual texture reload did not replace snapshot",
    )?;
    d.undo();
    check(d.snapshot() == cached, "Texture reload Undo differs")?;
    d.redo();
    let snapshot = d.snapshot();
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    check(
        prepare(
            &d,
            1,
            Change::Reload {
                material: 0,
                normal: false,
            },
            B,
            &AtomicBool::new(false),
        )
        .is_err()
            && d.snapshot() == snapshot
            && sample(&d, [32., 32.]) == [255, 255, 0, 255],
        "Missing reload discarded cached image",
    )?;
    edit(&mut d, Change::ClearTextures)?;
    edit(&mut d, Change::Subdivide(2))?;
    d.undo();
    d.undo();
    check(
        d.snapshot() == snapshot,
        "Clear maps/subdivision Undo lost texture data",
    )?;
    check(
        before.scene.worlds[0].vertices == d.scene.worlds[0].vertices,
        "Map edits changed original geometry",
    )?;
    Ok(())
}
pub fn normals(root: &Path) -> Result<(), String> {
    let mut d = fixture(Shape::Plane)?;
    edit(&mut d, Change::GenerateUv)?;
    let path = root.join("normal.png");
    image(&path, &[128, 230, 200, 255], 1, 1)?;
    crate::lighting::prepare(
        &d,
        crate::lighting::Settings {
            style: crate::lighting::Style::Flat,
            ambient: 0.,
            shadows: false,
            lights: vec![crate::lighting::Light {
                kind: crate::lighting::Kind::Directional {
                    direction: [0., -1., -1.],
                },
                ..Default::default()
            }],
            ..Default::default()
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let base = sample(&d, [32., 32.]);
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: true,
            path: path.clone(),
        },
    )?;
    let bumped = sample(&d, [32., 32.]);
    check(
        bumped[0] > base[0],
        "Tangent normal map did not change actual light",
    )?;
    edit(
        &mut d,
        Change::Normal {
            material: 0,
            strength: 0.,
            flip_green: false,
        },
    )?;
    check(
        sample(&d, [32., 32.]) == base,
        "Normal strength0 not identity",
    )?;
    edit(
        &mut d,
        Change::Normal {
            material: 0,
            strength: 1.,
            flip_green: true,
        },
    )?;
    check(
        sample(&d, [32., 32.])[0] < base[0],
        "Green inversion did not change normal handedness",
    )?;
    let saved = d.snapshot();
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    check(
        prepare(
            &d,
            1,
            Change::Reload {
                material: 0,
                normal: true,
            },
            B,
            &AtomicBool::new(false),
        )
        .is_err()
            && d.snapshot() == saved,
        "Missing normal source destroyed embedded data",
    )?;
    let mut scene = d.scene.clone();
    scene
        .world3d
        .surfaces
        .get_mut(&1)
        .unwrap()
        .texcoords
        .fill([0.; 3]);
    d.edit_scene(scene)?;
    check(
        sample(&d, [32., 32.]) == base,
        "Degenerate UV normal fallback invalid",
    )?;
    check(
        prepare(
            &d,
            1,
            Change::Normal {
                material: 0,
                strength: 4.01,
                flip_green: false,
            },
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Normal strength limit bypass",
    )?;
    Ok(())
}
pub fn rendering(root: &Path) -> Result<(), String> {
    let mut d = fixture(Shape::Box)?;
    edit(&mut d, Change::Display(Display::Face))?;
    let face = pixels(&d)?;
    edit(&mut d, Change::Display(Display::Wire))?;
    let wire = pixels(&d)?;
    check(
        wire != face
            && wire.iter().filter(|p| p[3] > 0).count() < face.iter().filter(|p| p[3] > 0).count(),
        "Wire display retained opaque faces",
    )?;
    edit(&mut d, Change::Display(Display::Both))?;
    let both = pixels(&d)?;
    check(
        both != wire && both != face,
        "Both display did not overlay wire",
    )?;
    let original = d.scene.worlds.clone();
    let mut setting = d.scene.world3d.lighting.clone().unwrap();
    setting.render = Some(Default::default());
    setting.render.as_mut().unwrap().objects.insert(
        1,
        crate::scene_render::Object {
            billboard: crate::scene_render::Billboard::AllAxes,
            ..Default::default()
        },
    );
    crate::lighting::prepare(&d, setting.clone(), B, &AtomicBool::new(false))?.commit(&mut d)?;
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().angles = [15., 15., 0.];
    d.edit_scene(scene)?;
    let full = pixels(&d)?;
    setting
        .render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .billboard = crate::scene_render::Billboard::OnlyHorizontal;
    crate::lighting::prepare(&d, setting, B, &AtomicBool::new(false))?.commit(&mut d)?;
    check(
        pixels(&d)? != full && d.scene.worlds == original,
        "Billboard modes identical or baked geometry",
    )?;
    let path = root.join("alpha.png");
    image(&path, &[255, 255, 255, 0], 1, 1)?;
    let mut d = fixture(Shape::Plane)?;
    edit(&mut d, Change::GenerateUv)?;
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: false,
            path,
        },
    )?;
    edit(
        &mut d,
        Change::Material {
            material: 0,
            alpha: Some(crate::model_import::Alpha::Opaque),
            cutoff: 0.5,
        },
    )?;
    check(
        sample(&d, [32., 32.])[3] == 255,
        "Per-material opaque left texture alpha",
    )?;
    edit(
        &mut d,
        Change::Material {
            material: 0,
            alpha: Some(crate::model_import::Alpha::Mask),
            cutoff: 0.,
        },
    )?;
    check(
        sample(&d, [32., 32.])[3] == 255,
        "Per-material alpha0/threshold0 equality failed",
    )?;
    edit(
        &mut d,
        Change::Material {
            material: 0,
            alpha: Some(crate::model_import::Alpha::Mask),
            cutoff: 1.,
        },
    )?;
    check(
        sample(&d, [32., 32.])[3] == 0,
        "Per-material threshold1 accepted alpha0",
    )?;
    edit(
        &mut d,
        Change::Material {
            material: 0,
            alpha: Some(crate::model_import::Alpha::Blend),
            cutoff: 0.5,
        },
    )?;
    check(
        sample(&d, [32., 32.])[3] == 0,
        "Per-material blend ignored alpha",
    )?;
    Ok(())
}
pub fn delivery(root: &Path) -> Result<(), String> {
    let mut d = fixture(Shape::Plane)?;
    edit(&mut d, Change::GenerateUv)?;
    let a = root.join("same.png");
    let b = root.join("second");
    std::fs::create_dir(&b).map_err(|e| e.to_string())?;
    let b = b.join("same.png");
    image(&a, &[255, 0, 0, 255], 1, 1)?;
    image(&b, &[128, 128, 255, 255], 1, 1)?;
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: false,
            path: a.clone(),
        },
    )?;
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: true,
            path: b.clone(),
        },
    )?;
    let bytes_a = std::fs::read(&a).map_err(|e| e.to_string())?;
    let bytes_b = std::fs::read(&b).map_err(|e| e.to_string())?;
    let snapshot = d.snapshot();
    let export = Export::capture(&d, B, &AtomicBool::new(false))?;
    check(
        export
            .write(&root.join("fail"), Conflict::Fail, &AtomicBool::new(false))
            .is_err()
            && !root.join("fail").exists(),
        "Duplicate names partially published",
    )?;
    let report = export.write(&root.join("skip"), Conflict::Skip, &AtomicBool::new(false))?;
    check(
        report.files == 1 && report.skipped == 1,
        "Duplicate skip policy incorrect",
    )?;
    let report = export.write(&root.join("all"), Conflict::Rename, &AtomicBool::new(false))?;
    check(
        report.files == 2 && report.folder.join("texture-same-2.png").is_file(),
        "Duplicate rename policy incorrect",
    )?;
    let again = export.write(&root.join("all"), Conflict::Rename, &AtomicBool::new(false))?;
    check(
        again.folder.canonicalize().map_err(|e| e.to_string())?
            == root
                .join("all-2")
                .canonicalize()
                .map_err(|e| e.to_string())?
            && again.files == 2,
        "Existing folder rename incorrect",
    )?;
    let skipped = export.write(&root.join("all"), Conflict::Skip, &AtomicBool::new(false))?;
    check(
        skipped.files == 0 && skipped.skipped == 2,
        "Existing folder skip incorrect",
    )?;
    check(
        export
            .write(
                &root.join("cancel"),
                Conflict::Rename,
                &AtomicBool::new(true),
            )
            .is_err()
            && !root.join("cancel").exists(),
        "Canceled export published output",
    )?;
    let flag = AtomicBool::new(false);
    let canceled = root.join("mid-cancel");
    check(
        export
            .write_with(&canceled, Conflict::Rename, &flag, || {
                flag.store(true, std::sync::atomic::Ordering::Relaxed)
            })
            .is_err()
            && !canceled.exists(),
        "Cancellation after staging published textures",
    )?;
    let raced = root.join("raced");
    check(
        export
            .write_with(&raced, Conflict::Rename, &AtomicBool::new(false), || {
                std::fs::create_dir(&raced).unwrap();
                std::fs::write(raced.join("foreign.txt"), b"keep").unwrap();
            })
            .is_err()
            && std::fs::read(raced.join("foreign.txt")).map_err(|e| e.to_string())? == b"keep",
        "Late foreign output folder changed or merged",
    )?;
    check(
        d.snapshot() == snapshot
            && std::fs::read(&a).map_err(|e| e.to_string())? == bytes_a
            && std::fs::read(&b).map_err(|e| e.to_string())? == bytes_b,
        "Export mutated original document/images",
    )?;
    check(
        !std::fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .any(|p| {
                p.file_name()
                    .to_string_lossy()
                    .starts_with(".drawer-textures-")
            }),
        "Texture staging leaked",
    )?;
    Ok(())
}
pub fn native(root: &Path) -> Result<(), String> {
    let mut d = fixture(Shape::Plane)?;
    edit(&mut d, Change::GenerateUv)?;
    let image_path = root.join("snapshot.png");
    image(&image_path, &[128, 128, 255, 255], 1, 1)?;
    edit(
        &mut d,
        Change::Load {
            material: 0,
            normal: true,
            path: image_path,
        },
    )?;
    edit(&mut d, Change::Display(Display::Both))?;
    let snap = d.snapshot();
    let output = pixels(&d)?;
    let path = root.join("surface.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == snap && pixels(&reopened)? == output,
        "Native120 surface map/options/pixels differ",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&119u32.to_le_bytes());
    std::fs::write(root.join("old.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "Native119 accepted editable surface metadata",
    )?;
    lib::prepare(
        &d,
        lib::Edit::Duplicate {
            ids: vec![1],
            offset: [2., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.surfaces[&2].tools == d.scene.world3d.surfaces[&1].tools,
        "Duplicate lost material snapshots",
    )?;
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            prepare(
                &d,
                1,
                Change::Display(Display::Wire),
                budget,
                &AtomicBool::new(cancel),
            )
            .is_err(),
            "Surface budget/cancel bypass",
        )?;
    }
    let prepared = crate::model_edit::prepare(
        &d,
        crate::model_edit::Edit::Surface {
            model: 1,
            change: Change::Display(Display::Wire),
        },
        B,
        &AtomicBool::new(false),
    )?;
    d.current_frame += 1;
    check(
        prepared.commit(&mut d).is_err(),
        "Stale surface preview frame accepted",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("surface-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        divisions()?;
        mapping(&dir)?;
        normals(&dir)?;
        rendering(&dir)?;
        delivery(&dir)?;
        native(&dir)
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn primitive_subdivisions_bounds_uv_and_one_undo() {
        super::divisions().unwrap();
    }
    #[test]
    fn texture_uv_repeat_mirror_offset_scale_rotation_reload_and_missing_cache() {
        let t = crate::test_support::TempDir::new();
        super::mapping(&t.0).unwrap();
    }
    #[test]
    fn tangent_normal_strength_green_orientation_and_degenerate_uv() {
        let t = crate::test_support::TempDir::new();
        super::normals(&t.0).unwrap();
    }
    #[test]
    fn wire_face_both_billboard_and_material_alpha_edges() {
        let t = crate::test_support::TempDir::new();
        super::rendering(&t.0).unwrap();
    }
    #[test]
    fn frozen_texture_export_names_folder_conflicts_cancel_and_source_unchanged() {
        let t = crate::test_support::TempDir::new();
        super::delivery(&t.0).unwrap();
    }
    #[test]
    fn native120_independent_copy_budget_cancel_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::native(&t.0).unwrap();
    }
}
