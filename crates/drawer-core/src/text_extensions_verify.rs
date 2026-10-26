use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    font_library::{self as fonts, owned},
    text_layout::{self as t, Font},
};
use std::path::Path;
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn owned_fonts(root: &Path) -> Result<(), String> {
    let source = root.join("font-source");
    std::fs::create_dir_all(source.join("other")).map_err(|e| e.to_string())?;
    let mut bytes = epaint_default_fonts::UBUNTU_LIGHT.to_vec();
    bytes.extend_from_slice(root.to_string_lossy().as_bytes());
    let path = source.join("same.ttf");
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    let storage =
        crate::storage::UserStorage::at(root.join("owned-font-user")).map_err(|e| e.to_string())?;
    check(
        owned::install(&storage, &path, false).is_err(),
        "Unconfirmed font copy accepted",
    )?;
    let first = owned::install(&storage, &path, true).map_err(|e| e.to_string())?;
    let actual = storage.root().join("fonts").join(&first.file);
    check(
        std::fs::read(&path).map_err(|e| e.to_string())? == bytes
            && std::fs::read(&actual).map_err(|e| e.to_string())? == bytes,
        "Font copy changed source",
    )?;
    check(
        owned::install(&storage, &path, true)
            .map_err(|e| e.to_string())?
            .digest
            == first.digest
            && owned::load(&storage).map_err(|e| e.to_string())?.len() == 1,
        "Duplicate font copy",
    )?;
    let second_source = source.join("other/same.ttf");
    let mut different = epaint_default_fonts::UBUNTU_LIGHT.to_vec();
    different.extend_from_slice(root.to_string_lossy().as_bytes());
    different.extend_from_slice(b"independent same-name font");
    std::fs::write(&second_source, &different).map_err(|e| e.to_string())?;
    let second = owned::install(&storage, &second_source, true).map_err(|e| e.to_string())?;
    check(
        first.faces[0].name() == second.faces[0].name()
            && first.file != second.file
            && owned::load(&storage).map_err(|e| e.to_string())?.len() == 2,
        "Same-named fonts overwrote each other",
    )?;
    let mut d = crate::text_layout_verify::fixture()?;
    let mut settings = crate::text_layout_verify::settings();
    settings.text = "AB 한글".into();
    settings.style.font = Font::Local(first.faces[0]);
    let p = t::prepare(&d, None, &settings)?;
    d.commit_state(p.state)?;
    let before = d.snapshot();
    check(
        fonts::data(first.faces[0]).is_some(),
        "Installed font was not registered",
    )?;
    check(
        owned::remove(&storage, first.digest, "wrong name").is_err() && actual.exists(),
        "Unconfirmed deletion changed font",
    )?;
    let temporary = storage.root().join("fonts/drawer-fonts-write.tmp");
    std::fs::write(&temporary, "foreign temporary").map_err(|e| e.to_string())?;
    check(
        owned::remove(&storage, first.digest, first.faces[0].name()).is_err()
            && actual.exists()
            && owned::load(&storage).map_err(|e| e.to_string())?.len() == 2,
        "Failed font removal did not roll back",
    )?;
    check(
        std::fs::read_to_string(&temporary).map_err(|e| e.to_string())? == "foreign temporary",
        "Foreign temporary overwritten",
    )?;
    std::fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    owned::remove(&storage, first.digest, first.faces[0].name()).map_err(|e| e.to_string())?;
    check(
        !actual.exists()
            && d.snapshot() == before
            && std::fs::read(&path).map_err(|e| e.to_string())? == bytes
            && fonts::data(first.faces[0]).is_none(),
        "Font removal changed document/source or retained deleted copy",
    )?;
    check(
        t::layout(
            settings.baseline,
            &settings.text,
            settings.size,
            &settings.style,
        )?
        .notice(settings.style.font)
        .contains("unavailable"),
        "Removed in-use font has no fallback notice",
    )?;
    let file = root.join("removed-font.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        t::Settings::from_object(
            &crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .scene
                .objects[0],
        ) == Some(settings),
        "Font removal lost native Unicode/font reference",
    )?;
    owned::install(&storage, &path, true).map_err(|e| e.to_string())?;
    check(
        fonts::data(first.faces[0]).is_some(),
        "Reinstalled font identity did not recover",
    )?;
    let mut recent = fonts::Settings::default();
    recent.used(Font::Ubuntu);
    recent.used(Font::Hack);
    recent.used(Font::Ubuntu);
    check(
        recent.recent == [Font::Ubuntu, Font::Hack] && recent.valid(),
        "Recent font order/dedup",
    )?;
    Ok(())
}
pub fn circular(root: &Path) -> Result<(), String> {
    use crate::text_path::{Circle, Effects, Side};
    let c = Circle {
        radius: 10.,
        angle: 0.,
        side: Side::Outside,
        equal: true,
    };
    let expected = [[10., 0.], [0., 10.], [-10., 0.], [0., -10.]];
    for (i, want) in expected.into_iter().enumerate() {
        let p = c.map([0.; 2], [0.; 2], 0., 0., i, 4);
        check(
            (0..2).all(|a| (p[a] - want[a]).abs() < 1e-9),
            "Equal circle coordinate formula",
        )?;
    }
    check(
        (c.map([0., -2.], [0.; 2], 0., 0., 0, 1)[0] - 12.).abs() < 1e-9
            && ((Circle {
                side: Side::Inside,
                ..c
            })
            .map([0., -2.], [0.; 2], 0., 0., 0, 1)[0]
                - 8.)
                .abs()
                < 1e-9,
        "Inside/outside glyph direction",
    )?;
    check(
        !(Circle { radius: 0., ..c }).valid(),
        "Zero circle radius accepted",
    )?;
    let mut settings = crate::text_layout_verify::settings();
    settings.text = "한글 AB\n둘째 줄".into();
    settings.style = crate::text_typeset_verify::style();
    settings.style.flow = t::Flow::Vertical;
    settings.style.wrap_width = 60.;
    settings.style.effects = Some(Effects {
        underline: true,
        strike: true,
        slant: 18.,
    });
    settings.style.ruby.push(t::Ruby {
        start: 0,
        end: 1,
        text: "한".into(),
        font: Font::Hack,
        ..Default::default()
    });
    settings.style.rich.as_mut().unwrap().spans = vec![crate::text_format::Span {
        start: 1,
        end: 2,
        attr: crate::text_format::Attr {
            font: Some(Font::Hack),
            color: Some([10, 20, 230, 255]),
            ..Default::default()
        },
    }];
    let regular = settings.clone();
    let normal = t::layout(
        settings.baseline,
        &settings.text,
        settings.size,
        &settings.style,
    )?;
    settings.style.circle = Some(Circle {
        radius: 42.,
        angle: -90.,
        side: Side::Outside,
        equal: false,
    });
    let circular = t::layout(
        settings.baseline,
        &settings.text,
        settings.size,
        &settings.style,
    )?;
    let notes = crate::text_path::hidden(&settings.text, &settings.style);
    check(
        notes.len() >= 4 && circular.lines == 1 && circular.glyphs < normal.glyphs,
        "Circle did not hide later lines/ruby/effects with notices",
    )?;
    let (first, transient) = crate::text_path::transient(&settings.text, &settings.style);
    check(
        first == "한글 AB"
            && transient.ruby.is_empty()
            && transient.effects.is_none()
            && transient.wrap_width == 0.
            && settings.style.ruby == regular.style.ruby,
        "Circular transient modified saved formatting",
    )?;
    let mut d = crate::text_layout_verify::fixture()?;
    let before = d.snapshot();
    let p = t::prepare(&d, None, &settings)?;
    d.commit_state(p.state)?;
    let path = root.join("circular-text.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let mut restored = t::Settings::from_object(&reopened.scene.objects[0]).unwrap();
    check(
        restored == settings,
        "Circle native lost Unicode or suppressed formatting",
    )?;
    restored.style.circle = None;
    check(
        restored == regular
            && t::layout(
                restored.baseline,
                &restored.text,
                restored.size,
                &restored.style,
            )?
            .paths
                == normal.paths,
        "Regular return lost original formatting/pixels",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&91u32.to_le_bytes());
    let invalid = root.join("invalid-circular.drawer");
    std::fs::write(&invalid, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&invalid, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted circle/effects",
    )?;
    check(d.undo() && d.snapshot() == before, "Circular text Undo")?;
    let mut empty = settings.clone();
    empty.text = "\n한글".into();
    empty.style.ruby.clear();
    empty.style.rich.as_mut().unwrap().spans.clear();
    check(
        t::layout(empty.baseline, &empty.text, empty.size, &empty.style)?
            .paths
            .is_empty(),
        "Empty circular first line rendered hidden later text",
    )?;
    empty.text = "A".repeat(512);
    empty.style.circle = Some(Circle {
        radius: 2.,
        equal: true,
        ..Default::default()
    });
    check(
        t::layout(empty.baseline, &empty.text, empty.size, &empty.style)?
            .notice(empty.style.font)
            .contains("overlap"),
        "Long circular text had no overlap notice",
    )?;
    Ok(())
}
pub fn page(text: &str) -> Result<crate::document::Document, String> {
    let mut d = crate::text_layout_verify::fixture()?;
    let mut s = crate::text_layout_verify::settings();
    s.text = text.into();
    s.style = crate::text_typeset_verify::style();
    let p = t::prepare(&d, None, &s)?;
    d.commit_state(p.state)?;
    Ok(d)
}
pub fn story(root: &Path) -> Result<(), String> {
    use crate::story_text::{self as story, Query};
    let mut docs = vec![page("한글 abc ABC\nabc")?, page("abc 한글 ABC abc")?];
    let mut files = vec![];
    let mut original_files = vec![];
    for (i, d) in docs.iter_mut().enumerate() {
        let path = root.join(format!("work-page-{i}.drawer"));
        crate::files::save(d, &path).map_err(|e| e.to_string())?;
        original_files.push(std::fs::read(&path).map_err(|e| e.to_string())?);
        files.push(d.path.clone().unwrap());
    }
    let work = story::Work::new("로컬 작품".into(), files.clone())?;
    let manifest = root.join("local-work.drawer-work");
    work.save(&manifest)?;
    check(
        story::Work::load(&manifest)?.pages == files,
        "Local work manifest roundtrip",
    )?;
    let query = Query {
        find: "abc".into(),
        replace: "새말".into(),
        case_sensitive: false,
    };
    let original: Vec<_> = docs.iter().map(|d| d.snapshot()).collect();
    let references: Vec<_> = docs.iter().collect();
    let prepared = story::prepare(&references, &query)?;
    check(
        prepared.matches.len() == 6
            && prepared.pages.len() == 2
            && docs.iter().zip(&original).all(|(d, s)| d.snapshot() == *s),
        "Whole-work preview mutated source or missed case-insensitive matches",
    )?;
    docs[1].revision += 1;
    check(
        prepared.apply(&mut docs).is_err()
            && docs.iter().zip(&original).all(|(d, s)| d.snapshot() == *s),
        "Stale second page partly changed the work",
    )?;
    docs[1].revision -= 1;
    let prepared = story::prepare(&docs.iter().collect::<Vec<_>>(), &query)?;
    docs[0].view.pan = [30., 20.];
    let tx = prepared.apply(&mut docs)?;
    check(
        docs[0].view.pan == [30., 20.]
            && docs.iter().all(|d| {
                t::Settings::from_object(&d.scene.objects[0])
                    .unwrap()
                    .text
                    .matches("새말")
                    .count()
                    == 3
            }),
        "Work Apply or view preservation",
    )?;
    for (i, file) in files.iter().enumerate() {
        check(
            std::fs::read(file).map_err(|e| e.to_string())? == original_files[i],
            "Replacement automatically overwrote source file",
        )?;
    }
    tx.step(&mut docs, false)?;
    check(
        docs.iter()
            .zip(&original)
            .all(|(d, s)| d.snapshot() == *s && !d.dirty),
        "Work one Undo failed to restore all pages/save markers",
    )?;
    tx.step(&mut docs, true)?;
    let other = docs[1].snapshot();
    docs[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    check(
        tx.step(&mut docs, false).is_err() && docs[1].snapshot() == other,
        "Later edit allowed partial linked Undo",
    )?;
    docs[0].undo();
    tx.step(&mut docs, false)?;
    check(
        story::prepare(&[&docs[0], &docs[0]], &query).is_err(),
        "Duplicate work page accepted",
    )?;
    check(
        story::prepare(
            &[&docs[0]],
            &Query {
                find: String::new(),
                ..query.clone()
            },
        )
        .is_err(),
        "Empty find accepted",
    )?;
    let no = story::prepare(
        &[&docs[0]],
        &Query {
            find: "missing".into(),
            ..query.clone()
        },
    )?;
    check(
        no.matches.is_empty() && no.pages.is_empty(),
        "No-match work changed text",
    )?;
    let combined = page("a\u{301}")?;
    check(
        story::prepare(
            &[&combined],
            &Query {
                find: "a".into(),
                ..query.clone()
            },
        )
        .is_err(),
        "Find split a combining grapheme",
    )?;
    let literal = page("a.b axb")?;
    check(
        story::prepare(
            &[&literal],
            &Query {
                find: "a.b".into(),
                replace: "X".into(),
                case_sensitive: true,
            },
        )?
        .matches
        .len()
            == 1,
        "Find interpreted regular expression syntax",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    owned_fonts(root)?;
    circular(root)?;
    story(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn circle_first_line_hidden_format_roundtrip_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::circular(&root.0).unwrap();
    }
    #[test]
    fn work_replacement_preview_native_atomic_history_and_limits() {
        let root = crate::test_support::TempDir::new();
        super::story(&root.0).unwrap();
    }
    #[test]
    fn owned_fonts_copy_delete_source_safety_and_reinstall() {
        let root = crate::test_support::TempDir::new();
        super::owned_fonts(&root.0).unwrap();
    }
}
