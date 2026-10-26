use crate::{
    book_production as book,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    page_work::{Edit, Model},
    scan_batch,
    story_text::Work,
    storyboard,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn scan_image(color: u8) -> scan_batch::Image {
    use image::ImageEncoder;
    let pixels = [color, 30, 60, 255].repeat(12);
    let mut original = vec![];
    image::codecs::png::PngEncoder::new(&mut original)
        .write_image(&pixels, 3, 4, image::ExtendedColorType::Rgba8)
        .unwrap();
    scan_batch::Image {
        image: crate::references::LocalImage::from_rgba(3, 4, &pixels, B).unwrap(),
        original: Arc::new(original),
        filename: "스캔.png".into(),
    }
}
fn blank() -> Document {
    Document::create(
        CanvasSpec {
            width: 60,
            height: 80,
            dpi: 254.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap()
}
pub fn image_resolution() -> Result<(), String> {
    let mut d = blank();
    d.scene
        .objects
        .push(crate::scene::PageObject::Material(Box::new(
            crate::material_layer::Layer {
                source: crate::material_layer::Source::Image(crate::material_layer::Image {
                    width: 2,
                    height: 2,
                    pixels: Arc::new(vec![[80, 90, 100, 255]; 4]),
                }),
                area: crate::scene::Rect {
                    min: [0., 0.],
                    max: [2., 2.],
                },
                transform: crate::scene::Affine([1., 0., 1., 1., 0., 0.]),
                repeat: [false; 2],
                mapping: Default::default(),
            },
        )));
    let work = Work::new(
        "Resolution".into(),
        vec![std::env::temp_dir().join("resolution.drawer")],
    )?;
    let setup = book::Setup {
        minimum_dpi: 170.,
        bleed_mm: 0.,
        safe_mm: 0.,
        ..Default::default()
    };
    let findings = book::preflight(&work, std::slice::from_ref(&d), &setup)?;
    check(
        findings
            .iter()
            .any(|f| f.message.contains("156.98") || f.message.contains("157.0")),
        "Sheared image warning missing: actual maximum-stretch157DPI is below170DPI even when column estimate180DPI passes",
    )?;
    if let crate::scene::PageObject::Material(m) = &mut d.scene.objects[0] {
        m.transform = Default::default();
    }
    check(
        book::preflight(&work, std::slice::from_ref(&d), &setup)?.is_empty(),
        "Identity image false low-DPI warning",
    )?;
    if let crate::scene::PageObject::Material(m) = &mut d.scene.objects[0] {
        m.mapping.warp = crate::material_mapping::Warp::Perspective(crate::material_mapping::UNIT);
    }
    check(
        book::preflight(&work, std::slice::from_ref(&d), &setup)?
            .iter()
            .any(|f| f.message.contains("nonlinear")),
        "Warped material lacks explicit resolution-review warning",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    image_resolution()?;
    let dir = root.join("book-production-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut pages = vec![blank(), blank(), blank()];
        let mut paths = vec![];
        for (n, d) in pages.iter_mut().enumerate() {
            let p = dir.join(format!("원본-{n}.drawer"));
            crate::files::save(d, &p).map_err(|e| e.to_string())?;
            paths.push(p.canonicalize().unwrap());
        }
        let originals = paths
            .iter()
            .map(std::fs::read)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut w = Work::new("한글 스토리보드".into(), paths.clone())?;
        w.page_ids = vec![10, 20, 40];
        w.next_page_id = 80;
        let setup = book::Setup {
            front: Some(10),
            back: Some(40),
            minimum_dpi: 254.,
            bleed_mm: 0.,
            safe_mm: 0.,
            ..Default::default()
        };
        check(
            book::Setup::decode(&setup.encode()?)? == setup,
            "Book setup local round trip",
        )?;
        let mut invalid = setup.clone();
        invalid.version = 2;
        check(!invalid.valid(), "Unknown setup version")?;
        invalid = setup.clone();
        invalid.back = Some(10);
        check(invalid.validate(&w).is_err(), "Duplicate cover")?;
        invalid.back = Some(999);
        check(invalid.validate(&w).is_err(), "Missing cover")?;
        check(
            book::preflight(&w, &pages, &setup)?.is_empty(),
            "Blank page physical preflight",
        )?;
        pages[1].dpi = 72.;
        let findings = book::preflight(&w, &pages, &setup)?;
        check(
            findings
                .iter()
                .any(|f| f.page == 20 && f.message.contains("resolution")),
            "Low DPI warning",
        )?;
        pages[1].dpi = 254.;
        pages[1].scene.comics.page.trim_mm = [100., 200.];
        check(
            book::preflight(&w, &pages, &setup)?
                .iter()
                .any(|f| f.blocking),
            "Trim outside page blocks output",
        )?;
        pages[1].scene.comics.page.trim_mm = [0.; 2];
        pages[1].scene.objects.push(crate::scene::PageObject::Text {
            layout: None,
            baseline: [-30., 5.],
            text: "대사".into(),
            size_px: 8.,
            color: [0, 0, 0, 255],
        });
        check(
            book::preflight(&w, &pages, &setup)?
                .iter()
                .any(|f| f.message.contains("Dialogue")),
            "Dialogue crosses safe bounds",
        )?;
        for right_binding in [false, true] {
            for first_right in [false, true] {
                w.right_binding = right_binding;
                w.first_right = first_right;
                let model = Model::new(w.clone())?;
                for index in 0..3 {
                    let slots = book::spread(&w, index)?;
                    check(
                        slots.into_iter().flatten().any(|n| n == index),
                        "Selected page missing from spread",
                    )?;
                    check(
                        slots[usize::from(first_right == index.is_multiple_of(2))] == Some(index),
                        "Page on wrong visual side",
                    )?;
                    check(
                        slots.into_iter().flatten().find(|n| *n != index) == model.facing(index),
                        "Facing order mismatch",
                    )?;
                    for closed in [false, true] {
                        for yaw in [-75., 0., 75.] {
                            let faces = book::preview(
                                &w,
                                &setup,
                                index,
                                closed,
                                170.,
                                yaw,
                                -45.,
                                [150., 200.],
                            )?;
                            check(
                                faces.len() == 3
                                    && faces.iter().any(|f| f.spine)
                                    && faces
                                        .iter()
                                        .flat_map(|f| f.points)
                                        .flatten()
                                        .all(f64::is_finite),
                                "Perspective covers/pages/spine invalid",
                            )?;
                            check(
                                faces.windows(2).all(|f| f[0].depth <= f[1].depth),
                                "Preview depth ordering",
                            )?;
                        }
                    }
                }
            }
        }
        check(
            book::preview(&w, &setup, 0, false, 170., f64::MAX, 0., [150., 200.]).is_err(),
            "Invalid preview camera accepted",
        )?;
        // Add a typed frame; the JSON can edit existing dialogue/frame objects.
        let frame = crate::comics::defaults(crate::scene::Rect {
            min: [1., 1.],
            max: [55., 75.],
        })
        .into_iter()
        .find(|o| matches!(o.kind, crate::comics::Kind::Frame(_)))
        .unwrap();
        let state = crate::comics::prepare(&pages[1], None, frame)?;
        pages[1].commit_state(state)?;
        let output = dir.join("교환.drawer-storyboard");
        let cancel = AtomicBool::new(false);
        let writer = &mut |a: &Path, b: &Path| std::fs::hard_link(a, b).map_err(|e| e.to_string());
        storyboard::export(&output, &w, &pages, &setup, B, &cancel, writer)?;
        let encoded = std::fs::read(&output).unwrap();
        let mut data: storyboard::File = serde_json::from_slice(&encoded).unwrap();
        let prepared = storyboard::decode(&output, B, &cancel)?;
        prepared.fresh()?;
        check(
            prepared.file.pages.iter().map(|p| p.id).collect::<Vec<_>>() == w.page_ids
                && prepared.file.pages[1].frames.len() == 1
                && prepared.file.pages[1].dialogues.len() == 1,
            "Storyboard IDs/order/objects lost",
        )?;
        let round = prepared.work(paths.clone())?;
        check(
            round.page_ids == w.page_ids
                && round.right_binding == w.right_binding
                && round.first_right == w.first_right,
            "Storyboard book binding lost",
        )?;
        if let crate::scene::PageObject::Text { text, .. } = &mut data.pages[1].dialogues[0].object
        {
            *text = "교환된 대사\n두 번째 줄".into();
        }
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        let edited = storyboard::decode(&output, B, &cancel)?;
        check(
            matches!(&edited.pages[1].scene.objects[0],crate::scene::PageObject::Text{text,..} if text.contains("두 번째")),
            "Storyboard dialogue edits ignored",
        )?;
        let valid = data.clone();
        data.pages[1].frames[0].id = 999;
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        check(
            storyboard::decode(&output, B, &cancel).is_err(),
            "Missing frame ID accepted",
        )?;
        data = valid.clone();
        data.pages[1].image.path = "누락.png".into();
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        check(
            storyboard::decode(&output, B, &cancel).is_err(),
            "Missing image accepted",
        )?;
        data = valid.clone();
        data.pages[1].image.path = "https://invalid.example/a.png".into();
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        check(
            storyboard::decode(&output, B, &cancel).is_err(),
            "Remote reference accepted",
        )?;
        data = valid.clone();
        data.pages[1].id = 10;
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        check(
            storyboard::decode(&output, B, &cancel).is_err(),
            "Duplicate page ID accepted",
        )?;
        data = valid;
        data.version = 2;
        std::fs::write(&output, serde_json::to_vec(&data).unwrap()).unwrap();
        check(
            storyboard::decode(&output, B, &cancel).is_err(),
            "Unknown storyboard version accepted",
        )?;
        check(
            storyboard::decode(&dir.join("external.cst"), B, &cancel).is_err(),
            "Unknown external format accepted",
        )?;
        std::fs::write(&output, &encoded).unwrap();
        check(
            storyboard::decode(&output, 64, &cancel).is_err(),
            "Storyboard memory budget ignored",
        )?;
        let names = || {
            std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        cancel.store(true, Ordering::Relaxed);
        check(
            storyboard::export(
                &dir.join("cancel.drawer-storyboard"),
                &w,
                &pages,
                &setup,
                B,
                &cancel,
                writer,
            )
            .is_err()
                && names() == before,
            "Cancelled exchange left resources",
        )?;
        cancel.store(false, Ordering::Relaxed);
        check(
            storyboard::export(&output, &w, &pages, &setup, B, &cancel, writer).is_err()
                && names() == before
                && std::fs::read(&output).unwrap() == encoded,
            "Existing storyboard overwritten or resources leaked",
        )?;
        check(
            storyboard::export(
                &dir.join("failed.drawer-storyboard"),
                &w,
                &pages,
                &setup,
                B,
                &cancel,
                &mut |_, _| Err("mock publication failure".into()),
            )
            .is_err()
                && names() == before,
            "Failed publication leaked page/image resources",
        )?;
        check(
            book::checked_pdf(&w, &pages, &setup, false, B, &cancel).is_err(),
            "Unreviewed safe-area warnings allowed PDF output",
        )?;
        let pdf = book::checked_pdf(&w, &pages, &setup, true, B, &cancel)?;
        check(pdf.starts_with(b"%PDF-"), "Checked book PDF invalid")?;
        let mut broken = pages.clone();
        broken[1].scene.comics.page.trim_mm = [900., 900.];
        check(
            book::checked_pdf(&w, &broken, &setup, true, B, &cancel).is_err(),
            "Acknowledgement bypassed blocking trim error",
        )?;
        let mut scans = scan_batch::Session::new(3, 300., B)?;
        scans.accept(scan_image(10))?;
        scans.fail("MOCK: slot2 driver failure".into());
        check(
            scans.slot == 1 && scans.accept(scan_image(20)).is_err(),
            "Failed scan did not pause at slot2",
        )?;
        scans.skip()?;
        scans.accept(scan_image(30))?;
        check(
            scans.done() && scans.accepted.len() == 2 && scans.skipped == vec![1],
            "Mock three scans / second failure / skip",
        )?;
        scans.bind(&w)?;
        check(
            scans.accepted[0].document.scene.comics.page.number == 4
                && scans.accepted[1].document.scene.comics.page.total == 5
                && scans.accepted[1].document.scene.comics.page.right_binding == w.right_binding,
            "Scan result book metadata incorrect",
        )?;
        let owned = scans.prepare(&dir, &cancel)?;
        let saved = owned.paths.clone();
        let scan_dir = saved[0].parent().unwrap().to_path_buf();
        check(
            std::fs::read(scan_dir.join("scan-0001.png")).unwrap() == *scans.accepted[0].original,
            "Original scan bytes changed",
        )?;
        let mut model = Model::new(w.clone())?;
        model.edit(Edit::Batch {
            paths: saved.clone(),
            remove: vec![],
            index: 3,
        })?;
        owned.keep();
        check(
            model.work.pages.len() == 5
                && model.undo()
                && model.work.pages == w.pages
                && model.work.page_ids == w.page_ids
                && model.work.right_binding == w.right_binding
                && model.work.first_right == w.first_right
                && saved.iter().all(|p| p.exists())
                && model.redo(),
            "Scan batch one Undo/Redo/source preservation",
        )?;
        let mut retry = scan_batch::Session::new(3, 300., B)?;
        retry.accept(scan_image(10))?;
        retry.fail("MOCK second scan".into());
        retry.retry()?;
        retry.accept(scan_image(20))?;
        retry.accept(scan_image(30))?;
        check(
            retry.done()
                && retry.accepted.iter().map(|p| p.slot).collect::<Vec<_>>() == vec![0, 1, 2],
            "Mock retry altered slot order",
        )?;
        let before = names();
        cancel.store(true, Ordering::Relaxed);
        check(
            retry.prepare(&dir, &cancel).is_err() && names() == before,
            "Cancelled scan publication left resources",
        )?;
        retry.cancel();
        check(
            !retry.done() && retry.accepted.is_empty(),
            "Mid-batch abort retained pages",
        )?;
        for (p, b) in paths.iter().zip(&originals) {
            check(
                std::fs::read(p).unwrap() == *b,
                "Original source changed during production",
            )?;
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn sheared_image_preflight_uses_maximum_stretch() {
        super::image_resolution().unwrap();
    }
    #[test]
    fn local_book_preflight_storyboard_and_mock_scan() {
        let root =
            std::env::temp_dir().join(format!("drawer-book-production-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::verify(&root);
        std::fs::remove_dir(root).unwrap();
        r.unwrap();
    }
}
