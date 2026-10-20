use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    material_layer::{Image, Layer, Source},
    material_mapping::{Tiling, UNIT, Warp},
    scene::{Affine, Rect},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn image() -> Layer {
    Layer {
        source: Source::Image(Image {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::new(vec![
                [10, 1, 2, 64],
                [20, 3, 4, 128],
                [30, 5, 6, 192],
                [40, 7, 8, 255],
            ]),
        }),
        area: Rect {
            min: [-4.; 2],
            max: [6.; 2],
        },
        transform: Default::default(),
        repeat: [true; 2],
        mapping: Default::default(),
    }
}
pub fn warps() -> Vec<Warp> {
    let q = [[0.1, 0.05], [1.2, 0.15], [0.9, 1.2], [-0.1, 0.8]];
    let mut mesh = (0..3)
        .flat_map(|y| (0..3).map(move |x| [x as f64 / 2., y as f64 / 2.]))
        .collect::<Vec<_>>();
    mesh[4] = [0.6, 0.4];
    vec![
        Warp::None,
        Warp::Distort(q),
        Warp::Perspective(q),
        Warp::Mesh {
            columns: 3,
            rows: 3,
            points: mesh,
        },
    ]
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    // An independent discrete oracle checks negative coordinates as well as
    // every axis/mode combination, without calling the mapping functions.
    for tiling in Tiling::ALL {
        for repeat in [[false; 2], [true, false], [false, true], [true; 2]] {
            let mut layer = image();
            layer.repeat = repeat;
            layer.mapping.tiling = tiling;
            for y in -5i32..7 {
                for x in -5i32..7 {
                    let expected = if !(-4..6).contains(&x)
                        || !(-4..6).contains(&y)
                        || (!repeat[0] && !(0..2).contains(&x))
                        || (!repeat[1] && !(0..2).contains(&y))
                    {
                        [0; 4]
                    } else {
                        let t = [
                            if repeat[0] { x.div_euclid(2) } else { 0 },
                            if repeat[1] { y.div_euclid(2) } else { 0 },
                        ];
                        let mut xy = [x.rem_euclid(2), y.rem_euclid(2)];
                        let reversed = match tiling {
                            Tiling::Repeat => [false; 2],
                            Tiling::Reverse => [t[0].rem_euclid(2) == 1, t[1].rem_euclid(2) == 1],
                            Tiling::Flip => [(t[0] + t[1]).rem_euclid(2) == 1; 2],
                        };
                        for (v, reverse) in xy.iter_mut().zip(reversed) {
                            if reverse {
                                *v = 1 - *v;
                            }
                        }
                        [
                            [10, 1, 2, 64],
                            [20, 3, 4, 128],
                            [30, 5, 6, 192],
                            [40, 7, 8, 255],
                        ][(xy[1] * 2 + xy[0]) as usize]
                    };
                    check(
                        layer.pixel([x as f64 + 0.5, y as f64 + 0.5]) == expected,
                        "Tile axis/mode/negative boundary differs from discrete reference",
                    )?;
                }
            }
        }
    }
    for warp in warps() {
        check(warp.valid(), "Valid mapping rejected")?;
        for y in 0..=10 {
            for x in 0..=10 {
                let uv = [x as f64 / 10., y as f64 / 10.];
                let p = warp.forward(uv).ok_or("Forward mapping failed")?;
                let original = warp.inverse(p).ok_or("Inverse mapping failed")?;
                check(
                    original
                        .into_iter()
                        .zip(uv)
                        .all(|(a, b)| (a - b).abs() < 1e-6),
                    "Mapping forward/inverse disagreement",
                )?;
            }
        }
        let mut layer = image();
        layer.area = Rect {
            min: [0.; 2],
            max: [2.; 2],
        };
        layer.mapping.warp = warp;
        layer.transform = Affine([0., 2., -2., 0., 7., 3.]);
        let source = layer.source.clone();
        let bounds = layer.bounds();
        for y in 0..2 {
            for x in 0..2 {
                let uv = [(x as f64 + 0.5) / 2., (y as f64 + 0.5) / 2.];
                let q = layer.mapping.warp.forward(uv).unwrap();
                let p = layer.transform.map(q.map(|v| v * 2.));
                let expected = if let Source::Image(i) = &source {
                    i.pixels[y * 2 + x]
                } else {
                    unreachable!()
                };
                check(
                    layer.pixel(p) == expected
                        && bounds.min.into_iter().zip(p).all(|(a, b)| a <= b)
                        && bounds.max.into_iter().zip(p).all(|(a, b)| a >= b),
                    "Transformed material changed source RGBA or bounds",
                )?;
            }
        }
        check(layer.source == source, "Mapping modified original bitmap")?;
        let mut d = crate::search_verify::page()?;
        let before = d.snapshot();
        let (state, id) =
            crate::material_layer::prepare(&d, None, "Warped local image", layer.clone())?;
        check(d.snapshot() == before, "Mapping prepare mutated source")?;
        d.commit_state(state)?;
        let after = d.snapshot();
        let path = root.join("material-mapping.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(reopened.snapshot() == after, "Mapping native roundtrip")?;
        if !layer.mapping.is_default() {
            let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            bytes[8..12].copy_from_slice(&62u32.to_le_bytes());
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            check(
                crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).is_err(),
                "New mapping accepted with old file header",
            )?;
        }
        check(
            d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
            "Mapping single Undo/Redo",
        )?;
        let key = crate::layer_search::Key::Object(id);
        let r = crate::render::Renderer::new(&d);
        for y in 0..16 {
            for x in 0..16 {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                check(
                    r.source_pixel_unbounded(key, point) == layer.pixel(point),
                    "Renderer ignored material deformation",
                )?;
            }
        }
    }
    let mut folded = UNIT;
    folded.swap(1, 3);
    check(
        !Warp::Distort(folded).valid() && !Warp::Perspective(folded).valid(),
        "Folded quadrilateral accepted",
    )?;
    check(
        !Warp::Mesh {
            columns: 255,
            rows: 255,
            points: vec![],
        }
        .valid(),
        "Oversized mesh accepted",
    )?;
    let a = Warp::Distort([[0., 0.], [1., 0.], [0.8, 1.], [0.2, 1.]]);
    let b = Warp::Perspective([[0., 0.], [1., 0.], [0.8, 1.], [0.2, 1.]]);
    check(
        a.forward([0.5, 0.5]) != b.forward([0.5, 0.5]),
        "Distort and Perspective collapsed to one operation",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn tiles_axes_negative_seams_deformations_native_rgba_single_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
