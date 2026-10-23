use crate::{
    cleanup_filters::{self as cleanup, Dust, Mode, Operation as C, Selection, SelectionKind},
    detail_filter::{self, Operation as D, Settings},
    document::DEFAULT_DOCUMENT_BUDGET,
    pixel_effects::Operation as P,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture(holes: bool) -> Result<crate::document::Document, String> {
    let mut d = crate::spatial_blur_verify::fixture()?;
    if holes {
        d.layers[0].pixels.fill([40, 80, 120, 255]);
        d.layers[0].pixels[8 * 17 + 8] = [255, 0, 255, 0];
        d.layers[0].pixels[8 * 17 + 9] = [255, 0, 0, 128];
        d.layers[0].pixels[0] = [0; 4];
    } else {
        d.layers[0].pixels.fill([11, 22, 33, 0]);
        d.layers[0].pixels[4 * 17 + 10] = [30, 60, 90, 255];
        d.layers[0].pixels[8 * 17 + 10] = [30, 60, 90, 255];
        d.layers[0].pixels[8 * 17 + 11] = [30, 60, 90, 255];
        for y in 3..14 {
            d.layers[0].pixels[y * 17 + 4] = [5, 5, 5, 255];
        }
        d.layers[0].pixels[17 * 2] = [0, 0, 0, 255];
    }
    Ok(d)
}
fn settings(op: C) -> Settings {
    Settings {
        operation: D::Pixels(P::Cleanup(op)),
        selection: true,
        options: Default::default(),
    }
}
fn run(d: &crate::document::Document, op: C) -> Result<crate::sizing::DocumentState, String> {
    detail_filter::prepare(d, 0, settings(op), |_, _| Ok(()))
}
pub fn modes() -> Result<(), String> {
    let d = fixture(false)?;
    let original = d.snapshot();
    let dust = |mode, max_area| {
        C::Dust(Dust {
            mode,
            max_area,
            ..Default::default()
        })
    };
    let a = run(&d, dust(Mode::Transparency, 1))?;
    check(
        a.layers[0].pixels[4 * 17 + 10][3] == 0
            && a.layers[0].pixels[8 * 17 + 10] == d.layers[0].pixels[8 * 17 + 10]
            && a.layers[0].pixels[6 * 17 + 4] == d.layers[0].pixels[6 * 17 + 4]
            && a.layers[0].pixels[34] == d.layers[0].pixels[34],
        "Transparency area1/2, long line or edge protection",
    )?;
    check(
        run(&d, dust(Mode::Transparency, 2))?.layers[0].pixels[8 * 17 + 10][3] == 0,
        "Dust threshold area2",
    )?;
    check(
        run(&d, dust(Mode::WhiteBackground, 4))? == original,
        "White mode repaired a transparent background",
    )?;
    let mut diagonal = fixture(false)?;
    diagonal.layers[0].pixels.fill([0; 4]);
    for n in 3..14 {
        diagonal.layers[0].pixels[n * 17 + n] = [0, 0, 0, 255];
    }
    check(
        run(&diagonal, dust(Mode::Transparency, 4))? == diagonal.snapshot(),
        "Diagonal continuous line treated as dust points",
    )?;
    let mut white = fixture(false)?;
    for p in &mut white.layers[0].pixels {
        if p[3] == 0 {
            *p = [255; 4];
        }
    }
    let removed = run(&white, dust(Mode::WhiteBackground, 2))?;
    check(
        removed.layers[0].pixels[4 * 17 + 10] == [255; 4]
            && removed.layers[0].pixels[8 * 17 + 11] == [255; 4]
            && removed.layers[0].pixels[6 * 17 + 4] == white.layers[0].pixels[6 * 17 + 4],
        "White dust fill or long line protection",
    )?;
    check(
        run(&white, dust(Mode::Transparency, 4))?.layers == white.layers,
        "Transparency mode erased white paper",
    )?;
    let holes = fixture(true)?;
    let surround = run(&holes, dust(Mode::Surrounding, 2))?;
    let foreground = run(
        &holes,
        C::Dust(Dust {
            mode: Mode::Foreground,
            max_area: 2,
            foreground: [0.25, 0.5, 0.75, 1.],
            ..Default::default()
        }),
    )?;
    check(
        surround.layers[0].pixels[8 * 17 + 8] == [40, 80, 120, 255]
            && surround.layers[0].pixels[8 * 17 + 9] == [40, 80, 120, 255]
            && surround.layers[0].pixels[0] == holes.layers[0].pixels[0],
        "Surrounding fill/partial alpha/border hole",
    )?;
    check(
        foreground.layers[0].pixels[8 * 17 + 8] == [64, 128, 191, 255]
            && foreground.layers[0].pixels[8 * 17 + 9] == [64, 128, 191, 255],
        "Foreground fill color",
    )?;
    check(
        run(&holes, dust(Mode::Surrounding, 1))? == holes.snapshot(),
        "Connected semi-transparent hole size",
    )?;
    for mode in Mode::ALL {
        check(run(&d, dust(mode, 0))? == original, "Area0 changed pixels")?;
    }
    let mut partial = fixture(false)?;
    partial.scene.selection = Some(crate::scene::Rect {
        min: [10., 8.],
        max: [11., 9.],
    });
    let clipped = run(&partial, dust(Mode::Transparency, 2))?;
    check(
        clipped.layers[0].pixels[8 * 17 + 10][3] == 0
            && clipped.layers[0].pixels[8 * 17 + 11] == partial.layers[0].pixels[8 * 17 + 11]
            && clipped.layers[0].pixels[4 * 17 + 10] == partial.layers[0].pixels[4 * 17 + 10],
        "Partial selection changed exterior",
    )?;
    check(
        run(&partial, dust(Mode::Transparency, 1))? == partial.snapshot(),
        "Selection was treated as input component boundary",
    )?;
    partial.scene.selection_mask = Some(Default::default());
    check(
        run(&partial, dust(Mode::Transparency, 2))? == partial.snapshot(),
        "Empty cleanup selection",
    )?;
    partial.scene.selection_mask = None;
    partial.scene.selection = Some(crate::scene::Rect {
        min: [-10.; 2],
        max: [-1.; 2],
    });
    check(
        run(&partial, dust(Mode::Transparency, 2))? == partial.snapshot(),
        "Outside cleanup selection",
    )?;
    let invalid = C::Dust(Dust {
        threshold: f64::NAN,
        ..Default::default()
    });
    check(
        run(&d, invalid).is_err()
            && run(
                &d,
                C::Dust(Dust {
                    max_area: 4097,
                    ..Default::default()
                }),
            )
            .is_err()
            && run(
                &d,
                C::Dust(Dust {
                    foreground: [2.; 4],
                    ..Default::default()
                }),
            )
            .is_err(),
        "Invalid cleanup input",
    )?;
    Ok(())
}
pub fn tone_selection() -> Result<(), String> {
    let mut d = fixture(true)?;
    d.layers[0].pixels.fill([255; 4]);
    for y in (2..15).step_by(2) {
        for x in (2..15).step_by(2) {
            d.layers[0].pixels[y * 17 + x] = [0, 0, 0, 255];
        }
    }
    let reduced = run(
        &d,
        C::Descreen {
            radius: 1,
            amount: 1.,
            line_threshold: 0.02,
            coverage: false,
        },
    )?;
    check(
        reduced.layers[0].pixels[8 * 17 + 8][0] > 100
            && reduced.layers[0].pixels[8 * 17 + 9][0] < 255,
        "Tone dots did not average into gray",
    )?;
    d.layers[0].pixels.fill([255; 4]);
    for y in 0..17 {
        d.layers[0].pixels[y * 17 + 8] = [0, 0, 0, 255];
    }
    let line = run(
        &d,
        C::Descreen {
            radius: 1,
            amount: 1.,
            line_threshold: 0.02,
            coverage: false,
        },
    )?;
    check(
        (0..17).all(|y| line.layers[0].pixels[y * 17 + 8] == [0, 0, 0, 255]),
        "Descreen erased continuous dark line endpoints",
    )?;
    let mut sparse = fixture(false)?;
    sparse.layers[0].pixels.fill([11, 22, 33, 0]);
    sparse.layers[0].pixels[144] = [0, 0, 0, 255];
    let alpha = run(
        &sparse,
        C::Descreen {
            radius: 1,
            amount: 1.,
            line_threshold: 0.02,
            coverage: true,
        },
    )?;
    check(
        alpha.layers[0].pixels[144] == [0, 0, 0, 28]
            && alpha.layers[0].pixels[145] == [0, 0, 0, 28]
            && alpha.layers[0].pixels[0] == sparse.layers[0].pixels[0],
        "Transparent tone coverage or hidden-color fringe",
    )?;
    let mut color = fixture(false)?;
    color.layers[0].pixels[4 * 17 + 10] = [255, 0, 0, 128];
    let gray = run(&color, C::Gray)?;
    check(
        gray.layers[0].pixels[4 * 17 + 10] == [54, 54, 54, 128]
            && gray.layers[0].pixels[0] == color.layers[0].pixels[0],
        "Grayscale luminance/alpha/hidden RGB",
    )?;
    for kind in SelectionKind::ALL {
        let d = fixture(kind == SelectionKind::Hole)?;
        let original = d.snapshot();
        let state = cleanup::select(
            &d,
            0,
            Selection {
                kind,
                max_area: 2,
                ..Default::default()
            },
            DEFAULT_DOCUMENT_BUDGET,
            |_, _| Ok(()),
        )?;
        check(
            state.layers == original.layers && d.snapshot() == original,
            "Selection tool modified source pixels",
        )?;
        let point = if kind == SelectionKind::Hole {
            [8.5, 8.5]
        } else {
            [10.5, 4.5]
        };
        let expected = if kind == SelectionKind::WhiteBackground {
            0
        } else {
            255
        };
        check(
            state.scene.selection_coverage(point) == expected,
            "Dust/hole selection classification",
        )?;
    }
    let mut white = fixture(false)?;
    for p in &mut white.layers[0].pixels {
        if p[3] == 0 {
            *p = [255; 4];
        }
    }
    let selected = cleanup::select(
        &white,
        0,
        Selection {
            kind: SelectionKind::WhiteBackground,
            max_area: 1,
            ..Default::default()
        },
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        selected.scene.selection_coverage([10.5, 4.5]) == 255
            && selected.scene.selection_coverage([10.5, 8.5]) == 0,
        "White dust selection size",
    )?;
    let task = crate::batch_filters::Task::Detail(Settings {
        operation: D::SelectCleanup(Default::default()),
        selection: true,
        options: Default::default(),
    });
    check(!task.valid(), "Selection-only tool accepted by pixel batch")?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture(true)?;
    d.color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        ..Default::default()
    };
    let mut high: Vec<_> = d.layers[0]
        .pixels
        .iter()
        .map(|p| crate::color_precision::Sample::from8(*p).rgba16())
        .collect();
    high[8 * 17 + 8] = [12345, 23456, 34567, 1];
    d.layers[0].pixels = high
        .iter()
        .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
        .collect();
    d.layers[0].precise = Some(high.clone());
    d.scene.selection = Some(crate::scene::Rect {
        min: [8., 8.],
        max: [10., 9.],
    });
    let original = d.snapshot();
    let op = C::Dust(Dust {
        mode: Mode::Surrounding,
        max_area: 2,
        ..Default::default()
    });
    let settings = settings(op);
    check(
        detail_filter::prepare(&d, 0, settings, |_, _| Err("cancel".into())).is_err()
            && d.snapshot() == original,
        "Cleanup cancellation changed original",
    )?;
    let prepared = run(&d, op)?;
    let p = prepared.layers[0]
        .precise
        .as_ref()
        .ok_or("Missing native16 cleanup")?;
    check(
        p[8 * 17 + 8] == [10280, 20560, 30840, 65535]
            && p.iter()
                .enumerate()
                .all(|(i, p)| i == 144 || i == 145 || *p == high[i]),
        "Native16 repair/exterior",
    )?;
    let count = d.history_counts();
    d.commit_state(prepared)?;
    check(d.history_counts().0 == count.0 + 1, "Cleanup not one Undo")?;
    let applied = d.snapshot();
    d.undo();
    check(d.snapshot() == original, "Cleanup Undo")?;
    d.redo();
    check(d.snapshot() == applied, "Cleanup Redo")?;
    let file = root.join("cleanup.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Cleanup native reopen",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&83u32.to_le_bytes());
    let fake = root.join("cleanup-fake83.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native accepted cleanup",
    )?;
    let storage =
        crate::storage::UserStorage::at(root.join("cleanup-preset")).map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    store
        .current
        .finishing_presets
        .values
        .push(("Repair holes".into(), settings));
    store.save(&storage).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Store::load(&storage)
            .map_err(|e| e.to_string())?
            .ok_or("Missing cleanup preset")?
            .current
            .finishing_presets
            == store.current.finishing_presets,
        "Cleanup preset restart",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    modes()?;
    tone_selection()?;
    lifecycle(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn dust_modes_size_lines_edges_partial_holes_and_scope() {
        super::modes().unwrap();
    }
    #[test]
    fn tone_gray_and_separate_selection_tools() {
        super::tone_selection().unwrap();
    }
    #[test]
    fn native16_cancel_undo_native_header_and_disk_preset() {
        let root = crate::test_support::TempDir::new();
        super::lifecycle(&root.0).unwrap();
    }
}
