use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    extended_filters::Operation as O,
    pixel_effects::{self as pixels, Operation as P},
    scene::{Point, Rect},
};
fn check(v: bool, m: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(m.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() < 1e-9)
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for y in 0..16 {
        for x in 0..16 {
            let v = (x * 8 + y * 4) as u8;
            d.layers[0].pixels[y * 16 + x] = [v, v, v, 255];
        }
    }
    d.layers[0].pixels[0] = [7, 200, 30, 0];
    d.layers[0].pixels[17][3] = 1;
    Ok(d)
}
fn run(d: &Document, o: O, selection: bool) -> Result<crate::sizing::DocumentState, String> {
    pixels::prepare(
        d,
        0,
        P::Extended(o),
        selection,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )
}
pub fn equations() -> Result<(), String> {
    let d = fixture()?;
    let at = |o: O| crate::extended_filters::sample(&d, 0, [5, 5], d.layers[0].sample_at(85), o);
    let a = at(O::Normal {
        strength: 2.,
        y_up: true,
    })?;
    let b = at(O::Normal {
        strength: 2.,
        y_up: false,
    })?;
    let len = (1. + (16f64 / 255.).powi(2) + (8f64 / 255.).powi(2)).sqrt();
    let rgb = a.rgb();
    check(
        (rgb[0] - (0.5 - 8. / 255. / len)).abs() < 1e-12
            && (rgb[1] - (0.5 + 4. / 255. / len)).abs() < 1e-12
            && (rgb[2] - (0.5 + 0.5 / len)).abs() < 1e-12,
        "Independent normal height gradient equation",
    )?;
    check(
        a.rgb()[0] == b.rgb()[0]
            && a.rgb()[2] == b.rgb()[2]
            && (a.rgb()[1] + b.rgb()[1] - 1.).abs() < 1e-12,
        "Normal Y inversion changed red/blue",
    )?;
    let mut flat = fixture()?;
    flat.layers[0].pixels.fill([80, 80, 80, 255]);
    let n = run(
        &flat,
        O::Normal {
            strength: 20.,
            y_up: true,
        },
        false,
    )?;
    check(
        n.layers[0].pixels[85] == [128, 128, 255, 255],
        "Uniform height did not produce neutral normal",
    )?;
    let e = d.layers[0].extent(16, 16);
    check(
        near(
            O::Polar {
                inverse: false,
                center: Some([8.; 2]),
                radius: 4.,
            }
            .map([10., 8.], e),
            [0., 8.],
        ) && near(
            O::Polar {
                inverse: true,
                center: Some([8.; 2]),
                radius: 4.,
            }
            .map([0., 8.], e),
            [10., 8.],
        ),
        "Polar forward/inverse coordinate oracle",
    )?;
    let sphere = O::Curved {
        strength: 1.,
        cylinder: false,
        center: Some([8.; 2]),
        radius: 4.,
    }
    .map([10., 10.], e);
    let cylinder = O::Curved {
        strength: 1.,
        cylinder: true,
        center: Some([8.; 2]),
        radius: 4.,
    }
    .map([10., 10.], e);
    check(
        sphere[1] < 10. && cylinder[1] == 10. && cylinder[0] < 10.,
        "Sphere/cylinder maps were not distinct",
    )?;
    check(
        near(
            O::Pinch {
                strength: 0.5,
                center: Some([8.; 2]),
                radius: 4.,
            }
            .map([10., 8.], e),
            [9., 8.],
        ) && near(
            O::Ripple {
                amplitude: 1.,
                period: 8.,
                center: Some([8.; 2]),
                radius: 4.,
            }
            .map([10., 8.], e),
            [10.5, 8.],
        ) && near(
            O::Zigzag {
                amplitude: 1.,
                period: 4.,
                center: Some([8.; 2]),
                radius: 4.,
            }
            .map([10., 8.], e),
            [10., 8.5],
        ),
        "Independent pinch/ripple/zigzag displacement equations",
    )?;
    check(
        at(O::Abstract {
            outline: 0.,
            levels: 5,
        })?
        .rgb()
            == [0.25; 3],
        "Abstraction color quantization equation",
    )?;
    for op in O::defaults() {
        check(op.valid(), "Invalid default finishing filter")?;
        if !op.preserves_alpha() {
            for p in [[8., 8.], [8., 12.], [12., 8.], [4., 8.], [8., 4.]] {
                check(
                    op.map(p, e).iter().all(|v| v.is_finite()),
                    "Map boundary/center was nonfinite",
                )?;
            }
        }
    }
    let mut blocks = fixture()?;
    for y in 0..16 {
        for x in 0..16 {
            blocks.layers[0].pixels[y * 16 + x] = if x < 8 { [40; 4] } else { [200; 4] };
            blocks.layers[0].pixels[y * 16 + x][3] = 255;
        }
    }
    let reduced = run(
        &blocks,
        O::Jpeg {
            block: 1.,
            ring: 0.,
        },
        false,
    )?;
    check(
        reduced.layers[0].pixels[87][0] == 120
            && reduced.layers[0].pixels[88][0] == 120
            && reduced.layers[0].pixels[85][0] == 40,
        "8px JPEG block boundary equation / interior preservation",
    )?;
    let mut ringing = fixture()?;
    ringing.layers[0].pixels.fill([130, 130, 130, 255]);
    ringing.layers[0].pixels[86] = [170, 170, 170, 255];
    let reduced_ring = run(
        &ringing,
        O::Jpeg {
            block: 0.,
            ring: 1.,
        },
        false,
    )?;
    check(
        reduced_ring.layers[0].pixels[86][0] < 170,
        "JPEG ringing control did not reduce a boundary overshoot",
    )?;
    let smooth = run(
        &blocks,
        O::Smoothing {
            radius: 1,
            edge: 0.,
            amount: 1.,
        },
        false,
    )?;
    let preserved = run(
        &blocks,
        O::Smoothing {
            radius: 1,
            edge: 1.,
            amount: 1.,
        },
        false,
    )?;
    check(
        preserved.layers[0].pixels[87][0] < smooth.layers[0].pixels[87][0],
        "Smoothing edge strength did not protect contrast",
    )?;
    let mut pulse = flat;
    pulse.layers[0].pixels.fill([0, 0, 0, 255]);
    pulse.layers[0].pixels[136] = [255; 4];
    let mild = run(&pulse, O::Blur, false)?;
    let strong = run(&pulse, O::BlurStrong, false)?;
    check(
        mild.layers[0].pixels[136][0] == 28 && strong.layers[0].pixels[136][0] == 5,
        "Separate3x3/7x7 blur equations",
    )?;
    pulse.layers[0].pixels.fill([100, 100, 100, 255]);
    pulse.layers[0].pixels[136] = [120, 120, 120, 255];
    let basic = crate::detail_filter::prepare(
        &pulse,
        0,
        crate::detail_filter::Settings {
            options: Default::default(),
            operation: crate::detail_filter::Operation::Sharpen { strength: 0.5 },
            selection: false,
        },
        |_, _| Ok(()),
    )?;
    let strong = run(&pulse, O::SharpenMore, false)?;
    check(
        strong.layers[0].pixels[136][0] > basic.layers[0].pixels[136][0],
        "Sharpen more did not exceed basic sharpening on a low-contrast impulse",
    )?;
    check(
        !O::Polar {
            inverse: false,
            center: None,
            radius: 0.,
        }
        .valid()
            && !O::Ripple {
                amplitude: 1.,
                period: 0.,
                center: None,
                radius: 8.,
            }
            .valid()
            && !O::Normal {
                strength: f64::NAN,
                y_up: true,
            }
            .valid(),
        "Invalid radius/period/nonfinite accepted",
    )?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    d.scene.selection = Some(Rect {
        min: [2.; 2],
        max: [13.; 2],
    });
    let original = d.snapshot();
    for (n, op) in O::defaults().into_iter().enumerate() {
        let counts = d.history_counts();
        let state = run(&d, op, true)?;
        check(
            d.snapshot() == original && d.history_counts() == counts,
            "Filter preview changed source",
        )?;
        for y in 0..16 {
            for x in 0..16 {
                if !(2..13).contains(&x) || !(2..13).contains(&y) {
                    check(
                        state.layers[0].pixels[y * 16 + x] == original.layers[0].pixels[y * 16 + x],
                        "Finishing filter changed outside selection",
                    )?;
                }
            }
        }
        check(
            state.color == d.color,
            "Filter changed color precision/profile",
        )?;
        d.commit_state(state)?;
        let path = root.join(format!("finishing-{n}.drawer"));
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Finishing recipe native roundtrip",
        )?;
        if d.snapshot() != original {
            check(
                d.undo() && d.snapshot() == original,
                "Finishing filter single Undo",
            )?;
        }
    }
    let zero = O::Smoothing {
        radius: 4,
        edge: 1.,
        amount: 0.,
    };
    check(
        run(&d, zero, false)? == original,
        "Smoothing amount0 was not identity",
    )?;
    for op in O::defaults() {
        if op.preserves_alpha() {
            let state = run(&d, op, false)?;
            check(
                state.layers[0].pixels[0] == original.layers[0].pixels[0]
                    && state.layers[0].pixels[17][3] == 1,
                "Transparent hidden RGB / alpha1 changed",
            )?;
        }
    }
    let saved = run(
        &d,
        O::Normal {
            strength: 2.,
            y_up: false,
        },
        false,
    )?;
    d.commit_state(saved)?;
    let file = root.join("finishing-new.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&81u32.to_le_bytes());
    let fake = root.join("finishing-fake81.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted finishing recipe",
    )?;
    let settings = crate::detail_filter::Settings {
        options: Default::default(),
        operation: crate::detail_filter::Operation::Pixels(P::Extended(O::Normal {
            strength: 2.,
            y_up: false,
        })),
        selection: true,
    };
    let mut workspace = crate::workspaces::Snapshot::default();
    workspace
        .finishing_presets
        .values
        .push(("My normal".into(), settings));
    let json = serde_json::to_vec(&workspace).map_err(|e| e.to_string())?;
    let restored: crate::workspaces::Snapshot =
        serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    check(
        restored.valid() && restored.finishing_presets == workspace.finishing_presets,
        "Local filter preset roundtrip",
    )?;
    workspace
        .finishing_presets
        .values
        .push(("My normal".into(), settings));
    check(!workspace.valid(), "Duplicate user preset accepted")?;
    let result = pixels::prepare(
        &d,
        0,
        P::Extended(O::Blur),
        false,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Err("Cancelled".into()),
    );
    check(result.is_err(), "Finishing cancellation ignored")?;
    Ok(())
}
pub fn precision_and_disk_presets(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    d.color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        ..Default::default()
    };
    let mut high: Vec<_> = d.layers[0]
        .pixels
        .iter()
        .map(|p| crate::color_precision::Sample::from8(*p).rgba16())
        .collect();
    high[0] = [0; 4];
    high[85] = [12345, 23456, 34567, 1];
    d.layers[0].pixels = high
        .iter()
        .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
        .collect();
    d.layers[0].precise = Some(high.clone());
    d.scene.selection = Some(Rect {
        min: [2.; 2],
        max: [13.; 2],
    });
    for op in O::defaults() {
        let state = run(&d, op, true)?;
        check(
            state.layers[0].precise.as_ref().unwrap()[0] == high[0]
                && state.layers[0].precise.as_ref().unwrap()[17] == high[17],
            "Native16 finishing filter changed unselected sample",
        )?;
        if op.preserves_alpha() {
            check(
                state.layers[0].precise.as_ref().unwrap()[85][3] == 1,
                "Native16 finishing filter lost integer alpha1",
            )?;
        }
    }
    let storage = crate::storage::UserStorage::at(root.join("finishing-preset-settings"))
        .map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    let settings = crate::detail_filter::Settings {
        options: Default::default(),
        operation: crate::detail_filter::Operation::Pixels(P::Extended(O::Smoothing {
            radius: 3,
            edge: 0.7,
            amount: 0.6,
        })),
        selection: true,
    };
    store
        .current
        .finishing_presets
        .values
        .push(("Own smoothing".into(), settings));
    store.save(&storage).map_err(|e| e.to_string())?;
    let restored = crate::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Missing saved filter presets")?;
    check(
        restored.current.finishing_presets == store.current.finishing_presets,
        "Disk/restart user filter presets",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    lifecycle(root)?;
    precision_and_disk_presets(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn native16_alpha_selected_samples_disk_preset_restart() {
        let root = crate::test_support::TempDir::new();
        super::precision_and_disk_presets(&root.0).unwrap();
    }
    #[test]
    fn normal_y_polar_jpeg_smoothing_distinct_blurs_invalid() {
        super::equations().unwrap();
    }
    #[test]
    fn filters_selection_alpha_presets_native_undo_cancel() {
        let root = crate::test_support::TempDir::new();
        super::lifecycle(&root.0).unwrap();
    }
}
