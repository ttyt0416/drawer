use crate::{
    color_precision::Sample,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    gaussian as core,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 5,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([0, 255, 0, 0]);
    d.layers[0].pixels[6] = [255, 0, 0, 255];
    d.layers[0].pixels[13] = [0, 0, 255, 128];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    check(
        core::prepare(&d, 0, 0, false)? == original,
        "Zero Gaussian radius changed pixels/storage",
    )?;
    let p = core::prepare(&d, 0, 2, false)?;
    check(
        d.snapshot() == original && d.history_counts() == counts && p.scene == original.scene,
        "Gaussian preview changed source/history/scene",
    )?;
    // Independent dense 2D convolution, rather than the production separable passes.
    let sigma = 2. / 3.;
    let normal: f64 = (-2i32..=2)
        .flat_map(|y| {
            (-2i32..=2).map(move |x| (-((x * x + y * y) as f64) / (2. * sigma * sigma)).exp())
        })
        .sum();
    let extent = p.layers[0].extent(5, 4);
    for y in 0..extent.height {
        for x in 0..extent.width {
            let pos = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
            let mut sum = [0.; 4];
            for sy in 0..4 {
                for sx in 0..5 {
                    let dx = sx as i64 - pos[0];
                    let dy = sy as i64 - pos[1];
                    if dx.abs() > 2 || dy.abs() > 2 {
                        continue;
                    }
                    let w = (-((dx * dx + dy * dy) as f64) / (2. * sigma * sigma)).exp() / normal;
                    let source = d.layers[0].pixels[sy * 5 + sx];
                    let a = source[3] as f64 / 255.;
                    for k in 0..3 {
                        sum[k] += source[k] as f64 * a * w;
                    }
                    sum[3] += a * w;
                }
            }
            let alpha = (sum[3] * 255.).round() as u8;
            let expected = if alpha == 0 {
                [0; 4]
            } else {
                [
                    (sum[0] / sum[3]).round() as u8,
                    (sum[1] / sum[3]).round() as u8,
                    (sum[2] / sum[3]).round() as u8,
                    alpha,
                ]
            };
            let actual = p.layers[0].pixel(5, 4, pos[0], pos[1]);
            check(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "Gaussian differs from independent 2D premultiplied reference",
            )?;
        }
    }
    d.commit_state(p)?;
    let file = root.join("gaussian-blur.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Gaussian native/one Undo/Redo",
    )?;
    d.undo();
    let mut one = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    one.layers[0].pixels[0] = [255, 20, 50, 200];
    let p = core::prepare(&one, 0, 1, false)?;
    check(
        p.layers[0].pixels.len() == 9
            && p.layers[0]
                .pixels
                .iter()
                .filter(|p| p[3] > 0)
                .all(|p| p[..3] == [255, 20, 50]),
        "One-pixel Gaussian gained a black or hidden-RGB border",
    )?;
    one.color = Default::default();
    one.layers[0].precise = Some(vec![[10000, 50000, 65535, 10]]);
    one.layers[0].pixels[0] = Sample::from16([10000, 50000, 65535, 10]).rgba8();
    let p = core::prepare(&one, 0, 1, false)?;
    check(
        p.layers[0].precise.as_ref().unwrap()[4] == [10000, 50000, 65535, 6],
        "Native16 low-alpha Gaussian lost ink/color precision",
    )?;
    let mut selected = fixture()?;
    selected.scene.selection = Some(crate::scene::Rect {
        min: [0., 0.],
        max: [2., 2.],
    });
    let p = core::prepare(&selected, 0, 2, true)?;
    for y in 0..4 {
        for x in 0..5 {
            if x >= 2 || y >= 2 {
                check(
                    p.layers[0].pixel(5, 4, x, y) == selected.layers[0].pixel(5, 4, x, y),
                    "Gaussian changed pixels outside selection",
                )?;
            }
        }
    }
    selected.scene.selection_mask = Some(Default::default());
    check(
        core::prepare(&selected, 0, 2, true)? == selected.snapshot(),
        "Empty Gaussian selection created an edit",
    )?;
    let mut locked = fixture()?;
    locked
        .scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&locked, 0));
    let p = core::prepare(&locked, 0, 2, false)?;
    check(
        p.layers[0].placement == locked.layers[0].placement
            && p.layers[0]
                .pixels
                .iter()
                .zip(&locked.layers[0].pixels)
                .all(|(a, b)| a[3] == b[3])
            && p.layers[0].pixels != locked.layers[0].pixels,
        "Gaussian transparency protection/inside-color blur",
    )?;
    d.layers[0].locked = true;
    check(
        core::prepare(&d, 0, 1, false).is_err(),
        "Locked Gaussian target accepted",
    )?;
    d.layers[0].locked = false;
    check(
        core::prepare(&d, 0, 65, false).is_err() && d.snapshot() == original,
        "Invalid Gaussian radius changed source",
    )?;
    let mut wide = Document::create(
        CanvasSpec {
            width: 32768,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    wide.layers[0].pixels[0] = [255; 4];
    let before = wide.snapshot();
    check(
        core::prepare(&wide, 0, 1, false).is_err() && wide.snapshot() == before,
        "Gaussian oversized extent left partial data",
    )?;
    let mut budget = Document::create(
        CanvasSpec {
            width: 512,
            height: 512,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    budget.layers[0].pixels[0] = [255; 4];
    check(
        core::prepare(&budget, 0, 64, false).is_err(),
        "Gaussian work limit ignored",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn gaussian_dense_reference_alpha_edges_one_pixel_zero_precision_selection_locks_limits_native_and_undo()
     {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
