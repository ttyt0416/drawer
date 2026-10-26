use crate::{
    text_format::{self as f, Attr, Border, Group, Mapping, Presets, Rich, Rule},
    text_layout::{self as t, Font, Style},
};
fn check(ok: bool, m: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(m.into()) }
}
pub fn equations() -> Result<(), String> {
    let text = "a\u{301}한👨\u{200d}👩\u{200d}👧\u{200d}👦🇰🇷";
    check(
        f::boundaries(text) == vec![0, 2, 3, 10, 12],
        "Extended grapheme boundaries",
    )?;
    check(
        f::expand(text, 1, 8) == (0, 10),
        "Combining/ZWJ range expansion",
    )?;
    let mut rich = Rich::default();
    rich.set(
        text,
        1,
        8,
        Some(Attr {
            font: Some(Font::Hack),
            color: Some([255, 0, 0, 255]),
            ..Default::default()
        }),
    )?;
    check(
        rich.spans[0].start == 0 && rich.spans[0].end == 10 && rich.valid(text),
        "Range formatting split a grapheme",
    )?;
    let mut invalid = rich.clone();
    invalid.spans[0].start = 1;
    check(
        !invalid.valid(text),
        "Middle-of-grapheme stored range accepted",
    )?;
    let mut mapping = Mapping {
        rules: Group::ALL
            .into_iter()
            .filter(|g| *g != Group::Custom)
            .map(|group| Rule {
                group,
                from: 0,
                through: 0,
                font: Font::Hack,
                ratio: 1.5,
            })
            .collect(),
        ..Default::default()
    };
    mapping.rules.insert(
        0,
        Rule {
            group: Group::Custom,
            from: 0x1f000,
            through: 0x1ffff,
            font: Font::Emoji,
            ratio: 2.,
        },
    );
    let mapped = Rich {
        mapping: Some(mapping.clone()),
        ..Default::default()
    };
    for g in ["한", "a\u{301}", "٣", "漢", "カ", "。"] {
        let a = mapped.resolve(g, 0, Font::Ubuntu);
        check(
            a.font == Some(Font::Hack) && a.ratio == Some(1.5),
            "Unicode script mapping / combining inheritance",
        )?;
    }
    check(
        mapped.resolve("😀", 0, Font::Ubuntu).font == Some(Font::Emoji),
        "Custom Unicode rule",
    )?;
    let base = Style {
        font: Font::Ubuntu,
        wrap_width: 0.,
        ..Default::default()
    };
    let raw = t::layout([0.; 2], "AB", 20., &base)?;
    let styled = t::layout(
        [0.; 2],
        "AB",
        20.,
        &Style {
            rich: Some(Box::new(Rich {
                mapping: Some(Mapping {
                    base: Font::Ubuntu,
                    rules: vec![Rule {
                        group: Group::Latin,
                        from: 0,
                        through: 0,
                        font: Font::Ubuntu,
                        ratio: 2.,
                    }],
                }),
                ..Default::default()
            })),
            ..base.clone()
        },
    )?;
    check(
        (styled.widths[0] - raw.widths[0] * 2.).abs() < 1e-8,
        "Mapped size ratio not used for actual layout",
    )?;
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦X";
    let l = t::layout(
        [0.; 2],
        family,
        20.,
        &Style {
            font: Font::Emoji,
            wrap_width: 1.,
            rich: Some(Box::default()),
            ..base.clone()
        },
    )?;
    check(l.lines == 2, "Line wrap split a ZWJ grapheme")?;
    let mut remapped = rich.clone();
    remapped.remap(text, &format!("ZZ{text}"));
    check(
        remapped.spans[0].start == 2 && remapped.spans[0].end == 12,
        "Span prefix remap",
    )?;
    let mut p = Presets::default();
    p.save("혼합".into(), mapping.clone(), false)?;
    check(
        p.save(" 혼합 ".into(), mapping.clone(), false).is_err(),
        "Duplicate preset replaced silently",
    )?;
    check(
        !Attr {
            ratio: Some(f64::NAN),
            ..Default::default()
        }
        .valid()
            && !Border {
                width: 129.,
                color: [0; 4],
            }
            .valid(),
        "Invalid typography parameters",
    )?;
    Ok(())
}
pub fn lifecycle(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::text_layout_verify::fixture()?;
    let original = d.snapshot();
    let mut s = crate::text_layout_verify::settings();
    s.text = "AB".into();
    s.baseline = [20., 40.];
    s.size = 20.;
    s.color = [0; 4];
    s.style = Style {
        font: Font::Ubuntu,
        wrap_width: 0.,
        rich: Some(Box::default()),
        ..Default::default()
    };
    let rich = s.style.rich.as_mut().unwrap();
    rich.set(
        &s.text,
        0,
        1,
        Some(Attr {
            font: Some(Font::Ubuntu),
            color: Some([255, 0, 0, 255]),
            border: Some(Border {
                width: 4.,
                color: [0, 255, 0, 255],
            }),
            ..Default::default()
        }),
    )?;
    rich.set(
        &s.text,
        1,
        2,
        Some(Attr {
            color: Some([0, 0, 255, 255]),
            ..Default::default()
        }),
    )?;
    d.commit_state(t::prepare(&d, None, &s)?.state)?;
    let saved = d.snapshot();
    let r = crate::render::Renderer::new(&d);
    let mut colors = [0usize; 3];
    for y in 0..70 {
        for x in 0..90 {
            let p = r.sample([x as f64 + 0.5, y as f64 + 0.5])?.rgba8();
            for (n, color) in [[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]]
                .iter()
                .enumerate()
            {
                colors[n] += usize::from(p == *color);
            }
        }
    }
    check(
        colors.iter().all(|n| *n > 10),
        "Mixed fill/outline pixels or transparent base color discarded styles",
    )?;
    drop(r);
    let bounds = crate::render::text_pixel_bounds(&d, 0)?;
    check(
        bounds.min[0] < 20. && bounds.max[0] > 35.,
        "Glyph alignment bounds omitted outline",
    )?;
    let file = root.join("formatted-text.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, crate::document::DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved,
        "Formatting native snapshot",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&87u32.to_le_bytes());
    let fake = root.join("format-fake87.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, crate::document::DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native header accepted formatting",
    )?;
    let storage = crate::storage::UserStorage::at(root.join("font-mix-settings"))
        .map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    let map = Mapping::default();
    store
        .current
        .text_mix
        .save("mixed".into(), map.clone(), false)?;
    store.save(&storage).map_err(|e| e.to_string())?;
    let reopened = crate::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Mixed preset restart missing")?;
    check(
        reopened.current.text_mix == store.current.text_mix,
        "Mixed-font preset disk restart",
    )?;
    s.style.rich.as_mut().unwrap().mapping = Some(map.clone());
    let id = d.scene.catalog.objects.id(0);
    d.commit_state(t::prepare(&d, Some(id), &s)?.state)?;
    let frozen = d.snapshot();
    let mut changed = map;
    changed.base = Font::Hack;
    store
        .current
        .text_mix
        .save("mixed".into(), changed.clone(), true)?;
    store.current.text_mix.values.clear();
    store.save(&storage).map_err(|e| e.to_string())?;
    check(
        d.snapshot() == frozen,
        "Preset edit/delete mutated existing document",
    )?;
    s.style.rich.as_mut().unwrap().mapping = Some(changed);
    d.commit_state(t::prepare(&d, Some(id), &s)?.state)?;
    d.undo();
    check(d.snapshot() == frozen, "Explicit mapping reapply Undo")?;
    d.undo();
    check(d.snapshot() == saved, "Mapping creation Undo")?;
    d.undo();
    check(
        d.snapshot() == original,
        "Formatted text create not one Undo",
    )
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    equations()?;
    lifecycle(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn grapheme_styles_scripts_ratios_wrap_and_validation() {
        super::equations().unwrap();
    }
    #[test]
    fn mixed_fill_outline_pixel_bounds_native_presets_and_undo() {
        let r = crate::test_support::TempDir::new();
        super::lifecycle(&r.0).unwrap();
    }
}
