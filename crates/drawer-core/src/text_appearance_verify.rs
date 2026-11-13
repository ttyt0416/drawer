use super::*;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    text_layout as t,
};
fn pixels(d: &Document) -> Vec<[u8; 4]> {
    let r = crate::render::Renderer::new(d);
    (0..d.height)
        .flat_map(|y| (0..d.width).map(move |x| (x, y)))
        .map(|(x, y)| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect()
}
#[test]
fn local_font_disappearance_preserves_confirmed_pixels_native_affine_and_replacement_undo() {
    let root = crate::test_support::TempDir::new();
    let fonts = crate::font_library_verify::fixture(&root.0).unwrap();
    let entry = fonts
        .entries
        .iter()
        .find(|e| matches!(e.font, t::Font::Local(_)) && e.name.to_lowercase().contains("ubuntu"))
        .unwrap();
    for rich in [false, true] {
        let mut d = crate::text_layout_verify::fixture().unwrap();
        let mut s = crate::text_layout_verify::settings();
        s.text = "AB 123".into();
        s.style.font = entry.font;
        if rich {
            s.style.rich = Some(Box::default());
            s.style.ruby.push(t::Ruby {
                start: 0,
                end: 2,
                text: "ab".into(),
                font: entry.font,
                ..Default::default()
            });
        }
        let prepared = t::prepare(&d, None, &s).unwrap();
        let id = prepared.id;
        d.commit_state(prepared.state).unwrap();
        assert!(d.scene.text_appearance.has_snapshot(id));
        let expected = pixels(&d);
        assert!(expected.iter().any(|p| p[3] > 0));
        let file = root.0.join(format!("cache-{rich}.drawer"));
        crate::files::save(&mut d, &file).unwrap();
        let stable = d.snapshot();
        let font_path = entry.path.as_ref().unwrap();
        let font_bytes = std::fs::read(font_path).unwrap();
        std::fs::remove_file(font_path).unwrap();
        crate::text_layout::invalidate_font_data();
        assert_eq!(pixels(&d), expected);
        let mut reopened = crate::files::open(&file, B).unwrap();
        assert_eq!(reopened.snapshot(), stable);
        assert_eq!(pixels(&reopened), expected);
        assert_eq!(reopened.scene.object_bounds(0), d.scene.object_bounds(0));
        let moved =
            crate::search_move::prepare(&reopened, crate::layer_search::Key::Object(id), [7, 5])
                .unwrap();
        reopened.commit_state(moved).unwrap();
        let shifted = pixels(&reopened);
        for y in 0..155 {
            for x in 0..233 {
                assert_eq!(expected[y * 240 + x], shifted[(y + 5) * 240 + x + 7]);
            }
        }
        assert!(reopened.undo());
        assert_eq!(reopened.snapshot(), stable);
        let history = reopened.history_counts();
        let mut edit = s.clone();
        edit.text.push('C');
        assert!(t::prepare(&reopened, Some(id), &edit).is_err());
        assert_eq!(reopened.snapshot(), stable);
        assert_eq!(reopened.history_counts(), history);
        let matrix = Affine([1., 0., 0., 1., 7., 5.]);
        let moved = crate::group_edits::transform(
            &reopened,
            &[crate::layer_search::Key::Object(id)],
            matrix,
        )
        .unwrap();
        reopened.commit_state(moved).unwrap();
        let shifted = pixels(&reopened);
        for y in 0..155 {
            for x in 0..233 {
                assert_eq!(expected[y * 240 + x], shifted[(y + 5) * 240 + x + 7]);
            }
        }
        assert!(reopened.undo());
        assert_eq!(reopened.snapshot(), stable);
        edit.style.font = t::Font::Hack;
        edit.style.ruby.clear();
        reopened
            .commit_state(t::prepare(&reopened, Some(id), &edit).unwrap().state)
            .unwrap();
        assert_ne!(pixels(&reopened), expected);
        assert!(reopened.undo());
        assert_eq!(reopened.snapshot(), stable);
        assert_eq!(pixels(&reopened), expected);
        let mut legacy = std::fs::read(&file).unwrap();
        legacy.truncate(legacy.len() - 64);
        legacy[8..12].copy_from_slice(&125u32.to_le_bytes());
        let mut rebuilt = Vec::new();
        let mut writer = crate::native_integrity::Writer::new(&mut rebuilt);
        writer.write_all(&legacy).unwrap();
        writer.finish(0, 0).unwrap();
        let legacy = rebuilt;
        let forged = root.0.join(format!("forged-{rich}.drawer"));
        std::fs::write(&forged, legacy).unwrap();
        let error = crate::files::open(&forged, B).err().unwrap().to_string();
        assert!(
            error.contains("Confirmed text appearance requires native version126"),
            "{error}"
        );
        std::fs::write(font_path, font_bytes).unwrap();
        crate::text_layout::invalidate_font_data();
    }
}
#[test]
fn bounded_cache_rejects_tampering_counts_nonfinite_metadata_and_stale_inputs() {
    let mut d = crate::text_layout_verify::fixture().unwrap();
    d.commit_state(
        t::prepare(&d, None, &crate::text_layout_verify::settings())
            .unwrap()
            .state,
    )
    .unwrap();
    validate_native(&d.scene, B).unwrap();
    let stable = d.snapshot();
    let id = d.scene.catalog.objects.id(0);
    let cached = d.scene.text_appearance.entries.get(&id).unwrap().clone();
    for kind in 0..4 {
        let mut c = cached.clone();
        c.decoded = OnceLock::new();
        match kind {
            0 => c.raw_sha256 = "0".repeat(64),
            1 => c.raw_bytes = MAX_RAW + 1,
            2 => c.heap_bytes += 1,
            _ => c.packed.push('!'),
        }
        d.scene.text_appearance.entries.insert(id, c);
        assert!(validate_native(&d.scene, B).is_err());
    }
    d.scene = stable.scene.clone();
    if let PageObject::Text { text, .. } = &mut d.scene.objects[0] {
        text.push('x');
    }
    assert!(validate_native(&d.scene, B).is_err());
    let mut raw = encode(&cached.geometry().unwrap()).unwrap();
    raw[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&raw).is_err());
    let mut raw = encode(&cached.geometry().unwrap()).unwrap();
    raw[25..33].copy_from_slice(&f64::NAN.to_le_bytes());
    assert!(decode(&raw).is_err());
    assert!(validate_native(&stable.scene, 0).is_err());
}
#[test]
fn uncached_legacy_noop_does_not_rewrite_text_generation() {
    let mut d = crate::text_layout_verify::fixture().unwrap();
    d.scene.objects.push(PageObject::Text {
        baseline: [12., 30.],
        text: "old".into(),
        size_px: 18.,
        color: [0, 0, 0, 255],
        layout: None,
    });
    let stable = d.snapshot();
    let s = t::Settings::from_object(&d.scene.objects[0]).unwrap();
    let id = d.scene.catalog.objects.id(0);
    assert_eq!(t::prepare(&d, Some(id), &s).unwrap().state, stable);
    assert!(d.scene.text_appearance.is_default());
}
