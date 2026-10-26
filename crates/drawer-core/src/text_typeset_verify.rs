use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    text_layout::{self as t, Flow, Font, Style},
    text_typeset as ty,
};
fn check(ok: bool, m: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(m.into()) }
}
pub fn style() -> Style {
    Style {
        font: Font::Ubuntu,
        wrap_width: 120.,
        generation: 2,
        rich: Some(Box::default()),
        typeset: Some(Box::default()),
        ..Default::default()
    }
}
pub fn equations() -> Result<(), String> {
    let mut s = style();
    let a = t::layout([0., 30.], "AV", 20., &s)?;
    s.typeset.as_mut().unwrap().kerning = ty::Kerning::Metrics;
    let b = t::layout([0., 30.], "AV", 20., &s)?;
    check(
        b.widths[0] < a.widths[0] - 0.01,
        "Real font metric kerning did not reduce AV",
    )?;
    let mut m = style();
    m.typeset.as_mut().unwrap().manual = 0.1;
    let spaced = t::layout([0., 30.], "AV", 20., &m)?;
    check(
        (spaced.widths[0] - a.widths[0] - 2.).abs() < 1e-9,
        "Manual kerning formula",
    )?;
    let mut lig = style();
    let separate = t::layout([0., 30.], "fi", 20., &lig)?;
    lig.typeset
        .as_mut()
        .unwrap()
        .features
        .push(ty::Feature::Ligatures);
    let joined = t::layout([0., 30.], "fi", 20., &lig)?;
    check(
        joined.glyphs < separate.glyphs,
        "Real font OpenType ligature option",
    )?;
    lig.typeset.as_mut().unwrap().half_punctuation = true;
    check(
        t::layout([0., 30.], "fi", 20., &lig)?.glyphs < separate.glyphs,
        "Punctuation compression disabled Latin ligatures",
    )?;
    lig.font = Font::Hack;
    lig.typeset
        .as_mut()
        .unwrap()
        .features
        .push(ty::Feature::ContextualAlternates);
    check(
        t::layout([0., 30.], "fi", 20., &lig)?
            .notice(lig.font)
            .contains("Contextual alternates unavailable"),
        "Missing calt feature notice",
    )?;
    let mut centered = style();
    centered.typeset.as_mut().unwrap().last = ty::LastLine::Center;
    let c = t::layout([0., 30.], "AV", 20., &centered)?;
    check(
        a.paths
            .iter()
            .flatten()
            .zip(c.paths.iter().flatten())
            .all(|(p, q)| {
                (q[0] - p[0] - (120. - a.widths[0]) / 2.).abs() < 1e-9 && (q[1] - p[1]).abs() < 1e-9
            }),
        "Last line center independent coordinates",
    )?;
    let mut justified = style();
    justified.wrap_width = 31.;
    {
        let o = justified.typeset.as_mut().unwrap();
        o.justify = true;
        o.background = [10, 20, 30, 120];
    }
    let j = t::layout([0., 30.], "ABCD", 20., &justified)?;
    check(
        j.lines == 2 && j.backgrounds.len() == 4 && (j.backgrounds[1][1][0] - 31.).abs() < 1e-9,
        "Wrapped line justification did not fill its box",
    )?;
    let mut vertical = style();
    vertical.flow = Flow::Vertical;
    vertical.typeset.as_mut().unwrap().background = [20, 40, 60, 128];
    let full = t::layout([60., 30.], "12한글", 20., &vertical)?;
    vertical.typeset.as_mut().unwrap().horizontal.push([0, 2]);
    let tcy = t::layout([60., 30.], "12한글", 20., &vertical)?;
    check(
        full.backgrounds.len() == 4 && tcy.backgrounds.len() == 3 && tcy.glyphs == full.glyphs,
        "Horizontal-in-vertical did not form one atomic cell",
    )?;
    vertical.rich.as_mut().unwrap().spans = vec![
        crate::text_format::Span {
            start: 0,
            end: 1,
            attr: crate::text_format::Attr {
                color: Some([255, 0, 0, 255]),
                ..Default::default()
            },
        },
        crate::text_format::Span {
            start: 1,
            end: 2,
            attr: crate::text_format::Attr {
                color: Some([0, 0, 255, 255]),
                ..Default::default()
            },
        },
    ];
    let mixed = t::layout([60., 30.], "12한글", 20., &vertical)?;
    check(
        mixed
            .paint
            .iter()
            .any(|p| p.paint.color == Some([255, 0, 0, 255]))
            && mixed
                .paint
                .iter()
                .any(|p| p.paint.color == Some([0, 0, 255, 255])),
        "TCY discarded range colors",
    )?;
    vertical.typeset.as_mut().unwrap().horizontal.clear();
    vertical.rich = Some(Box::default());
    let top = t::layout([60., 30.], "AB", 20., &vertical)?;
    vertical.typeset.as_mut().unwrap().last = ty::LastLine::Bottom;
    let bottom = t::layout([60., 30.], "AB", 20., &vertical)?;
    check(
        top.paths
            .iter()
            .flatten()
            .zip(bottom.paths.iter().flatten())
            .all(|(a, b)| (b[1] - a[1] - (120. - top.widths[0])).abs() < 1e-9),
        "Vertical last-line bottom",
    )?;
    let mut rtl = style();
    rtl.rich.as_mut().unwrap().spans = (0..4)
        .map(|i| crate::text_format::Span {
            start: i,
            end: i + 1,
            attr: crate::text_format::Attr {
                color: Some([i as u8 + 1, 0, 0, 255]),
                ..Default::default()
            },
        })
        .collect();
    let reordered = t::layout([0., 30.], "אABב", 20., &rtl)?;
    let order: Vec<_> = reordered
        .paint
        .iter()
        .filter_map(|p| p.paint.color.map(|c| c[0]))
        .collect();
    check(
        order == [4, 2, 3, 1],
        "Mixed RTL/LTR visual ordering changed Latin order",
    )?;
    let mut punctuation = style();
    punctuation.flow = Flow::Vertical;
    let full = t::layout([0., 0.], "。！", 20., &punctuation)?;
    punctuation.typeset.as_mut().unwrap().half_punctuation = true;
    check(
        t::layout([0., 0.], "。！", 20., &punctuation)?.widths[0] <= full.widths[0],
        "Punctuation compression expanded line",
    )?;
    let mut limits = style();
    limits.typeset.as_mut().unwrap().manual = f64::NAN;
    check(
        t::layout([0., 0.], "", 20., &limits).is_err(),
        "Invalid manual kerning accepted",
    )?;
    limits = style();
    limits.typeset.as_mut().unwrap().horizontal = vec![[0, 1]];
    check(
        t::layout([0., 0.], "a\u{301}", 20., &limits).is_err(),
        "TCY split grapheme",
    )?;
    limits = style();
    limits.typeset.as_mut().unwrap().manual = -0.5;
    check(
        t::layout([0., 0.], "", 0.1, &limits).is_ok(),
        "Empty minimum text rejected",
    )?;
    limits.typeset.as_mut().unwrap().manual = 4.;
    check(
        t::layout([0., 0.], "A", 4096., &limits).is_ok(),
        "Maximum typography rejected",
    )?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::text_layout_verify::fixture()?;
    let original = d.snapshot();
    let mut settings = crate::text_layout_verify::settings();
    settings.text = "한글 Latin 日本語\n12 한글".into();
    settings.style = style();
    settings.style.flow = Flow::Vertical;
    settings.style.wrap_width = 90.;
    settings.style.typeset.as_mut().unwrap().background = [70, 90, 30, 120];
    settings.style.ruby.push(t::Ruby {
        start: 0,
        end: 2,
        text: "독음".into(),
        font: Font::Hack,
        ..Default::default()
    });
    let p = t::prepare(&d, None, &settings)?;
    d.commit_state(p.state)?;
    let expected = crate::render::Renderer::new(&d)
        .output(d.dpi, DEFAULT_DOCUMENT_BUDGET)?
        .2;
    let path = root.join("advanced-text.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        t::Settings::from_object(&reopened.scene.objects[0]) == Some(settings.clone())
            && crate::render::Renderer::new(&reopened)
                .output(reopened.dpi, DEFAULT_DOCUMENT_BUDGET)?
                .2
                == expected,
        "Typography Unicode/ruby/spacing native roundtrip",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&90u32.to_le_bytes());
    std::fs::write(root.join("invalid-advanced.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(
            &root.join("invalid-advanced.drawer"),
            DEFAULT_DOCUMENT_BUDGET,
        )
        .is_err(),
        "Old header accepted typography",
    )?;
    check(
        d.undo() && d.snapshot() == original,
        "Typography document one Undo",
    )?;
    let mut bg = settings.clone();
    bg.text = "A".into();
    bg.style.ruby.clear();
    bg.style.flow = Flow::Horizontal;
    bg.color = [0; 4];
    bg.baseline = [12., 28.];
    bg.size = 20.;
    let p = t::prepare(&d, None, &bg)?;
    let page = crate::layer_merge::preview(&p.state, 0)?;
    let r = crate::render::Renderer::new(&page);
    check(
        r.pixel([13., 10.]) == [70, 90, 30, 120],
        "Character background alpha or independence from transparent text",
    )?;
    Ok(())
}
pub fn preferences(root: &std::path::Path) -> Result<(), String> {
    let mut tool = ty::Tool::default();
    let mut current = crate::text_layout_verify::settings();
    current.style.frame = crate::scene::Affine([1., 0.2, 0., 1., 5., 7.]);
    tool.defaults.size = 37.;
    tool.defaults.color = [20, 40, 60, 70];
    for target in [
        ty::ApplyTarget::None,
        ty::ApplyTarget::NewTextOnly,
        ty::ApplyTarget::SelectedText,
    ] {
        tool.target = target;
        for selected in [false, true] {
            let next = ty::apply_defaults(&tool, selected, &current);
            let applies = matches!(
                (target, selected),
                (ty::ApplyTarget::NewTextOnly, false) | (ty::ApplyTarget::SelectedText, true)
            );
            check(
                (next.size == 37.) == applies
                    && next.text == current.text
                    && next.baseline == current.baseline,
                "Text default target policy",
            )?;
            if selected {
                check(
                    next.style.frame == current.style.frame,
                    "Default format removed text transform",
                )?;
            }
        }
    }
    let storage =
        crate::storage::UserStorage::at(root.join("typeset-user")).map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    tool.quality = ty::Quality::High;
    tool.create = ty::CreateAction::AddToSelectedText;
    tool.drag = ty::DragAction::SelectText;
    *store.current.text_tool = tool.clone();
    store.save(&storage).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Store::load(&storage)
            .map_err(|e| e.to_string())?
            .ok_or("Saved text tool missing")?
            .current
            .text_tool
            .as_ref()
            == &tool,
        "Text tool preferences restart",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    lifecycle(root)?;
    preferences(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn typography_equations_bidi_tcy_features_and_limits() {
        super::equations().unwrap();
    }
    #[test]
    fn typography_native_background_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::lifecycle(&root.0).unwrap();
    }
    #[test]
    fn text_tool_targets_and_disk_restart() {
        let root = crate::test_support::TempDir::new();
        super::preferences(&root.0).unwrap();
    }
}
