use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    drawer_core::book_options::verify::run(root)?;
    drawer_platform::book_pdf::verify_options()?;
    let dir = root.join("book-options-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut a = Drawer::new(&ctx, None).with_storage(storage.clone());
        a.simple_view = true;
        a.create_canvas(CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let before = a.documents[0].snapshot();
        let history = a.documents[0].history_counts();
        a.begin_book_io()?;
        let mut time = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .book_io
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing PDF option {key}"))?;
            if r.center().y > 690. {
                return Err(format!("PDF option clipped: {key} {r:?}"));
            }
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            Ok(())
        };
        click(&mut a, &mut time, "pdf-options")?;
        click(&mut a, &mut time, "right-binding")?;
        click(&mut a, &mut time, "from-right")?;
        click(&mut a, &mut time, "annotations")?;
        click(&mut a, &mut time, "compression")?;
        click(&mut a, &mut time, "JPEG (Low)")?;
        click(&mut a, &mut time, "layout")?;
        click(&mut a, &mut time, "Two-Up Continuous")?;
        a.book_io.title = "한글 PDF".into();
        a.book_io.author = "로컬 저자".into();
        a.book_io.language = "ko".into();
        a.book_io.right_binding = true;
        click(&mut a, &mut time, "save-pdf")?;
        let saved = drawer_core::book_options::Settings::load(&storage)?;
        if !saved.input.annotations
            || !saved.input.from_right
            || !saved.input.right_binding
            || saved.output.compression != drawer_core::book_options::Compression::Low
            || saved.output.layout != drawer_core::book_options::Layout::TwoUpContinuous
            || saved.output.metadata.title != "한글 PDF"
        {
            return Err(format!(
                "PDF input/output pointer settings: {}",
                a.book_io.notice
            ));
        }
        let mut restarted = Drawer::new(&ctx, None).with_storage(storage);
        restarted.begin_book_io()?;
        if restarted.book_io.pdf != saved {
            return Err("PDF presets restart".into());
        }
        drop(restarted);
        let source = dir.join("source.pdf");
        let bytes = drawer_core::book_options::verify::annotation_fixture()?;
        std::fs::write(&source, &bytes).map_err(|e| e.to_string())?;
        a.book_io.path = source.display().to_string();
        a.book_io.import_range = "1,3".into();
        a.book_io.dpi = 96.;
        a.start_book_import(&ctx)?;
        for _ in 0..8000 {
            a.quiesce_book_io(&ctx);
            if a.book_io.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (_, pages) = a
            .book_io
            .prepared
            .as_ref()
            .ok_or_else(|| a.book_io.notice.clone())?;
        if pages.len() != 2 || pages[0].image.height != 21 || pages[1].image.height != 24 {
            return Err("PDF from-right selected-page preparation".into());
        }
        a.apply_book_pages()?;
        if a.documents.len() != 3
            || !a.documents[1].scene.comics.page.right_binding
            || a.documents[0].snapshot() != before
            || a.documents[0].history_counts() != history
            || std::fs::read(&source).map_err(|e| e.to_string())? != bytes
        {
            return Err("PDF import changed source / omitted binding".into());
        }
        a.book_io.author = "로컬 저자".into();
        a.book_io.title = "한글 PDF".into();
        a.book_io.language = "ko".into();
        a.book_io.export_range = "2-3".into();
        let output = dir.join("options.pdf");
        a.start_book_output(&ctx, 1, Some(output.clone()))?;
        for _ in 0..8000 {
            a.quiesce_book_io(&ctx);
            if a.book_io.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let out = std::fs::read(&output).map_err(|e| format!("{e}: {}", a.book_io.notice))?;
        if drawer_core::book_options::metadata(&out)?.author != "로컬 저자" {
            return Err("PDF output metadata job".into());
        }
        // The real PSD review checkbox invalidates prior acceptance.
        let psd = dir.join("review.psd");
        a.begin_psd_exchange(psd)?;
        for _ in 0..8000 {
            frame(&mut a, &mut time, vec![]);
            if a.textures.export.as_ref().is_some_and(|e| e.plan.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        a.book_io.open = false;
        for _ in 0..3 {
            frame(&mut a, &mut time, vec![]);
        }
        let e = a.textures.export.as_mut().ok_or("Missing PSD review")?;
        e.confirmed = true;
        let pos = e
            .controls
            .get("merged-preview")
            .ok_or("Missing merged preview checkbox")?
            .center();
        for pressed in [true, false] {
            frame(
                &mut a,
                &mut time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        if a.textures
            .export
            .as_ref()
            .is_none_or(|e| e.merged_preview || e.confirmed)
        {
            return Err("PSD merged checkbox / review invalidation".into());
        }
        a.textures.export = None;
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn pdf_pointer_presets_annotations_order_import_and_psd_preview_review() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
