use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    text_layout::{self as text, Align, Font, Settings, Style},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 240,
            height: 160,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn settings() -> Settings {
    Settings {
        baseline: [12., 28.],
        text: "한글 123 😀\nsecond line".into(),
        size: 18.,
        color: [20, 40, 80, 255],
        style: Style {
            wrap_width: 200.,
            ..Default::default()
        },
    }
}
fn min_x(paths: &[Vec<[f64; 2]>]) -> f64 {
    paths
        .iter()
        .flatten()
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min)
}
pub fn equations() -> Result<(), String> {
    let left = Style {
        font: Font::Ubuntu,
        wrap_width: 200.,
        ..Default::default()
    };
    let a = text::layout([10., 30.], "X", 20., &left)?;
    let center = text::layout(
        [10., 30.],
        "X",
        20.,
        &Style {
            align: Align::Center,
            ..left.clone()
        },
    )?;
    let right = text::layout(
        [10., 30.],
        "X",
        20.,
        &Style {
            align: Align::Right,
            ..left.clone()
        },
    )?;
    let gap = 200. - a.widths[0];
    check(
        (min_x(&center.paths) - min_x(&a.paths) - gap / 2.).abs() < 1e-9
            && (min_x(&right.paths) - min_x(&a.paths) - gap).abs() < 1e-9,
        "Text center/right alignment advance",
    )?;
    let two = text::layout(
        [10., 30.],
        "X\nX",
        20.,
        &Style {
            line_height: 2.,
            ..left.clone()
        },
    )?;
    check(
        two.lines == 2
            && two.paths.len() == a.paths.len() * 2
            && two.paths[a.paths.len()]
                .iter()
                .zip(&a.paths[0])
                .all(|(p, q)| (p[1] - q[1] - 40.).abs() < 1e-9),
        "Line spacing did not change baseline distance",
    )?;
    let wrapped = text::layout(
        [0.; 2],
        &"X".repeat(200),
        20.,
        &Style {
            wrap_width: 60.,
            ..left.clone()
        },
    )?;
    check(
        wrapped.lines > 1 && wrapped.widths.iter().all(|v| *v <= 60.),
        "Long line wrapping",
    )?;
    let mono = Style {
        font: Font::Hack,
        wrap_width: 0.,
        ..left.clone()
    };
    let i = text::layout([0.; 2], "iiii", 20., &mono)?;
    let w = text::layout([0.; 2], "WWWW", 20., &mono)?;
    check(
        i.widths == w.widths,
        "Hack font did not use monospace advances",
    )?;
    let i = text::layout([0.; 2], "iiii", 20., &left)?;
    let w = text::layout([0.; 2], "WWWW", 20., &left)?;
    check(i.widths != w.widths, "Ubuntu font choice not applied")?;
    let s = settings();
    let mixed = text::layout(s.baseline, &s.text, s.size, &s.style)?;
    check(
        mixed.missing == 0 && !mixed.paths.is_empty() && mixed.lines == 2,
        "Korean/numeric/emoji glyph coverage",
    )?;
    let emoji = text::layout(
        [0.; 2],
        "😀",
        20.,
        &Style {
            font: Font::Emoji,
            ..left.clone()
        },
    )?;
    check(
        emoji.missing == 0 && !emoji.paths.is_empty(),
        "Monochrome emoji outline missing",
    )?;
    let empty = text::layout([0.; 2], "", 20., &left)?;
    check(
        empty.lines == 1 && empty.paths.is_empty(),
        "Empty text layout",
    )?;
    for invalid in [
        Style {
            line_height: f64::NAN,
            ..left.clone()
        },
        Style {
            wrap_width: -1.,
            ..left.clone()
        },
    ] {
        check(
            text::layout([0.; 2], "X", 20., &invalid).is_err(),
            "Invalid text layout accepted",
        )?;
    }
    check(
        text::layout([0.; 2], &"x".repeat(4097), 20., &left).is_err()
            && text::layout([f64::INFINITY, 0.], "X", 20., &left).is_err()
            && text::layout([0.; 2], "X", 4097., &left).is_err(),
        "Text count/coordinate/size limit",
    )?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let s = settings();
    let p = text::prepare(&d, None, &s)?;
    check(
        d.snapshot() == original
            && p.state.layers == original.layers
            && p.state.scene.objects.len() == 1,
        "Text preview source isolation",
    )?;
    let id = p.id;
    let before = d.history_counts();
    d.commit_state(p.state)?;
    check(
        d.history_counts().0 == before.0 + 1
            && Settings::from_object(&d.scene.objects[0]) == Some(s.clone()),
        "Text object settings/one Undo",
    )?;
    let renderer = crate::render::Renderer::new(&d);
    check(renderer.error().is_none(), "Styled text render failed")?;
    let mut visible = 0;
    for y in 0..100 {
        for x in 0..240 {
            visible += usize::from(renderer.sample([x as f64 + 0.5, y as f64 + 0.5])?.alpha() > 0.);
        }
    }
    check(visible > 100, "Styled text produced no visible glyphs")?;
    drop(renderer);
    check(
        text::prepare(&d, Some(id), &s)?.state == d.snapshot(),
        "Unchanged text edit created new state",
    )?;
    let mut edited = s.clone();
    edited.text = "수정 42 😀".into();
    edited.style.align = Align::Right;
    edited.style.font = Font::Hack;
    let p = text::prepare(&d, Some(id), &edited)?;
    d.commit_state(p.state)?;
    check(
        d.scene.objects.len() == 1 && Settings::from_object(&d.scene.objects[0]) == Some(edited),
        "Text editing replaced identity or lost parameters",
    )?;
    d.undo();
    check(
        Settings::from_object(&d.scene.objects[0]) == Some(s.clone()),
        "Text edit Undo",
    )?;
    let file = root.join("text-layout.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot(),
        "Text native font/alignment/spacing roundtrip",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&85u32.to_le_bytes());
    let fake = root.join("text-fake85.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native header accepted text layout",
    )?;
    d.undo();
    check(d.snapshot() == original, "Create text Undo")?;
    let mut empty = s;
    empty.text.clear();
    let p = text::prepare(&d, None, &empty)?;
    check(
        p.state.scene.valid() && p.state.scene.objects.len() == 1,
        "Empty editable text object invalid",
    )?;
    d.commit_state(p.state)?;
    let file = root.join("empty-text.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_ok(),
        "Empty text native reopen",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    lifecycle(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn fonts_alignment_line_spacing_wrap_unicode_and_limits() {
        super::equations().unwrap();
    }
    #[test]
    fn text_create_edit_identity_render_native_empty_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::lifecycle(&root.0).unwrap();
    }
}
