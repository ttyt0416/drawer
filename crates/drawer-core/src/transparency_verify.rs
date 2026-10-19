//! Independent strict/loose transparency fixtures shared by fill and wand consumers.
use crate::{
    bucket::{self, Mode, Reference, Region, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    selection_edit::{self, Combine},
};
fn make(w: u32, h: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut opaque = make(3, 1)?;
    opaque.layers[0].pixels = vec![[40, 50, 60, 255], [40, 50, 60, 127], [41, 50, 60, 255]];
    for mode in [Mode::Strict, Mode::CloseGaps] {
        let settings = Settings {
            mode,
            gap: 0,
            antialias: false,
            ..Default::default()
        };
        let r = Region::build(&opaque, 0, [0.5, 0.5], settings)?;
        check(
            r.coverage(1, 0) == 255 && r.coverage(2, 0) == 0,
            "RGB tolerance incorrectly compared opaque alpha",
        )?;
        check(
            Region::build(
                &opaque,
                0,
                [0.5, 0.5],
                Settings {
                    color_tolerance: 1.0 / 255.0,
                    ..settings
                },
            )?
            .coverage(2, 0)
                == 255,
            "Independent RGB threshold",
        )?;
        let all = Region::build_with_connectivity(
            &opaque,
            0,
            [0.5, 0.5],
            Settings {
                alpha_tolerance: 127.0 / 255.0,
                color_tolerance: 1.0 / 255.0,
                ..settings
            },
            false,
        )?;
        check(
            all.coverage(1, 0) == 0 && all.coverage(2, 0) == 255,
            "Alpha classification was coupled to RGB/connectedness",
        )?;
    }
    let mut d = make(5, 1)?;
    d.layers[0].pixels = vec![
        [70, 80, 90, 0],
        [100, 200, 30, 127],
        [250, 50, 70, 128],
        [0, 0, 0, 129],
        [0, 0, 0, 255],
    ];
    for mode in [Mode::Strict, Mode::CloseGaps] {
        for threshold in [
            0.0,
            127.0 / 255.0 - 1e-9,
            127.0 / 255.0,
            127.0 / 255.0 + 1e-9,
            128.0 / 255.0,
            129.0 / 255.0,
        ] {
            for color in [0.0, 1.0] {
                let settings = Settings {
                    mode,
                    gap: 0,
                    alpha_tolerance: threshold,
                    color_tolerance: color,
                    antialias: false,
                    ..Default::default()
                };
                let before = d.snapshot();
                let r = Region::build(&d, 0, [0.5, 0.5], settings)?;
                let wand = selection_edit::wand(
                    &d,
                    0,
                    [0.5, 0.5],
                    selection_edit::Settings {
                        region: settings,
                        ..Default::default()
                    },
                )?;
                let fill = bucket::prepare(
                    &d,
                    0,
                    Some([0.5, 0.5]),
                    [200, 40, 20, 255],
                    false,
                    false,
                    settings,
                )?;
                for x in 0..5 {
                    let wanted =
                        u8::from(d.layers[0].pixels[x][3] as f64 / 255.0 <= threshold) * 255;
                    check(
                        r.coverage(x as u32, 0) == wanted
                            && wand.selection_coverage([x as f64 + 0.5, 0.5]) == wanted,
                        "Strict/loose alpha threshold before/equal/after",
                    )?;
                    check(
                        (fill.layers[0].pixels[x] == [200, 40, 20, 255]) == (wanted > 0),
                        "Fill/wand alpha domain mismatch",
                    )?;
                }
                check(
                    d.snapshot() == before,
                    "Transparency preparation edited source",
                )?;
            }
        }
    }
    for gap in [0usize, 1, 3] {
        let mut ring = make(7, 7)?;
        for v in 1..6 {
            for (x, y) in [(v, 1), (v, 5), (1, v), (5, v)] {
                ring.layers[0].pixels[y * 7 + x] = [0, 0, 0, 255];
            }
        }
        for x in 3 - gap / 2..3 - gap / 2 + gap {
            ring.layers[0].pixels[7 + x] = [0; 4];
        }
        for mode in [Mode::Strict, Mode::CloseGaps] {
            let settings = Settings {
                mode,
                gap: 1,
                antialias: false,
                ..Default::default()
            };
            let spill = gap > 0 && (mode == Mode::Strict || gap > 1);
            let before = ring.snapshot();
            let r = Region::build(&ring, 0, [3.5, 3.5], settings)?;
            let wand = selection_edit::wand(
                &ring,
                0,
                [3.5, 3.5],
                selection_edit::Settings {
                    region: settings,
                    ..Default::default()
                },
            )?;
            let state = bucket::prepare(
                &ring,
                0,
                Some([3.5, 3.5]),
                [200, 40, 20, 255],
                false,
                false,
                settings,
            )?;
            check(
                (r.coverage(0, 0) > 0) == spill
                    && (wand.selection_coverage([0.5, 0.5]) > 0) == spill
                    && (state.layers[0].pixels[0][3] > 0) == spill,
                "Strict/loose zero/one/large gap domain",
            )?;
            ring.commit_state(state)?;
            check(
                ring.history_counts().0 == 1 && ring.undo() && ring.snapshot() == before,
                "Transparency fill single Undo",
            )?;
        }
    }
    let mut edge = make(4, 1)?;
    edge.layers[0].pixels = vec![[0; 4], [0, 0, 0, 127], [0, 0, 0, 255], [0; 4]];
    let base = Settings {
        antialias: false,
        ..Default::default()
    };
    let strict = Region::build(&edge, 0, [0.5, 0.5], base)?;
    let aa = Region::build(
        &edge,
        0,
        [0.5, 0.5],
        Settings {
            antialias: true,
            ..base
        },
    )?;
    let expanded = Region::build(&edge, 0, [0.5, 0.5], Settings { expand: 1, ..base })?;
    check(
        strict.coverage(1, 0) == 0
            && aa.coverage(1, 0) == 128
            && aa.coverage(2, 0) == 0
            && expanded.coverage(1, 0) == 255
            && expanded.coverage(2, 0) == 0,
        "AA/expansion changed transparency eligibility instead of edge coverage",
    )?;
    let mut refs = make(7, 7)?;
    let source = refs.layers[0].clone();
    refs.layers.push(source);
    for v in 1..6 {
        for (x, y) in [(v, 1), (v, 5), (1, v), (5, v)] {
            refs.layers[1].pixels[y * 7 + x] = [0, 0, 0, 255];
        }
    }
    for (reference, spill) in [
        (Reference::Active, true),
        (Reference::Layer(1), false),
        (Reference::Composite, false),
    ] {
        let region = Settings {
            reference,
            antialias: false,
            ..Default::default()
        };
        check(
            (Region::build(&refs, 0, [3.5, 3.5], region)?.coverage(0, 0) > 0) == spill
                && (selection_edit::wand(
                    &refs,
                    0,
                    [3.5, 3.5],
                    selection_edit::Settings {
                        region,
                        ..Default::default()
                    },
                )?
                .selection_coverage([0.5, 0.5])
                    > 0)
                    == spill,
            "Reference change differs between fill and wand",
        )?;
    }
    let settings = Settings {
        mode: Mode::CloseGaps,
        reference: Reference::Layer(1),
        alpha_tolerance: 0.5,
        color_tolerance: 0.25,
        gap: 2,
        expand: 1,
        antialias: true,
        ..Default::default()
    };
    let mut snapshot = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.input.fills[0] = settings;
    snapshot.selection_edit.region = settings;
    snapshot.selection_edit.combine = Combine::Add;
    let path = root.join("transparency-controls.json");
    snapshot.export(&path).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Snapshot::read(&path).map_err(|e| e.to_string())?;
    check(
        loaded.input.fills[0] == settings && loaded.selection_edit == snapshot.selection_edit,
        "Fill/wand transparency settings reopen",
    )?;
    let preset = crate::brush_presets::Preset::capture(
        &snapshot,
        crate::assets::BrushTip::Pen,
        "Transparency",
    )?;
    let reopened =
        crate::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    check(
        reopened
            .apply(&crate::workspaces::Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })?
            .input
            .fills[0]
            == settings,
        "Fill transparency preset reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn strict_loose_transparency_independent_rgb_alpha_edges_refs_settings() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
