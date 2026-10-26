use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    layer_search::Key,
    scene::{Affine, PageObject},
    text_layout::{self as t, Font, Style},
    text_version as v,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<crate::document::Document, String> {
    let mut d = crate::text_layout_verify::fixture()?;
    d.scene.objects.push(PageObject::Text {
        baseline: [12., 30.],
        text: "AV a\u{301} 한글\nsecond".into(),
        size_px: 16.,
        color: [20, 30, 50, 255],
        layout: None,
    });
    let mut s = crate::text_layout_verify::settings();
    s.text = "한글 fi".into();
    s.baseline = [12., 90.];
    s.style.font = Font::Hack;
    let p = t::prepare(&d, None, &s)?;
    d.commit_state(p.state)?;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    movement()?;
    let mut old_style = serde_json::to_value(Style::default()).map_err(|e| e.to_string())?;
    old_style.as_object_mut().unwrap().remove("generation");
    old_style.as_object_mut().unwrap().remove("frame");
    let old_style: Style = serde_json::from_value(old_style).map_err(|e| e.to_string())?;
    check(
        old_style.generation == 1 && old_style.frame.is_identity(),
        "Older style defaults changed",
    )?;
    let mut d = fixture()?;
    let original = d.snapshot();
    let key = Key::Object(d.scene.catalog.objects.id(0));
    let matrix = Affine([1.2, 0.2, 0.4, 0.8, 7., 11.]);
    let legacy = t::Settings::from_object(&d.scene.objects[0]).unwrap();
    let paths = crate::render::legacy_text_paths(legacy.baseline, &legacy.text, legacy.size);
    check(
        t::prepare(&d, Some(d.scene.catalog.objects.id(0)), &legacy)?.state == original,
        "No-op editor silently upgraded original layout",
    )?;
    let moved = crate::group_edits::transform(&d, &[key], matrix)?;
    let s = t::Settings::from_object(&moved.scene.objects[0]).unwrap();
    check(
        s.text == legacy.text
            && s.size == legacy.size
            && s.style.generation == 0
            && s.style.frame == matrix,
        "Transform lost editable text or original layout",
    )?;
    let expected: Vec<Vec<_>> = paths
        .iter()
        .map(|path| path.iter().map(|p| matrix.map(*p)).collect())
        .collect();
    check(
        t::layout(s.baseline, &s.text, s.size, &s.style)?.paths == expected,
        "Legacy glyphs changed during affine transform",
    )?;
    let reflected = crate::group_edits::transform(&d, &[key], Affine([-1., 0., 0., 1., 200., 0.]))?;
    let reflected = t::Settings::from_object(&reflected.scene.objects[0]).unwrap();
    check(
        t::layout(
            reflected.baseline,
            &reflected.text,
            reflected.size,
            &reflected.style,
        )?
        .paths
            == paths
                .iter()
                .map(|path| path.iter().map(|p| [200. - p[0], p[1]]).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        "Editable text mirror geometry",
    )?;
    d.commit_state(moved)?;
    check(d.undo() && d.snapshot() == original, "Transform Undo")?;
    check(d.redo(), "Transform Redo")?;
    // Preserved style generation 1 also composes its own frame exactly under scene transforms.
    d.scene.transform = Affine([1., 0., 0., 1., 3., 4.]);
    let styled = t::Settings::from_object(&d.scene.objects[1]).unwrap();
    let prior = t::layout(styled.baseline, &styled.text, styled.size, &styled.style)?;
    let q =
        crate::group_edits::transform(&d, &[Key::Object(d.scene.catalog.objects.id(1))], matrix)?;
    let next = t::Settings::from_object(&q.scene.objects[1]).unwrap();
    let local = d
        .scene
        .transform
        .then(matrix)
        .then(d.scene.transform.inverse());
    check(
        t::layout(next.baseline, &next.text, next.size, &next.style)?.paths
            == prior
                .paths
                .iter()
                .map(|path| path.iter().map(|p| local.map(*p)).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        "Styled affine composition",
    )?;
    let before = d.snapshot();
    let ids = [d.scene.catalog.objects.id(0)];
    let p = v::prepare(&d, Some(&ids))?;
    check(
        p.changed == 1
            && v::generation(&p.state.scene.objects[0]) == Some(2)
            && v::generation(&p.state.scene.objects[1]) == Some(1)
            && d.snapshot() == before,
        "Selected conversion changed original or unselected text",
    )?;
    let frame = t::Settings::from_object(&p.state.scene.objects[0])
        .unwrap()
        .style
        .frame;
    check(frame == matrix, "Conversion discarded transform")?;
    d.commit_state(p.state)?;
    let mixed = root.join("mixed-text-version.drawer");
    crate::files::save(&mut d, &mixed).map_err(|e| e.to_string())?;
    let mut reopened =
        crate::files::open(&mixed, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.scene == d.scene,
        "Mixed generations/native Unicode/transform changed",
    )?;
    let mut bytes = std::fs::read(&mixed).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&89u32.to_le_bytes());
    let invalid = root.join("invalid-text-version.drawer");
    std::fs::write(&invalid, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&invalid, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted transformed/versioned text",
    )?;
    let old = reopened.snapshot();
    let p = v::prepare(&reopened, None)?;
    check(p.changed == 1, "All-text mixed conversion count")?;
    reopened.commit_state(p.state)?;
    check(
        reopened.undo() && reopened.snapshot() == old,
        "All-text conversion one Undo",
    )?;
    let p = v::prepare(&reopened, Some(&[u64::MAX]));
    check(p.is_err(), "Missing object accepted")?;
    let mut style = Style {
        font: Font::Local(crate::font_library::Reference::new(
            [217; 32],
            0,
            "Absent local face",
        )),
        ..Default::default()
    };
    style = v::upgrade(&style, "한글")?;
    check(
        t::layout([0., 20.], "한글", 16., &style)?
            .notice(style.font)
            .contains("unavailable"),
        "Missing-font conversion notice absent",
    )?;
    let psd = root.join("transformed-text.psd");
    let saved = d.snapshot();
    let history = d.history_counts();
    let dirty = d.dirty;
    check(
        crate::psd::text_loss(&d)
            .is_some_and(|s| s.contains("2 editable text") && s.contains("raster")),
        "PSD text loss notice",
    )?;
    check(
        !crate::psd::export(
            &d,
            &psd,
            crate::psd::Choice::Cancel,
            false,
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?
            && !psd.exists(),
        "Cancelled PSD created file",
    )?;
    check(
        crate::psd::export(
            &d,
            &psd,
            crate::psd::Choice::RenderedLayers,
            false,
            DEFAULT_DOCUMENT_BUDGET,
        )
        .is_err(),
        "Unverified editable PSD layers accepted",
    )?;
    crate::psd::export(
        &d,
        &psd,
        crate::psd::Choice::RenderedImage,
        false,
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let opened = crate::psd::open(&psd, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        opened.layers[0].pixels
            == crate::render::Renderer::new(&d)
                .output(d.dpi, DEFAULT_DOCUMENT_BUDGET)?
                .2,
        "PSD transformed text pixel mismatch",
    )?;
    check(
        d.snapshot() == saved && d.history_counts() == history && d.dirty == dirty,
        "PSD export modified editable original",
    )?;
    // Old editable files remain readable without opt-in conversion.
    let mut old = crate::text_layout_verify::fixture()?;
    let p = t::prepare(&old, None, &crate::text_layout_verify::settings())?;
    old.commit_state(p.state)?;
    let path = root.join("preserved-v89.drawer");
    crate::files::save(&mut old, &path).map_err(|e| e.to_string())?;
    let mut b = std::fs::read(&path).map_err(|e| e.to_string())?;
    b[8..12].copy_from_slice(&89u32.to_le_bytes());
    std::fs::write(&path, b).map_err(|e| e.to_string())?;
    let old_read = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        old_read.scene == old.scene && v::generation(&old_read.scene.objects[0]) == Some(1),
        "Opening v89 automatically relaid out text",
    )?;
    Ok(())
}
pub fn movement() -> Result<(), String> {
    let mut d = fixture()?;
    let key = Key::Object(d.scene.catalog.objects.id(0));
    d.commit_state(crate::group_edits::transform(
        &d,
        &[key],
        Affine([1.2, 0.2, 0.4, 0.8, 7., 11.]),
    )?)?;
    let mut distant = t::Settings::from_object(&d.scene.objects[0]).unwrap();
    distant.style.frame.0[4] += 1000.;
    check(
        distant
            .style
            .bounds(distant.baseline, &distant.text, distant.size)
            .min[0]
            > 900.,
        "Text transform bounds retained untransformed baseline",
    )?;
    let prior = d.snapshot();
    let s = t::Settings::from_object(&d.scene.objects[0]).unwrap();
    let paths = t::layout(s.baseline, &s.text, s.size, &s.style)?.paths;
    let moved = crate::search_move::prepare(&d, key, [5, 7])?;
    let next = t::Settings::from_object(&moved.scene.objects[0]).unwrap();
    let actual = t::layout(next.baseline, &next.text, next.size, &next.style)?.paths;
    let expected: Vec<Vec<_>> = paths
        .iter()
        .map(|path| path.iter().map(|p| [p[0] + 5., p[1] + 7.]).collect())
        .collect();
    check(
        actual.len() == expected.len()
            && actual.iter().zip(expected).all(|(a, b)| {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(p, q)| (0..2).all(|n| (p[n] - q[n]).abs() < 1e-9))
            }),
        "Transformed text movement applied its frame to the movement a second time",
    )?;
    check(d.snapshot() == prior, "Movement preview changed source")?;
    d.commit_state(moved)?;
    check(
        d.undo() && d.snapshot() == prior,
        "Moved transformed text Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn transformed_text_translation_uses_document_axes_once() {
        super::movement().unwrap();
    }
    #[test]
    fn editable_transform_explicit_generation_native_psd_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
