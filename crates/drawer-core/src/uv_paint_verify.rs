use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_library::{self as lib, Placement},
    uv_paint::{self as uv, *},
    world3d,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn fixture(root: &Path) -> Result<(Document, std::path::PathBuf), String> {
    let path = root.join("own-uv.obj");
    std::fs::write(&path,"v -1 -1 0\nv 1 -1 0\nv 1 1 0\nv -1 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n").map_err(|e|e.to_string())?;
    let mut d = crate::model_library_verify::blank()?;
    lib::prepare(
        &d,
        lib::Edit::Import {
            path: path.clone(),
            placement: Placement {
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
    Ok((d, path))
}
fn prep(d: &Document, p: &[[f64; 2]], b: Brush) -> Result<world3d::Prepared, String> {
    uv::prepare(d, 1, p, b, B, &AtomicBool::new(false))
}
fn brush() -> Brush {
    Brush {
        size: 64,
        radius: 3.,
        opacity: 1.,
        rgb: [255, 0, 0],
    }
}
fn texture(d: &Document) -> Vec<u8> {
    d.scene.world3d.surfaces[&1]
        .paint
        .as_ref()
        .unwrap()
        .decode()
        .unwrap()
}
pub fn verify(root: &Path) -> Result<(), String> {
    for size in [64, 1024] {
        let pixels = vec![255; size as usize * size as usize * 4];
        let t = Texture::encode(size, &pixels)?;
        check(
            t.decode()? == pixels,
            "Min/max texture PNG roundtrip failed",
        )?;
    }
    let dir = root.join("uv-paint-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let (mut d, path) = fixture(&dir)?;
        let original = std::fs::read(&path).map_err(|e| e.to_string())?;
        let before = d.snapshot();
        let p = prep(&d, &[[32., 32.], [34., 32.]], brush())?;
        check(
            d.snapshot() == before && p.warnings.is_empty(),
            "UV preview changed original / shared edge false overlap",
        )?;
        drop(p);
        check(d.snapshot() == before, "Stroke cancel changed source")?;
        prep(&d, &[[32., 32.], [34., 32.]], brush())?.commit(&mut d)?;
        let painted = d.snapshot();
        let color = crate::render::Renderer::new(&d).pixel([32., 32.]);
        check(
            color == [255, 0, 0, 255] && texture(&d)[..4] == [255; 4],
            "Projected material texture / unpainted region incorrect",
        )?;
        check(
            d.scene.world3d.lighting.as_ref().unwrap().style == crate::lighting::Style::Unlit,
            "Wire texture not visible in Unlit",
        )?;
        d.undo();
        check(d.snapshot() == before, "Texture stroke not one Undo")?;
        d.redo();
        check(d.snapshot() == painted, "Texture Redo failed")?;
        let png = dir.join("texture.png");
        uv::export(&d, 1, &png)?;
        check(
            image::open(&png)
                .map_err(|e| e.to_string())?
                .to_rgba8()
                .into_raw()
                == texture(&d),
            "PNG export differs from owned texture",
        )?;
        let native = dir.join("paint.drawer");
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        let opened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == painted
                && crate::render::Renderer::new(&opened).pixel([32., 32.]) == color,
            "Native112 texture / rendered pixel differs",
        )?;
        let mut raw = std::fs::read(&native).map_err(|e| e.to_string())?;
        raw[8..12].copy_from_slice(&111u32.to_le_bytes());
        std::fs::write(&native, raw).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, B).is_err(),
            "Native111 accepted painted texture",
        )?;
        check(
            uv::export(&d, 1, &path).is_err()
                && std::fs::read(&path).map_err(|e| e.to_string())? == original,
            "Original model file modified",
        )?;
        let before = d.snapshot();
        let counts = d.history_counts();
        check(
            !prep(
                &d,
                &[[32., 32.]],
                Brush {
                    opacity: 0.,
                    ..brush()
                },
            )?
            .commit(&mut d)?
                && d.snapshot() == before
                && d.history_counts() == counts,
            "Zero opacity changed texture",
        )?;
        // Tile-edge brush coverage wraps to both edges, without a bridge across UV islands.
        let (mut edge, _) = fixture(&dir)?;
        prep(&edge, &[[19.3, 32.]], brush())?.commit(&mut edge)?;
        let pixels = texture(&edge);
        let left = (31 * 64) * 4;
        let right = (31 * 64 + 63) * 4;
        check(
            pixels[left + 1] < 100 && pixels[right + 1] < 100,
            "Brush did not wrap texture seam",
        )?;
        let (mut seams, _) = fixture(&dir)?;
        let mut s = seams.scene.clone();
        let surface = s.world3d.surfaces.get_mut(&1).unwrap();
        surface.texcoords = vec![
            [0.1, 0.1, 0.],
            [0.3, 0.1, 0.],
            [0.3, 0.3, 0.],
            [0.7, 0.7, 0.],
            [0.9, 0.9, 0.],
            [0.7, 0.9, 0.],
        ];
        surface.triangles[0].t = Some([0, 1, 2]);
        surface.triangles[1].t = Some([3, 4, 5]);
        seams.edit_scene(s)?;
        let points = seams.scene.world3d.surfaces[&1]
            .triangles
            .iter()
            .map(|t| {
                let p = std::array::from_fn(|a| {
                    t.v.iter()
                        .map(|i| seams.scene.worlds[0].vertices[*i as usize][a])
                        .sum::<f64>()
                        / 3.
                });
                seams
                    .scene
                    .world3d
                    .lens
                    .unwrap()
                    .project(seams.scene.camera.as_ref().unwrap(), p)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        prep(
            &seams,
            &points,
            Brush {
                radius: 1.,
                ..brush()
            },
        )?
        .commit(&mut seams)?;
        check(
            texture(&seams)[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4] == [255; 4],
            "Stroke bridged unrelated UV islands",
        )?;
        let mut overlap = seams.scene.world3d.surfaces[&1].clone();
        overlap.triangles[1].t = overlap.triangles[0].t;
        check(
            inspect(&overlap)?.iter().any(|w| w.contains("Overlapping")),
            "Overlapping UV not reported",
        )?;
        let mut no_uv = overlap.clone();
        for t in &mut no_uv.triangles {
            t.t = None;
        }
        check(inspect(&no_uv).is_err(), "No-UV model accepted")?;
        let (mut shifted, _) = fixture(&dir)?;
        let mut s = shifted.scene.clone();
        s.transform = s
            .transform
            .then(crate::scene::Affine([1., 0., 0., 1., 4., 2.]));
        shifted.edit_scene(s)?;
        prep(&shifted, &[[36., 34.]], brush())?.commit(&mut shifted)?;
        check(
            crate::render::Renderer::new(&shifted).pixel([36., 34.]) == [255, 0, 0, 255],
            "Page transform projection incorrect",
        )?;
        let before = d.snapshot();
        check(
            uv::prepare(&d, 1, &[[32., 32.]], brush(), 1, &AtomicBool::new(false)).is_err()
                && uv::prepare(&d, 1, &[[32., 32.]], brush(), B, &AtomicBool::new(true)).is_err()
                && d.snapshot() == before,
            "Budget / cancel changed texture",
        )?;
        for b in [
            Brush {
                size: 63,
                ..brush()
            },
            Brush {
                radius: f64::NAN,
                ..brush()
            },
        ] {
            check(
                prep(&d, &[[32., 32.]], b).is_err(),
                "Invalid texture brush accepted",
            )?;
        }
        let mut bad = d.scene.world3d.surfaces[&1].paint.clone().unwrap();
        bad.size = 2048;
        check(bad.decode().is_err(), "Texture dimension limit bypass")?;
        bad.size = 64;
        bad.png = "bad".into();
        check(bad.decode().is_err(), "Corrupt texture accepted")?;
        lib::prepare(
            &d,
            lib::Edit::Primitive {
                shape: lib::Shape::Box,
                detail: 6,
                placement: Placement {
                    position: [0., 0., 2.],
                    scale: [0.8, 0.8, 0.2],
                    fit: None,
                    ..Default::default()
                },
            },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
        check(
            prep(&d, &[[32., 32.]], brush()).is_err(),
            "Paint passed through front geometry",
        )?;
        d.undo();
        let p = prep(
            &d,
            &[[32., 32.]],
            Brush {
                rgb: [0, 0, 255],
                ..brush()
            },
        )?;
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let before = d.snapshot();
        check(
            p.commit(&mut d).is_err() && d.snapshot() == before,
            "Stale texture committed",
        )?;
        let original_map = texture(&d);
        lib::prepare(
            &d,
            lib::Edit::Duplicate {
                ids: vec![1],
                offset: [1.5, 0., 0.],
            },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
        uv::prepare(
            &d,
            2,
            &[[51.3, 32.]],
            Brush {
                rgb: [0, 255, 0],
                ..brush()
            },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
        check(
            texture(&d) == original_map
                && d.scene.world3d.surfaces[&2].paint != d.scene.world3d.surfaces[&1].paint,
            "Copied texture aliases original",
        )?;
        let mut locked = d.scene.clone();
        locked
            .catalog
            .world_info
            .entry(1)
            .or_default()
            .properties
            .locked = true;
        d.edit_scene(locked)?;
        check(
            prep(&d, &[[32., 32.]], brush()).is_err(),
            "Locked UV model accepted paint",
        )?;

        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn projected_uv_pixels_native112_png_seams_overlap_limits_occlusion_and_undo() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
