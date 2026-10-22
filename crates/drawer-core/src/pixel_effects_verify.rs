use crate::{
    color_precision::Sample,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    pixel_effects::{self as core, DustTarget as T, Operation as O},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn canvas(w: u32, h: u32, background: Background) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.,
            background,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn fixture() -> Result<Document, String> {
    let mut d = canvas(4, 4, Background::Transparent)?;
    d.layers[0].pixels.fill([0, 255, 0, 0]);
    d.layers[0].pixels[0] = [255, 0, 0, 255];
    d.layers[0].pixels[1] = [0, 0, 255, 128];
    d.layers[0].pixels[10] = [120, 120, 120, 200];
    Ok(d)
}
fn run(d: &Document, op: O) -> Result<crate::sizing::DocumentState, String> {
    core::prepare(d, 0, op, false, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    for op in [
        O::Mosaic {
            cell: 1,
            origin: [0; 2],
        },
        O::Noise {
            amount: 0.,
            seed: 9,
            monochrome: false,
        },
        O::Dust {
            max_area: 0,
            threshold: 0.5,
            target: T::Ink,
        },
        O::LineWidth { delta: 0 },
    ] {
        check(
            run(&d, op)? == original,
            "Identity pixel effect changed storage",
        )?;
    }
    let mosaic = run(
        &d,
        O::Mosaic {
            cell: 2,
            origin: [0; 2],
        },
    )?;
    let uniform = canvas(5, 7, Background::Color([37, 123, 210]))?;
    check(
        run(
            &uniform,
            O::Mosaic {
                cell: 3,
                origin: [-1, 2],
            },
        )? == uniform.snapshot(),
        "Uniform partial mosaic cells changed pixels",
    )?;
    let mut transparent = fixture()?;
    transparent.layers[0].pixels.fill([0, 255, 0, 0]);
    check(
        run(
            &transparent,
            O::Mosaic {
                cell: 3,
                origin: [0; 2],
            },
        )? == transparent.snapshot(),
        "Transparent mosaic cleared hidden raw RGB",
    )?;
    for i in [0, 1, 4, 5] {
        check(
            mosaic.layers[0].pixels[i] == [170, 0, 85, 96],
            "Mosaic differs from independent premultiplied cell average",
        )?;
    }
    check(
        d.snapshot() == original && d.history_counts() == counts,
        "Pixel preview changed source/history",
    )?;
    let mut placed = fixture()?;
    placed.layers[0].placement = Some(PlacedRaster {
        width: 4,
        height: 4,
        offset: [-3, -3],
    });
    let p = run(
        &placed,
        O::Mosaic {
            cell: 2,
            origin: [0; 2],
        },
    )?;
    check(
        p.layers[0].pixels[0] == placed.layers[0].pixels[0]
            && p.layers[0].pixels[1] != placed.layers[0].pixels[1],
        "Negative document-grid cell alignment/partial cell average",
    )?;
    let mut high = fixture()?;
    high.color = Default::default();
    let mut values = vec![[0; 4]; 16];
    values[0] = [10000, 50000, 65535, 10];
    high.layers[0].pixels = values.iter().map(|v| Sample::from16(*v).rgba8()).collect();
    high.layers[0].precise = Some(values);
    check(
        run(
            &high,
            O::Mosaic {
                cell: 2,
                origin: [0; 2],
            },
        )?
        .layers[0]
            .precise
            .as_ref()
            .unwrap()[0]
            == [10000, 50000, 65535, 3],
        "Native16 low-alpha mosaic lost bits",
    )?;
    let mut gray = canvas(7, 7, Background::Color([128; 3]))?;
    let op = O::Noise {
        amount: 0.2,
        seed: 777,
        monochrome: true,
    };
    let a = run(&gray, op)?;
    let b = run(&gray, op)?;
    check(
        a == b
            && a.layers[0].pixels != gray.layers[0].pixels
            && a.layers[0]
                .pixels
                .iter()
                .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255),
        "Seeded monochrome noise reproducibility/coverage",
    )?;
    check(
        run(
            &gray,
            O::Noise {
                amount: 0.2,
                seed: 778,
                monochrome: true,
            },
        )?
        .layers
            != a.layers,
        "Different noise seeds did not change output",
    )?;
    check(
        run(
            &gray,
            O::Noise {
                amount: 1.,
                seed: u64::MAX,
                monochrome: false,
            },
        )?
        .layers[0]
            .pixels
            .iter()
            .any(|p| p[0] != p[1]),
        "Independent color noise missing",
    )?;
    check(
        run(&d, op)?.layers[0].pixels[4] == d.layers[0].pixels[4],
        "Noise altered hidden transparent RGB",
    )?;
    gray.layers[0].pixels.fill([240, 240, 240, 255]);
    gray.layers[0].pixels[8] = [10, 10, 10, 255];
    gray.layers[0].pixels[24] = [10, 10, 10, 255];
    gray.layers[0].pixels[25] = [10, 10, 10, 255];
    gray.layers[0].pixels[0] = [0, 0, 0, 255];
    // Move the protected boundary speck away from the one-pixel candidate's neighbors.
    gray.layers[0].pixels[0] = [240, 240, 240, 255];
    gray.layers[0].pixels[6] = [0, 0, 0, 255];
    let dust = run(
        &gray,
        O::Dust {
            max_area: 1,
            threshold: 0.1,
            target: T::Dark,
        },
    )?;
    check(
        dust.layers[0].pixels[8] == [240, 240, 240, 255]
            && dust.layers[0].pixels[24] == gray.layers[0].pixels[24]
            && dust.layers[0].pixels[25] == gray.layers[0].pixels[25]
            && dust.layers[0].pixels[6] == gray.layers[0].pixels[6],
        "Speck area/intentional connected mark/border protection",
    )?;
    let mut light = canvas(5, 5, Background::Color([10; 3]))?;
    light.layers[0].pixels[12] = [250, 250, 250, 255];
    check(
        run(
            &light,
            O::Dust {
                max_area: 1,
                threshold: 0.9,
                target: T::Light,
            },
        )?
        .layers[0]
            .pixels[12]
            == [10, 10, 10, 255],
        "Light speck removal",
    )?;
    let mut ink = canvas(4, 4, Background::Transparent)?;
    ink.layers[0].pixels[5] = [255, 10, 20, 128];
    check(
        run(
            &ink,
            O::Dust {
                max_area: 1,
                threshold: 0.1,
                target: T::Ink,
            },
        )?
        .layers[0]
            .pixels[5]
            == [0; 4],
        "Isolated transparent ink not erased",
    )?;
    let expanded = run(&ink, O::LineWidth { delta: 1 })?;
    check(
        expanded.layers[0].extent(4, 4).offset == [-1, -1]
            && expanded.layers[0]
                .pixels
                .iter()
                .filter(|p| p[3] > 0)
                .count()
                == 9
            && expanded.layers[0]
                .pixels
                .iter()
                .filter(|p| p[3] > 0)
                .all(|p| *p == [255, 10, 20, 128]),
        "Line expansion alpha/color/extent",
    )?;
    let shrunk = run(&ink, O::LineWidth { delta: -1 })?;
    check(
        shrunk.layers[0].pixels.iter().all(|p| p[3] == 0),
        "Thin line did not shrink to zero safely",
    )?;
    ink.scene.selection = Some(crate::scene::Rect {
        min: [1., 1.],
        max: [2., 2.],
    });
    let p = core::prepare(
        &ink,
        0,
        O::LineWidth { delta: 1 },
        true,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(p == ink.snapshot(), "Line expansion escaped selection")?;
    ink.scene.selection = None;
    ink.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&ink, 0));
    check(
        run(&ink, O::LineWidth { delta: 1 }).is_err()
            && run(
                &ink,
                O::Dust {
                    max_area: 1,
                    threshold: 0.1,
                    target: T::Ink,
                },
            )
            .is_err(),
        "Alpha-locked shape edit accepted",
    )?;
    let p = run(
        &ink,
        O::Mosaic {
            cell: 2,
            origin: [0; 2],
        },
    )?;
    check(
        p.layers[0]
            .pixels
            .iter()
            .zip(&ink.layers[0].pixels)
            .all(|(a, b)| a[3] == b[3]),
        "Alpha-locked mosaic changed coverage",
    )?;
    let mut selected = fixture()?;
    selected.scene.selection = Some(crate::scene::Rect {
        min: [0., 0.],
        max: [1., 1.],
    });
    let p = core::prepare(
        &selected,
        0,
        O::Mosaic {
            cell: 2,
            origin: [0; 2],
        },
        true,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        p.layers[0]
            .pixels
            .iter()
            .zip(&selected.layers[0].pixels)
            .enumerate()
            .all(|(i, (a, b))| i == 0 || a == b),
        "Pixel effect changed selection exterior",
    )?;
    selected.scene.selection_mask = Some(Default::default());
    check(
        core::prepare(&selected, 0, op, true, DEFAULT_DOCUMENT_BUDGET, |_, _| {
            Ok(())
        })? == selected.snapshot(),
        "Empty effect selection changed storage",
    )?;
    for op in [
        O::Mosaic {
            cell: 0,
            origin: [0; 2],
        },
        O::Noise {
            amount: f64::NAN,
            seed: 0,
            monochrome: false,
        },
        O::Dust {
            max_area: 4097,
            threshold: 0.5,
            target: T::Ink,
        },
        O::LineWidth { delta: 17 },
    ] {
        check(run(&d, op).is_err(), "Invalid pixel effect accepted")?;
    }
    check(
        core::prepare(&d, 0, O::defaults()[0], false, 16, |_, _| Ok(())).is_err(),
        "Insufficient pixel budget accepted",
    )?;
    let mut cancelled = false;
    check(
        core::prepare(
            &d,
            0,
            O::LineWidth { delta: 16 },
            false,
            DEFAULT_DOCUMENT_BUDGET,
            |done, _| {
                if done >= 128 {
                    cancelled = true;
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && cancelled
            && d.snapshot() == original,
        "Large width cancellation ignored",
    )?;
    let mut d = fixture()?;
    d.commit_state(mosaic)?;
    let file = root.join("pixel-effect.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Pixel recipe/native/one Undo/Redo",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&71u32.to_le_bytes());
    let fake = root.join("invalid-v71-pixels.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted pixel recipe",
    )?;
    let mut legacy = crate::detail_filter_verify::fixture()?;
    legacy.commit_state(crate::detail_filter::prepare(
        &legacy,
        0,
        crate::detail_filter::Settings {
            operation: crate::detail_filter::Operation::defaults()[0],
            selection: false,
        },
        |_, _| Ok(()),
    )?)?;
    crate::files::save(&mut legacy, &file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&71u32.to_le_bytes());
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == legacy.snapshot(),
        "Existing v71 detail recipe no longer opens",
    )?;

    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_pixel_reference_seed_components_morphology_precision_cancel_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
