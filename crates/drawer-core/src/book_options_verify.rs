use super::*;
use crate::{
    book_pdf,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use lopdf::{Object, dictionary};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
pub fn annotation_fixture() -> Result<Vec<u8>, String> {
    super::annotations::fixture()
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("book-options-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let pages = crate::book_verify::pages();
        let before = pages.clone();
        check(
            book_pdf::encode_options_budget(&pages, &Output::default(), 1024, &c).is_err(),
            "PDF encoding allocation reserve",
        )?;
        let m = Metadata {
            title: "한글 제목 左右".into(),
            author: "로컬 저자 😀".into(),
            language: "ko-KR".into(),
        };
        let mut jpeg_payloads = std::collections::BTreeSet::new();
        for compression in Compression::ALL {
            for layout in Layout::ALL {
                for right_binding in [false, true] {
                    let options = Output {
                        compression,
                        layout,
                        right_binding,
                        metadata: m.clone(),
                    };
                    let bytes = book_pdf::encode_options(&pages, &options, &c)?;
                    check(metadata(&bytes)? == m, "Unicode PDF metadata round trip")?;
                    let d = lopdf::Document::load_mem(&bytes).map_err(|e| e.to_string())?;
                    let catalog = d.catalog().map_err(|e| e.to_string())?;
                    check(
                        catalog
                            .get(b"PageLayout")
                            .and_then(Object::as_name)
                            .map_err(|e| e.to_string())?
                            == layout.name(right_binding).as_bytes(),
                        "Viewer layout / first-side metadata",
                    )?;
                    let v = catalog
                        .get(b"ViewerPreferences")
                        .and_then(Object::as_dict)
                        .map_err(|e| e.to_string())?;
                    check(
                        v.get(b"Direction")
                            .and_then(Object::as_name)
                            .map_err(|e| e.to_string())?
                            == if right_binding { b"R2L" } else { b"L2R" },
                        "Binding direction metadata",
                    )?;
                    for (n, id) in d.get_pages().into_values().enumerate() {
                        let page = d.get_dictionary(id).map_err(|e| e.to_string())?;
                        let resources = page
                            .get(b"Resources")
                            .and_then(Object::as_dict)
                            .map_err(|e| e.to_string())?;
                        let images = resources
                            .get(b"XObject")
                            .and_then(Object::as_dict)
                            .map_err(|e| e.to_string())?;
                        let image = d
                            .get_object(
                                images
                                    .get(b"PageImage")
                                    .and_then(Object::as_reference)
                                    .map_err(|e| e.to_string())?,
                            )
                            .and_then(Object::as_stream)
                            .map_err(|e| e.to_string())?;
                        let mask = d
                            .get_object(
                                image
                                    .dict
                                    .get(b"SMask")
                                    .and_then(Object::as_reference)
                                    .map_err(|e| e.to_string())?,
                            )
                            .and_then(Object::as_stream)
                            .map_err(|e| e.to_string())?;
                        check(
                            mask.decompressed_content().map_err(|e| e.to_string())?
                                == pages[n]
                                    .image
                                    .pixels
                                    .iter()
                                    .map(|p| p[3])
                                    .collect::<Vec<_>>(),
                            "PDF lossless alpha",
                        )?;
                        if compression == Compression::Zip {
                            check(
                                image.decompressed_content().map_err(|e| e.to_string())?
                                    == pages[n]
                                        .image
                                        .pixels
                                        .iter()
                                        .flat_map(|p| p[..3].iter().copied())
                                        .collect::<Vec<_>>(),
                                "PDF ZIP pixel identity",
                            )?;
                        } else {
                            check(
                                image
                                    .dict
                                    .get(b"Filter")
                                    .and_then(Object::as_name)
                                    .map_err(|e| e.to_string())?
                                    == b"DCTDecode",
                                "PDF JPEG stream marker",
                            )?;
                            let decoded = image::load_from_memory(&image.content)
                                .map_err(|e| e.to_string())?;
                            check(
                                decoded.width() == pages[n].image.width
                                    && decoded.height() == pages[n].image.height,
                                "PDF JPEG dimensions",
                            )?;
                            if n == 0 {
                                jpeg_payloads.insert(image.content.clone());
                            }
                        }
                    }
                }
            }
        }
        check(
            jpeg_payloads.len() == 5 && pages == before,
            "Distinct five JPEG levels / immutable source",
        )?;
        let input = Input {
            right_binding: true,
            from_right: true,
            annotations: true,
        };
        check(
            input.order(4, "1,3-4")? == vec![3, 2, 0] && input.order(1, "0").is_err(),
            "Independent input order / binding",
        )?;
        let bytes = annotation_fixture()?;
        let original = bytes.clone();
        for annotations in [false, true] {
            let options = Input {
                annotations,
                ..Default::default()
            };
            let p = prepare_import(&bytes, "1", &options, &c)?;
            let d = lopdf::Document::load_mem(&p).map_err(|e| e.to_string())?;
            let id = d.get_pages()[&1];
            let page = d.get_dictionary(id).map_err(|e| e.to_string())?;
            check(
                !page.has(b"Annots"),
                "Live annotations survived preparation",
            )?;
            let content = d.get_page_content(id).map_err(|e| e.to_string())?;
            check(
                String::from_utf8_lossy(&content).contains("DrawerAnnotation") == annotations,
                "Annotation image include/exclude content",
            )?;
        }
        check(
            bytes == original
                && prepare_import(&bytes, "1", &input, &AtomicBool::new(true)).is_err(),
            "Annotation cancellation / source bytes",
        )?;
        let mut remote = lopdf::Document::load_mem(&bytes).map_err(|e| e.to_string())?;
        remote.add_object(
            dictionary! {"S"=>"URI","URI"=>Object::string_literal("https://example.invalid/")},
        );
        let mut bad = vec![];
        remote.save_to(&mut bad).map_err(|e| e.to_string())?;
        for annotations in [false, true] {
            check(
                prepare_import(
                    &bad,
                    "1",
                    &Input {
                        annotations,
                        ..Default::default()
                    },
                    &c,
                )
                .is_err(),
                "Remote PDF accepted before annotation option",
            )?;
        }
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let settings = Settings {
            version: 1,
            input,
            output: Output {
                compression: Compression::Low,
                layout: Layout::TwoUpContinuous,
                right_binding: true,
                metadata: m,
            },
        };
        settings.save(&storage)?;
        check(Settings::load(&storage)? == settings, "PDF options restart")?;
        let mut doc = Document::create(
            CanvasSpec {
                width: 96,
                height: 64,
                dpi: 144.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let mut t = crate::text_layout_verify::settings();
        t.text = "PDF text".into();
        t.size = 9.;
        t.baseline = [2., 13.];
        t.style.wrap_width = 80.;
        doc.commit_state(crate::text_layout::prepare(&doc, None, &t)?.state)?;
        let mut balloon = crate::comics::defaults(crate::scene::Rect {
            min: [30., 20.],
            max: [85., 55.],
        })[0]
            .clone();
        balloon.name = "PDF balloon".into();
        if let crate::comic_object::Kind::Balloon(b) = &mut balloon.kind {
            b.tail = crate::comic_object::Tail::None;
        }
        doc.commit_state(crate::comics::prepare(&doc, None, balloon)?)?;
        let state = doc.snapshot();
        let history = doc.history_counts();
        let plan = crate::psd_exchange::prepare(&doc, B, &c)?;
        check(
            plan.partial.is_some()
                && plan.losses.iter().any(|s| s.contains("cached raster"))
                && plan
                    .ranges
                    .iter()
                    .flatten()
                    .any(|s| s.contains("PDF balloon")),
            "Text / balloon raster replacement loss report",
        )?;
        for format in [
            crate::psd_exchange::Format::Psd,
            crate::psd_exchange::Format::Psb,
        ] {
            for merged in [true, false] {
                let ext = if format == crate::psd_exchange::Format::Psd {
                    "psd"
                } else {
                    "psb"
                };
                let path = dir.join(format!("copy-{merged}.{ext}"));
                plan.write_with_preview(&doc, &path, format, false, merged, true, false, B, &c)?;
                let (loaded, warnings, _) = crate::psd_exchange::open_detailed(&path, B)?;
                let actual = book_io::compose(&[&loaded], loaded.dpi, B, &c)?;
                check(
                    crate::psd_exchange::difference(
                        &actual[0].image.pixels,
                        &plan.flattened.merged,
                    ) <= 1,
                    "PSD layers without real preview reconstructed incorrectly",
                )?;
                check(
                    warnings
                        .iter()
                        .any(|w| w.contains("No embedded merged preview"))
                        != merged,
                    "PSD merged-preview availability report",
                )?;
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                check(
                    bytes.windows(6).any(|w| w == [0x04, 0x21, 0, 0, 0, 0]),
                    "PSD Version Info resource missing",
                )?;
            }
        }
        check(
            doc.snapshot() == state && doc.history_counts() == history,
            "PDF / PSD options changed editable source",
        )?;
        let mut copy = doc.clone();
        let native = dir.join("editable.drawer");
        crate::files::save(&mut copy, &native).map_err(|e| e.to_string())?;
        let loaded = crate::files::open(&native, B).map_err(|e| e.to_string())?;
        check(
            loaded.scene.objects == doc.scene.objects,
            "Native text / balloons lost editability",
        )?;
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn pdf_options_static_annotations_unicode_compression_layout_and_psd_preview() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
