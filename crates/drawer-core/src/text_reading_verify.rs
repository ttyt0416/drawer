use crate::text_layout::{self as t, Align, Flow, Font, Ruby, Side, Style};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-8
}
pub fn equations() -> Result<(), String> {
    let base = Style {
        font: Font::Ubuntu,
        wrap_width: 0.,
        ..Default::default()
    };
    let a = t::layout([0.; 2], "AB", 20., &base)?;
    let spaced = t::layout(
        [0.; 2],
        "AB",
        20.,
        &Style {
            tracking: 0.2,
            ..base.clone()
        },
    )?;
    check(
        close(spaced.widths[0] - a.widths[0], 4.),
        "Body tracking equation",
    )?;
    let horizontal = t::layout([0.; 2], "A", 20., &base)?;
    let vertical = t::layout(
        [0.; 2],
        "A",
        20.,
        &Style {
            flow: Flow::Vertical,
            ..base.clone()
        },
    )?;
    check(
        horizontal.paths.len() == vertical.paths.len()
            && horizontal
                .paths
                .iter()
                .flatten()
                .zip(vertical.paths.iter().flatten())
                .all(|(p, q)| close(q[0], -p[1] - 7.) && close(q[1], p[0] - 16.)),
        "Vertical Latin rotation independent equation",
    )?;
    let korean = Style {
        font: Font::Korean,
        ..base.clone()
    };
    let h = t::layout([0.; 2], "한", 20., &korean)?;
    let v = t::layout(
        [0.; 2],
        "한",
        20.,
        &Style {
            flow: Flow::Vertical,
            ..korean.clone()
        },
    )?;
    check(
        h.paths.len() == v.paths.len()
            && h.paths
                .iter()
                .flatten()
                .zip(v.paths.iter().flatten())
                .all(|(p, q)| close(q[0], p[0] - h.widths[0] / 2.) && close(q[1], p[1])),
        "Vertical Hangul upright equation",
    )?;
    let mixed = t::layout(
        [50., 30.],
        "한A\n글B",
        20.,
        &Style {
            flow: Flow::Vertical,
            wrap_width: 50.,
            ..korean.clone()
        },
    )?;
    check(
        mixed.lines == 2 && mixed.widths.iter().all(|w| *w <= 50.),
        "Mixed vertical hard newline",
    )?;
    let wrapped = t::layout(
        [100., 30.],
        "한글한글",
        20.,
        &Style {
            flow: Flow::Vertical,
            wrap_width: 30.,
            ..korean.clone()
        },
    )?;
    check(wrapped.lines == 4, "Vertical column height wrapping")?;
    let on = Style {
        ligatures: true,
        ..base.clone()
    };
    let off = t::layout([0.; 2], "ffi fi fl", 20., &base)?;
    let lig = t::layout([0.; 2], "ffi fi fl", 20., &on)?;
    check(
        lig.glyphs < off.glyphs,
        "Font liga toggle did not shape actual ligatures",
    )?;
    let emoji = t::layout(
        [0.; 2],
        "😀😀",
        20.,
        &Style {
            font: Font::Emoji,
            ligatures: true,
            ..base.clone()
        },
    )?;
    check(
        emoji.glyphs == 2
            && !emoji.ligature_unavailable.is_empty()
            && emoji.notice(Font::Emoji).contains("unavailable"),
        "Unsupported standard ligature notice",
    )?;
    let r = Ruby {
        start: 0,
        end: 1,
        text: "X".into(),
        font: Font::Ubuntu,
        ..Default::default()
    };
    let style = Style {
        ruby: vec![r],
        ..korean.clone()
    };
    let ruby = t::layout([0.; 2], "한", 20., &style)?;
    let mut farther = style.clone();
    farther.ruby[0].gap += 1.;
    let far = t::layout([0.; 2], "한", 20., &farther)?;
    check(
        ruby.paths[..h.paths.len()] == far.paths[..h.paths.len()]
            && ruby.paths[h.paths.len()..]
                .iter()
                .flatten()
                .zip(far.paths[h.paths.len()..].iter().flatten())
                .all(|(p, q)| close(p[0], q[0]) && close(q[1], p[1] - 20.)),
        "Ruby distance moved the base or wrong axis",
    )?;
    let mut long = style.clone();
    long.wrap_width = 40.;
    long.ruby[0].text = "long reading sentence".into();
    let layout = t::layout([0.; 2], "한 글", 20., &long)?;
    check(
        layout.widths[0] > 40. && layout.lines >= 2,
        "Long reading was split away from base / no space reserved",
    )?;
    for side in [Side::Before, Side::After] {
        for flow in [Flow::Horizontal, Flow::Vertical] {
            let mut s = style.clone();
            s.flow = flow;
            s.ruby[0].side = side;
            check(
                !t::layout([0.; 2], "한", 20., &s)?.paths.is_empty(),
                "Ruby side/flow omitted reading",
            )?;
        }
    }
    let mut start = style.clone();
    start.ruby[0].end = 4;
    start.ruby[0].align = Align::Left;
    let left = t::layout([0.; 2], "한글한글", 20., &start)?;
    let body = t::layout([0.; 2], "한글한글", 20., &korean)?;
    start.ruby[0].align = Align::Right;
    let right = t::layout([0.; 2], "한글한글", 20., &start)?;
    let delta = left.widths[0] - t::layout([0.; 2], "X", 10., &base)?.widths[0];
    check(
        left.paths[body.paths.len()..]
            .iter()
            .flatten()
            .zip(right.paths[body.paths.len()..].iter().flatten())
            .all(|(p, q)| close(q[0] - p[0], delta) && close(q[1], p[1])),
        "Ruby start/end alignment",
    )?;
    let mut invalid = style.clone();
    invalid.ruby[0].end = 3;
    check(
        t::layout([0.; 2], "한", 20., &invalid).is_err()
            && t::layout([0.; 2], "한\n글", 20., &invalid).is_err(),
        "Invalid/outside/newline annotation accepted",
    )?;
    invalid = style.clone();
    invalid.ruby.push(invalid.ruby[0].clone());
    check(
        t::layout([0.; 2], "한", 20., &invalid).is_err(),
        "Overlapping annotations accepted",
    )?;
    let mut mapped = vec![Ruby {
        start: 1,
        end: 3,
        text: "reading".into(),
        ..Default::default()
    }];
    t::remap_ruby("A한글Z", "xxA한글Z", &mut mapped);
    check(
        mapped[0].start == 3 && mapped[0].end == 5,
        "Ruby did not follow inserted prefix",
    )?;
    t::remap_ruby("xxA한글Z", "xxA한가글Z", &mut mapped);
    check(mapped[0].end == 6, "Ruby base range failed expansion")?;
    t::remap_ruby("xxA한가글Z", "xxAZ", &mut mapped);
    check(mapped.is_empty(), "Deleted base kept orphan reading")?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::text_layout_verify::fixture()?;
    let original = d.snapshot();
    let mut s = crate::text_layout_verify::settings();
    s.style.flow = Flow::Vertical;
    s.style.tracking = 0.1;
    s.style.ligatures = true;
    s.style.ruby = vec![Ruby {
        start: 0,
        end: 2,
        text: "한글 reading".into(),
        font: Font::Hack,
        ratio: 0.45,
        align: Align::Right,
        side: Side::After,
        gap: 0.2,
        tracking: 0.1,
    }];
    let p = t::prepare(&d, None, &s)?;
    let id = p.id;
    d.commit_state(p.state)?;
    let file = root.join("text-reading.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, crate::document::DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot(),
        "Reading mapping/style/native roundtrip",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&86u32.to_le_bytes());
    let fake = root.join("reading-fake86.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, crate::document::DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted reading/vertical features",
    )?;
    let saved = d.snapshot();
    let before = s.text.clone();
    s.text = " 123 😀\nsecond line".into();
    s.style.remap_text(&before, &s.text);
    check(s.style.ruby.is_empty(), "Deleted body annotation retained")?;
    d.commit_state(t::prepare(&d, Some(id), &s)?.state)?;
    d.undo();
    check(d.snapshot() == saved, "Text delete Undo lost reading link")?;
    d.redo();
    check(
        t::Settings::from_object(&d.scene.objects[0]).is_some_and(|s| s.style.ruby.is_empty()),
        "Redo reading removal",
    )?;
    d.undo();
    d.undo();
    check(d.snapshot() == original, "Reading create not one Undo")?;
    let mut basic = crate::text_layout_verify::settings();
    basic.style = Style::default();
    d.commit_state(t::prepare(&d, None, &basic)?.state)?;
    let basic_file = root.join("reading-basic86.drawer");
    crate::files::save(&mut d, &basic_file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&basic_file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&86u32.to_le_bytes());
    std::fs::write(&basic_file, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&basic_file, crate::document::DEFAULT_DOCUMENT_BUDGET).is_ok(),
        "Basic v86 text was unnecessarily rejected",
    )
}
pub fn limits() -> Result<(), String> {
    let base = Style {
        font: Font::Ubuntu,
        wrap_width: 0.,
        ruby: (0..64)
            .map(|n| Ruby {
                start: n,
                end: n + 1,
                text: "A".into(),
                font: Font::Hack,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let text = "A".repeat(64);
    check(
        t::layout([0.; 2], &text, 10., &base).is_ok(),
        "Maximum64 annotations rejected",
    )?;
    let mut s = base.clone();
    s.ruby.push(Ruby {
        start: 64,
        end: 65,
        ..Default::default()
    });
    check(
        t::layout([0.; 2], &text, 10., &s).is_err(),
        "65 annotations accepted",
    )?;
    for number in [f64::NAN, -1., 4.1] {
        let mut s = base.clone();
        s.ruby[0].gap = number;
        check(
            t::layout([0.; 2], &text, 10., &s).is_err(),
            "Invalid ruby distance",
        )?;
    }
    let mut s = base.clone();
    s.ruby[0].text = "A".repeat(1025);
    check(
        t::layout([0.; 2], &text, 10., &s).is_err(),
        "Reading character limit",
    )?;
    let mut s = base.clone();
    for r in &mut s.ruby {
        r.text = "A".repeat(65);
    }
    check(
        t::layout([0.; 2], &text, 10., &s).is_err(),
        "Total reading character limit",
    )?;
    for tracking in [-0.5, 4.] {
        let mut s = base.clone();
        s.tracking = tracking;
        check(
            t::layout([0.; 2], &text, 10., &s).is_ok(),
            "Tracking boundary rejected",
        )?;
    }
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    limits()?;
    lifecycle(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn annotation_count_range_and_spacing_limits() {
        super::limits().unwrap();
    }
    #[test]
    fn vertical_tracking_ligature_ruby_equations_and_ranges() {
        super::equations().unwrap();
    }
    #[test]
    fn reading_mapping_native_versions_delete_undo_and_redo() {
        let r = crate::test_support::TempDir::new();
        super::lifecycle(&r.0).unwrap();
    }
}
