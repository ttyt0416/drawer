use crate::{
    color_precision::Sample,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    sizing::DocumentState,
    spatial_blur::{self as core, Mode, Settings},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 17,
            height: 17,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([0, 255, 0, 0]);
    d.layers[0].pixels[8 * 17 + 8] = [255, 0, 0, 128];
    Ok(d)
}
fn settings(mode: Mode) -> Settings {
    Settings {
        mode,
        center: [8.5, 8.5],
        length: 2.,
        radius: 4.,
        angle: if mode == Mode::Spin { 90. } else { 0. },
        ..Default::default()
    }
}
fn run(d: &Document, s: Settings) -> Result<DocumentState, String> {
    core::prepare(d, 0, s, |_, _| Ok(()))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    for mode in Mode::ALL {
        let s = settings(mode);
        let zero = Settings {
            length: 0.,
            radius: 0.,
            angle: 0.,
            ..s
        };
        check(
            run(&d, zero)? == original && run(&d, Settings { amount: 0., ..s })? == original,
            "Zero blur changed storage",
        )?;
        let p = run(&d, s)?;
        check(
            p != original
                && p.scene.last_spatial_blur == Some(s)
                && d.snapshot() == original
                && d.history_counts() == counts,
            "Blur source isolation/record",
        )?;
        check(
            p.layers[0]
                .pixels
                .iter()
                .filter(|p| p[3] > 0)
                .all(|p| p[..3] == [255, 0, 0]),
            "Blur acquired hidden green/black fringe",
        )?;
    }
    let motion = run(&d, settings(Mode::Motion))?;
    for (x, alpha) in [(7, 32), (8, 64), (9, 32)] {
        check(
            motion.layers[0].pixel(17, 17, x, 8) == [255, 0, 0, alpha],
            "Motion differs from independent trapezoidal/bilinear weights",
        )?;
    }
    let vertical = run(
        &d,
        Settings {
            angle: 90.,
            ..settings(Mode::Motion)
        },
    )?;
    check(
        vertical.layers[0].pixel(17, 17, 8, 7) == [255, 0, 0, 32]
            && vertical.layers[0].pixel(17, 17, 7, 8)[3] == 0,
        "Motion angle90",
    )?;
    let radial = run(&d, settings(Mode::Radial))?;
    check(
        radial.layers[0].pixel(17, 17, 8, 8) == [255, 0, 0, 128],
        "Radial center moved",
    )?;
    for mode in [Mode::Radial, Mode::Spin] {
        let p = run(
            &d,
            Settings {
                center: [-12., 5.],
                angle: 30.,
                ..settings(mode)
            },
        )?;
        check(
            p.layers[0].pixels.iter().filter(|p| p[3] > 0).count() > 1,
            "Outside-center blur did not spread ink",
        )?;
    }
    // Independently check the regular triangle: vertex at +X and opposite side at -R/2.
    let triangle = Settings {
        sides: 3,
        ..settings(Mode::Lens)
    };
    check(
        core::aperture_contains(triangle, [3.9, 0.])
            && !core::aperture_contains(triangle, [-2.1, 0.]),
        "Triangle aperture geometry",
    )?;
    let tri = run(&d, triangle)?;
    check(
        tri.layers[0].pixel(17, 17, 11, 8)[3] > 0 && tri.layers[0].pixel(17, 17, 5, 8)[3] == 0,
        "Triangle point light orientation",
    )?;
    for sides in 3..=8 {
        let s = Settings {
            sides,
            ..settings(Mode::Lens)
        };
        let a = run(&d, s)?;
        let b = run(
            &d,
            Settings {
                angle: 360. / sides as f64,
                ..s
            },
        )?;
        check(a.layers == b.layers, "Polygon rotation period")?;
        let round = run(&d, Settings { roundness: 1., ..s })?;
        let turned = run(
            &d,
            Settings {
                roundness: 1.,
                angle: 23.7,
                ..s
            },
        )?;
        check(
            round.layers == turned.layers && round.layers != a.layers,
            "Aperture roundness endpoints/circle orientation",
        )?;
    }
    check(
        run(
            &d,
            Settings {
                angle: 45.,
                ..triangle
            },
        )?
        .layers
            != tri.layers,
        "Triangle rotation ignored",
    )?;
    check(
        crate::gaussian::prepare(&d, 0, 4, false)?.layers != tri.layers,
        "Lens is indistinguishable from Gaussian",
    )?;
    let mut lights = fixture()?;
    lights.layers[0].pixels.fill([0, 0, 0, 255]);
    lights.layers[0].pixels[144] = [255; 4];
    let a = run(&lights, settings(Mode::Lens))?;
    let b = run(
        &lights,
        Settings {
            highlight: 3.,
            ..settings(Mode::Lens)
        },
    )?;
    check(
        b.layers[0].pixel(17, 17, 8, 8)[0] > a.layers[0].pixel(17, 17, 8, 8)[0]
            && a.layers[0]
                .pixels
                .iter()
                .zip(&b.layers[0].pixels)
                .all(|(a, b)| a[3] == b[3]),
        "Highlight gain or unchanged coverage",
    )?;
    lights.layers[0].placement = Some(PlacedRaster {
        width: 17,
        height: 17,
        offset: [-20, -20],
    });
    check(
        run(&lights, settings(Mode::Lens))?.layers[0]
            .extent(17, 17)
            .offset[0]
            < -20,
        "Off-canvas light discarded",
    )?;
    let mut high = fixture()?;
    high.color = Default::default();
    let mut values = vec![[0; 4]; 289];
    values[144] = [10000, 50000, 65535, 10];
    high.layers[0].pixels = values.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    high.layers[0].precise = Some(values);
    let p = run(&high, settings(Mode::Motion))?;
    let v = p.layers[0].sample(17, 17, 8, 8).rgba16();
    check(
        v == [10000, 50000, 65535, 5],
        "Native16 low-alpha blur lost precision",
    )?;
    d.scene.selection = Some(crate::scene::Rect {
        min: [8., 8.],
        max: [9., 9.],
    });
    let selected = run(
        &d,
        Settings {
            selection: true,
            ..settings(Mode::Lens)
        },
    )?;
    for y in 0..17 {
        for x in 0..17 {
            if x != 8 || y != 8 {
                check(
                    selected.layers[0].pixel(17, 17, x, y) == d.layers[0].pixel(17, 17, x, y),
                    "Blur changed selection exterior raw values",
                )?;
            }
        }
    }
    d.scene.selection_mask = Some(Default::default());
    check(
        run(
            &d,
            Settings {
                selection: true,
                ..settings(Mode::Lens)
            },
        )? == d.snapshot(),
        "Empty selection created edit",
    )?;
    d.scene.selection = None;
    d.scene.selection_mask = None;
    d.scene
        .clipping
        .alpha
        .insert(crate::layer_properties::raster(&d, 0));
    let p = run(&d, settings(Mode::Motion))?;
    check(
        p.layers[0].placement == d.layers[0].placement
            && p.layers[0]
                .pixels
                .iter()
                .zip(&d.layers[0].pixels)
                .all(|(a, b)| a[3] == b[3]),
        "Alpha lock changed alpha/extent",
    )?;
    d = fixture()?;
    let mut cancelled = false;
    check(
        core::prepare(&d, 0, settings(Mode::Lens), |done, _| {
            if done >= 128 {
                cancelled = true;
                Err("cancel".into())
            } else {
                Ok(())
            }
        })
        .is_err()
            && cancelled
            && d.snapshot() == original,
        "Processing cancellation changed source or did not respond",
    )?;
    let mut calls = 0;
    check(
        core::prepare(
            &d,
            0,
            Settings {
                radius: 64.,
                ..settings(Mode::Lens)
            },
            |_, _| {
                calls += 1;
                if calls == 4 {
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err()
            && calls == 4,
        "Large aperture kernel cancellation",
    )?;
    for s in [
        Settings {
            sides: 2,
            ..settings(Mode::Lens)
        },
        Settings {
            radius: f64::NAN,
            ..settings(Mode::Lens)
        },
        Settings {
            center: [1e7, 0.],
            ..settings(Mode::Radial)
        },
        Settings {
            angle: 361.,
            ..settings(Mode::Spin)
        },
    ] {
        check(run(&d, s).is_err(), "Invalid blur accepted")?;
    }
    check(
        run(
            &d,
            Settings {
                radius: 64.,
                ..settings(Mode::Lens)
            },
        )
        .is_err(),
        "Oversized processing accepted",
    )?;
    d.layers[0].locked = true;
    check(
        run(&d, settings(Mode::Motion)).is_err(),
        "Locked target accepted",
    )?;
    d.layers[0].locked = false;
    d.commit_state(motion)?;
    let file = root.join("spatial-blur.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Blur native/one Undo/Redo",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&69u32.to_le_bytes());
    let fake = root.join("invalid-v69-blur.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted blur settings",
    )?;
    d.undo();
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&69u32.to_le_bytes());
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == original,
        "Compatible v69 no-record file rejected",
    )?;
    let storage =
        crate::storage::UserStorage::at(root.join("blur-presets")).map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    for mode in Mode::ALL {
        store.current.spatial_blur.values[mode.index()] = Some(settings(mode));
    }
    store.save(&storage).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Store::load(&storage)
            .map_err(|e| e.to_string())?
            .unwrap()
            .current
            .spatial_blur
            == store.current.spatial_blur,
        "Local blur preset roundtrip",
    )?;
    let json =
        serde_json::to_value(crate::workspaces::Snapshot::default()).map_err(|e| e.to_string())?;
    let old: crate::workspaces::Snapshot =
        serde_json::from_value(json).map_err(|e| e.to_string())?;
    check(
        old.spatial_blur.is_default() && old.valid(),
        "Old preferences did not get defaults",
    )?;
    store.current.spatial_blur.values[0] = Some(settings(Mode::Lens));
    check(
        store.save(&storage).is_err(),
        "Invalid per-mode preset saved",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn trajectory_aperture_native_precision_cancel_and_presets() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
