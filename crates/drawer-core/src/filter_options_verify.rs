use crate::{
    detail_filter::{self, Operation as D, Settings},
    document::DEFAULT_DOCUMENT_BUDGET,
    extended_filters::Operation as E,
    filter_options::*,
    pixel_effects::Operation as P,
    stylized_effects::Operation as S,
    warp_filters::Operation as W,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn fixture() -> Result<crate::document::Document, String> {
    let mut d = crate::spatial_blur_verify::fixture()?;
    for (y, row) in d.layers[0]
        .pixels
        .as_chunks_mut::<17>()
        .0
        .iter_mut()
        .enumerate()
    {
        for (x, p) in row.iter_mut().enumerate() {
            *p = [(x * 15) as u8, (y * 15) as u8, ((x + y) * 7) as u8, 255];
        }
    }
    Ok(d)
}
fn run(
    d: &crate::document::Document,
    op: P,
    options: Options,
    selection: bool,
) -> Result<crate::sizing::DocumentState, String> {
    detail_filter::prepare(
        d,
        0,
        Settings {
            operation: D::Pixels(op),
            options,
            selection,
        },
        |_, _| Ok(()),
    )
}
pub fn equations() -> Result<(), String> {
    let d = fixture()?;
    for (boundary, expected) in [
        (Boundary::Transparent, [0; 4]),
        (Boundary::Edge, [0, 0, 0, 255]),
        (Boundary::Wrap, [225, 225, 210, 255]),
        (Boundary::Mirror, [15, 15, 14, 255]),
    ] {
        let s = Sampler {
            document: &d,
            layer: 0,
            boundary,
        };
        check(
            s.at(-2, -2).rgba8() == expected,
            "Document-corner boundary marker",
        )?;
        check(
            s.at(34, 34).rgba8()
                == match boundary {
                    Boundary::Transparent => [0; 4],
                    Boundary::Edge => [240, 240, 224, 255],
                    Boundary::Wrap | Boundary::Mirror => [0, 0, 0, 255],
                },
            "Far positive boundary marker",
        )?;
    }
    for boundary in [Boundary::Edge, Boundary::Wrap, Boundary::Mirror] {
        let s = Sampler {
            document: &d,
            layer: 0,
            boundary,
        };
        let x = 17_000_000_002_f64;
        check(
            s.bilinear([x, 0.]).rgba8()
                == if boundary == Boundary::Edge {
                    [240, 0, 112, 255]
                } else {
                    [30, 0, 14, 255]
                },
            "Far finite bilinear boundary",
        )?;
    }
    check(
        Waveform::Sine.value(0.).abs() < 1e-12
            && Waveform::Triangle.value(0.).abs() < 1e-12
            && Waveform::Rectangular.value(0.) == 1.,
        "Wave origin",
    )?;
    check(
        (Waveform::Sine.value(0.125) - 0.5_f64.sqrt()).abs() < 1e-12
            && (Waveform::Triangle.value(0.125) - 0.5).abs() < 1e-12
            && Waveform::Rectangular.value(0.625) == -1.,
        "Wave quarter slopes",
    )?;
    let op = W::Wave {
        amplitude: [4., 0.],
        period: [8.; 2],
        phase: 0.,
    };
    for waveform in [Waveform::Sine, Waveform::Triangle, Waveform::Rectangular] {
        let o = Options {
            waveform,
            boundary: Boundary::Wrap,
            ..Default::default()
        };
        let state = run(&d, P::Warp(op), o, false)?;
        let expected = Sampler {
            document: &d,
            layer: 0,
            boundary: Boundary::Wrap,
        }
        .bilinear([4. * waveform.value(0.5 / 8.), 0.]);
        check(
            state.layers[0].pixels[0] == expected.rgba8(),
            "Waveform not used in native raster",
        )?;
    }
    let mut values = Vec::new();
    for mode in [
        Abstraction::ColorAndLines,
        Abstraction::ColorOnly,
        Abstraction::LinesOnly,
    ] {
        values.push(
            run(
                &d,
                P::Extended(E::Abstract {
                    outline: 2.,
                    levels: 4,
                }),
                Options {
                    abstraction: mode,
                    ..Default::default()
                },
                false,
            )?
            .layers[0]
                .pixels[24],
        );
    }
    check(
        values[0] != values[1] && values[1] != values[2] && values[2][0] == values[2][1],
        "Abstraction modes coincide",
    )?;
    let film = S::Film {
        strength: 1.,
        contrast: 0.,
        fade: 0.,
        temperature: 0.,
        grain: 0.,
        seed: 7,
    };
    let mut films = Vec::new();
    for mode in [
        Film::None,
        Film::Sepia,
        Film::Slant,
        Film::All,
        Film::SepiaSlant,
        Film::SepiaAll,
    ] {
        films.push(
            run(
                &d,
                P::Style(film),
                Options {
                    film: mode,
                    ..Default::default()
                },
                false,
            )?
            .layers[0]
                .pixels[253],
        );
    }
    check(
        films[0] == d.layers[0].pixels[253]
            && films
                .iter()
                .enumerate()
                .all(|(i, p)| !films[..i].contains(p)),
        "Film presets/zero base tone",
    )?;
    let lateral = Options {
        aberration: Aberration::Lateral,
        angle: 90.,
        boundary: Boundary::Edge,
        ..Default::default()
    };
    let style = P::Style(S::Aberration {
        offset: [2, 0],
        strength: 1.,
    });
    let state = run(&d, style, lateral, false)?;
    check(
        state.layers[0].sample(17, 17, 8, 8).rgba8()[0] == 120
            && state.layers[0].sample(17, 17, 8, 8).rgba8()[2] == 126,
        "Lateral ninety-degree channel displacement",
    )?;
    let a = run(
        &d,
        style,
        Options {
            aberration: Aberration::Radial,
            center: Some([8.5; 2]),
            ..Default::default()
        },
        false,
    )?;
    check(
        a.layers[0].sample(17, 17, 8, 8).rgba8() == d.layers[0].pixels[8 * 17 + 8]
            && a.layers[0].sample(17, 17, 14, 8).rgba8() != d.layers[0].pixels[8 * 17 + 14],
        "Radial center and peripheral displacement",
    )?;
    let mut line = crate::spatial_blur_verify::fixture()?;
    line.layers[0].pixels.fill([0; 4]);
    for y in 2..15 {
        line.layers[0].pixels[y * 17 + 8] = [10, 20, 30, 255];
    }
    let erased = run(&line, P::LineWidth { delta: -2 }, Default::default(), false)?;
    let kept = run(
        &line,
        P::LineWidth { delta: -2 },
        Options {
            minimum_line: true,
            ..Default::default()
        },
        false,
    )?;
    check(
        erased.layers[0].pixels.iter().all(|p| p[3] == 0)
            && kept.layers[0].pixels[8 * 17 + 8][3] == 255,
        "At least one pixel failed on isolated line",
    )?;
    let sphere = run(
        &d,
        P::Extended(E::Polar {
            inverse: false,
            center: None,
            radius: 8.,
        }),
        Options {
            spherize: true,
            ..Default::default()
        },
        false,
    )?;
    let polar = run(
        &d,
        P::Extended(E::Polar {
            inverse: false,
            center: None,
            radius: 8.,
        }),
        Default::default(),
        false,
    )?;
    check(
        sphere.layers[0].pixels[8 * 17 + 8] == d.layers[0].pixels[8 * 17 + 8]
            && sphere.layers[0] != polar.layers[0],
        "Spherize center/polar separation",
    )?;
    for invalid in [
        Options {
            shape: 0.,
            ..Default::default()
        },
        Options {
            shape: f64::INFINITY,
            ..Default::default()
        },
        Options {
            angle: f64::NAN,
            ..Default::default()
        },
        Options {
            center: Some([1e7, 0.]),
            ..Default::default()
        },
    ] {
        check(
            run(&d, P::Extended(E::Blur), invalid, false).is_err(),
            "Invalid options accepted",
        )?;
    }
    Ok(())
}
pub fn directions() -> Result<(), String> {
    use crate::spatial_blur::{self as blur, Direction, Kernel, Mode, Settings};
    let mut d = crate::spatial_blur_verify::fixture()?;
    d.layers[0].pixels.fill([0; 4]);
    d.layers[0].pixels[8 * 17 + 12] = [255, 0, 0, 255];
    let centroid = |s: &crate::sizing::DocumentState| {
        let l = &s.layers[0];
        let e = l.extent(17, 17);
        let mut sum = [0.; 3];
        for (i, p) in l.pixels.iter().enumerate() {
            let a = p[3] as f64;
            sum[0] += (e.offset[0] as f64 + (i % e.width as usize) as f64) * a;
            sum[1] += (e.offset[1] as f64 + (i / e.width as usize) as f64) * a;
            sum[2] += a;
        }
        [sum[0] / sum[2], sum[1] / sum[2]]
    };
    for mode in [Mode::Motion, Mode::Radial, Mode::Spin] {
        let s = Settings {
            mode,
            center: [8.5; 2],
            length: 4.,
            radius: 4.,
            angle: if mode == Mode::Spin { 60. } else { 0. },
            ..Default::default()
        };
        let forward = blur::prepare(
            &d,
            0,
            Settings {
                direction: Direction::Forward,
                ..s
            },
            |_, _| Ok(()),
        )?;
        let backward = blur::prepare(
            &d,
            0,
            Settings {
                direction: Direction::Backward,
                ..s
            },
            |_, _| Ok(()),
        )?;
        let a = centroid(&forward);
        let b = centroid(&backward);
        check(
            if mode == Mode::Spin {
                a[1] > 8. && b[1] < 8.
            } else {
                a[0] > 12. && b[0] < 12.
            },
            "One-sided blur direction reversed or ignored",
        )?;
        let box_state = blur::prepare(&d, 0, s, |_, _| Ok(()))?;
        let smooth = blur::prepare(
            &d,
            0,
            Settings {
                kernel: Kernel::Smooth,
                ..s
            },
            |_, _| Ok(()),
        )?;
        check(
            box_state.layers[0] != smooth.layers[0],
            "Box and smooth kernels coincide",
        )?;
    }
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    d.scene.selection = Some(crate::scene::Rect {
        min: [0., 0.],
        max: [3., 3.],
    });
    let original = d.snapshot();
    for boundary in Boundary::ALL {
        let settings = Settings {
            operation: D::Pixels(P::Warp(W::Wave {
                amplitude: [20., 0.],
                period: [8.; 2],
                phase: 90.,
            })),
            selection: true,
            options: Options {
                boundary,
                waveform: Waveform::Rectangular,
                ..Default::default()
            },
        };
        let prepared = detail_filter::prepare(&d, 0, settings, |_, _| Ok(()))?;
        check(
            d.snapshot() == original
                && prepared.layers[0]
                    .pixels
                    .iter()
                    .enumerate()
                    .all(|(i, p)| i % 17 < 3 && i / 17 < 3 || *p == d.layers[0].pixels[i]),
            "Selection exterior/source changed",
        )?;
        check(
            detail_filter::prepare(&d, 0, settings, |_, _| Err("cancel".into())).is_err()
                && d.snapshot() == original,
            "Cancel changed source",
        )?;
        check(
            detail_filter::prepare(&d, 0, settings, |_, _| Ok(()))? == prepared,
            "Filter nondeterminism",
        )?;
    }
    let mut high = fixture()?;
    high.color.depth = crate::color_precision::Depth::Sixteen;
    let values: Vec<_> = high.layers[0]
        .pixels
        .iter()
        .enumerate()
        .map(|(i, p)| {
            [
                p[0] as u16 * 257 + 1,
                p[1] as u16 * 257 + 2,
                p[2] as u16 * 257 + 3,
                if i == 18 { 1 } else { 65535 },
            ]
        })
        .collect();
    high.layers[0].pixels = values
        .iter()
        .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
        .collect();
    high.layers[0].precise = Some(values.clone());
    high.scene.selection = Some(crate::scene::Rect {
        min: [0., 0.],
        max: [3., 3.],
    });
    for boundary in Boundary::ALL {
        let state = run(
            &high,
            P::Extended(E::Smoothing {
                radius: 2,
                edge: 0.,
                amount: 1.,
            }),
            Options {
                boundary,
                ..Default::default()
            },
            true,
        )?;
        let actual = state.layers[0]
            .precise
            .as_ref()
            .ok_or("Missing precise filter output")?;
        check(
            actual[18][3] == 1
                && actual
                    .iter()
                    .enumerate()
                    .all(|(i, p)| i % 17 < 3 && i / 17 < 3 || *p == values[i]),
            "Native16 edge/alpha1 or selection exterior",
        )?;
    }
    let options = Options {
        boundary: Boundary::Wrap,
        area: Area::EntireSelection,
        shape: 2.,
        ..Default::default()
    };
    let a = run(
        &d,
        P::Extended(E::Pinch {
            strength: 0.5,
            center: Some([100.; 2]),
            radius: 64.,
        }),
        options,
        true,
    )?;
    let b = run(
        &d,
        P::Extended(E::Pinch {
            strength: 0.5,
            center: Some([-100.; 2]),
            radius: 16.,
        }),
        Options {
            shape: 0.5,
            ..options
        },
        true,
    )?;
    check(
        a.layers[0] == b.layers[0],
        "Entire selection still uses specified center/radius/shape",
    )?;
    let op = P::Warp(W::Wave {
        amplitude: [2., 0.],
        period: [8.; 2],
        phase: 0.,
    });
    let settings = Settings {
        operation: D::Pixels(op),
        selection: true,
        options: Options {
            boundary: Boundary::Mirror,
            waveform: Waveform::Triangle,
            ..Default::default()
        },
    };
    let before = d.history_counts();
    d.commit_state(detail_filter::prepare(&d, 0, settings, |_, _| Ok(()))?)?;
    check(
        d.history_counts().0 == before.0 + 1,
        "Apply did not add one Undo",
    )?;
    let applied = d.snapshot();
    d.undo();
    check(d.snapshot() == original, "Undo filter options")?;
    d.redo();
    check(d.snapshot() == applied, "Redo filter options")?;
    let path = root.join("filter-options.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let opened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        opened.snapshot() == d.snapshot(),
        "Native options roundtrip",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&82u32.to_le_bytes());
    let fake = root.join("filter-options-fake82.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted new options",
    )?;
    let storage = crate::storage::UserStorage::at(root.join("filter-options-preset"))
        .map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    store
        .current
        .finishing_presets
        .values
        .push(("Triangle mirror".into(), settings));
    store.save(&storage).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Missing options preset")?;
    check(
        loaded.current.finishing_presets == store.current.finishing_presets,
        "Options preset restart",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    directions()?;
    lifecycle(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn boundary_wave_film_abstraction_aberration_minimum_line() {
        super::equations().unwrap();
    }
    #[test]
    fn motion_radial_spin_directions_and_kernels() {
        super::directions().unwrap();
    }
    #[test]
    fn selection_native_presets_cancel_undo() {
        let root = crate::test_support::TempDir::new();
        super::lifecycle(&root.0).unwrap();
    }
}
