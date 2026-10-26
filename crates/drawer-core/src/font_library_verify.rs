use crate::{
    font_library::{self as fonts, Catalog, Display, Settings},
    text_layout::{self as text, Font, Style},
};
use std::path::Path;
fn check(ok: bool, m: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(m.into()) }
}
pub fn fixture(root: &Path) -> Result<Catalog, String> {
    let dir = root.join("local-fonts");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, data) in [
        ("ubuntu.ttf", epaint_default_fonts::UBUNTU_LIGHT),
        ("hack.ttf", epaint_default_fonts::HACK_REGULAR),
    ] {
        let mut data = data.to_vec();
        data.extend_from_slice(root.to_string_lossy().as_bytes());
        std::fs::write(dir.join(name), data).map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join("damaged.ttf"), b"invalid font").map_err(|e| e.to_string())?;
    Ok(Catalog::discover(&[dir], || false))
}
pub fn lifecycle(root: &Path) -> Result<(), String> {
    let catalog = fixture(root)?;
    check(
        catalog.failures == 1 && catalog.entries.len() == 6,
        "Local font discovery/damaged file",
    )?;
    let entry = catalog
        .entries
        .iter()
        .find(|e| matches!(e.font, Font::Local(_)) && e.name.to_lowercase().contains("ubuntu"))
        .ok_or("Local Ubuntu fixture")?;
    let path = entry.path.as_ref().unwrap();
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let font = entry.font;
    let a = text::layout(
        [0.; 2],
        "AB",
        20.,
        &Style {
            font,
            wrap_width: 0.,
            ..Default::default()
        },
    )?;
    let b = text::layout(
        [0.; 2],
        "AB",
        20.,
        &Style {
            font: Font::Ubuntu,
            wrap_width: 0.,
            ..Default::default()
        },
    )?;
    check(
        a.paths == b.paths && a.widths == b.widths && a.substituted == 0,
        "Local content identity not used for actual glyph layout",
    )?;
    check(
        fonts::character(font, 0x41)?.1
            && !fonts::character(font, 0x10ffff)?.1
            && fonts::character(font, 0xd800).is_err(),
        "Character map coverage/surrogate policy",
    )?;
    let mut d = crate::text_layout_verify::fixture()?;
    let original = d.snapshot();
    let mut settings = crate::text_layout_verify::settings();
    settings.text = "AB".into();
    settings.style.font = font;
    d.commit_state(text::prepare(&d, None, &settings)?.state)?;
    let file = root.join("local-font.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let saved = d.snapshot();
    check(
        crate::files::open(&file, crate::document::DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved,
        "Local font native reference roundtrip",
    )?;
    let mut fake = std::fs::read(&file).map_err(|e| e.to_string())?;
    fake[8..12].copy_from_slice(&88u32.to_le_bytes());
    let fake_file = root.join("local-font-fake88.drawer");
    std::fs::write(&fake_file, fake).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake_file, crate::document::DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native header accepted local fonts",
    )?;
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    let missing = text::layout(
        [0.; 2],
        "AB",
        20.,
        &Style {
            font,
            wrap_width: 0.,
            ..Default::default()
        },
    )?;
    check(
        !missing.requested_available
            && missing.substituted == 2
            && missing.notice(font).contains(&font.label()),
        "Missing local font lost identity/notice",
    )?;
    check(
        crate::files::open(&file, crate::document::DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved,
        "Missing font changed original document",
    )?;
    std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    check(
        text::layout(
            [0.; 2],
            "AB",
            20.,
            &Style {
                font,
                ..Default::default()
            },
        )?
        .substituted
            == 0,
        "Reinstalled font identity failed",
    )?;
    let _ = Catalog::discover(&[root.join("local-fonts")], || true);
    check(
        std::fs::read(path).map_err(|e| e.to_string())? == bytes,
        "Discovery/cancellation modified a font file",
    )?;
    d.undo();
    check(d.snapshot() == original, "Local font text document Undo")
}
pub fn lists(root: &Path) -> Result<(), String> {
    let catalog = fixture(root)?;
    let mut s = Settings::default();
    let first = s.create("한글 글꼴 목록".into())?;
    check(
        s.lists[0].members.is_empty() && s.choices(&catalog).is_empty(),
        "New font list is not empty",
    )?;
    s.include(first, Font::Hack, true)?;
    s.bulk(first, &catalog, "Ubuntu", true)?;
    check(
        s.lists[0].members.contains(&Font::Hack) && s.lists[0].members.len() >= 3,
        "Search bulk changed hidden membership",
    )?;
    s.bulk(first, &catalog, "Ubuntu", false)?;
    check(
        s.lists[0].members == vec![Font::Hack],
        "Search exclude changed hidden member",
    )?;
    s.bulk(first, &catalog, "", true)?;
    check(
        s.lists[0].members.len() == catalog.entries.len(),
        "Empty query did not apply to all fonts",
    )?;
    let second = s.duplicate(first, "독립 복사".into())?;
    s.include(second, Font::Hack, false)?;
    check(
        s.lists[0].members.contains(&Font::Hack) && !s.lists[1].members.contains(&Font::Hack),
        "Duplicated font lists share mutable membership",
    )?;
    let unchanged = s.clone();
    check(
        s.rename(second, "한글 글꼴 목록".into()).is_err() && s == unchanged,
        "Duplicate list name overwrote existing data",
    )?;
    check(
        s.delete(second, "wrong name").is_err() && s == unchanged,
        "List delete ignored confirmation",
    )?;
    s.lists[0].display = Display::OwnFont;
    s.favorite(Font::Hack, true);
    let storage =
        crate::storage::UserStorage::at(root.join("font-list-store")).map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    store.current.font_lists = s.clone();
    store.font_restore = Some(Box::new(Settings::default()));
    store.save(&storage).map_err(|e| e.to_string())?;
    let reopened = crate::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Font preferences missing after restart")?;
    check(
        reopened.current.font_lists == s && reopened.font_restore == store.font_restore,
        "Font list/favorite/display/restore record restart",
    )?;
    s.delete(second, "독립 복사")?;
    s.delete(first, "한글 글꼴 목록")?;
    check(
        s.lists.is_empty() && s.active.is_none() && s.valid(),
        "Last list deletion invalidated settings",
    )?;
    let next = s.create("한글".repeat(30))?;
    check(next > second, "Stable font-list identity reused")?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let one = root.join("font-lifecycle");
    std::fs::create_dir_all(&one).map_err(|e| e.to_string())?;
    lifecycle(&one)?;
    let two = root.join("font-lists");
    std::fs::create_dir_all(&two).map_err(|e| e.to_string())?;
    lists(&two)
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_font_hash_glyphs_native_missing_reinstall_and_file_immutability() {
        let r = crate::test_support::TempDir::new();
        super::lifecycle(&r.0).unwrap();
    }
    #[test]
    fn font_lists_search_duplicate_confirmation_favorites_restart_and_last_delete() {
        let r = crate::test_support::TempDir::new();
        super::lists(&r.0).unwrap();
    }
}
