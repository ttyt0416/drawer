use crate::{
    comics::{self, Kind},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    page_batch,
    page_work::{Edit, Model},
    render::Renderer,
    scene::{PageObject, Rect},
    story_text::Work,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
fn page() -> Document {
    Document::create(
        CanvasSpec {
            width: 32,
            height: 64,
            dpi: 300.,
            background: Background::Color([20, 80, 160]),
        },
        B,
    )
    .unwrap()
}
pub fn frames() -> Result<(), String> {
    let mut d = page();
    let mut frame = comics::defaults(Rect {
        min: [0.; 2],
        max: [32., 64.],
    })[1]
        .clone();
    let Kind::Frame(f) = &mut frame.kind else {
        unreachable!()
    };
    f.children = vec![crate::layer_search::Key::Layer(
        crate::layer_properties::Target::Raster(1),
    )];
    f.ink = [255, 0, 0, 255];
    d.commit_state(comics::prepare(&d, None, frame)?)?;
    let id = d.scene.catalog.objects.id(0);
    let before = d.snapshot();
    let mut u = page();
    let mut shape = comics::defaults(Rect {
        min: [0.; 2],
        max: [30.; 2],
    })[1]
        .clone();
    let Kind::Frame(f) = &mut shape.kind else {
        unreachable!()
    };
    f.polygon = vec![
        [0., 0.],
        [10., 0.],
        [10., 20.],
        [20., 20.],
        [20., 0.],
        [30., 0.],
        [30., 30.],
        [0., 30.],
    ];
    f.children = vec![crate::layer_search::Key::Layer(
        crate::layer_properties::Target::Raster(1),
    )];
    u.commit_state(comics::prepare(&u, None, shape)?)?;
    let id_u = u.scene.catalog.objects.id(0);
    let halves = crate::frame_ops::divide(&u, id_u, 2, false)?;
    let split_u = crate::layer_merge::preview(&halves, 0)?;
    let r = Renderer::new(&split_u);
    check(
        r.pixel([5.5, 5.5])[3] > 0 && r.pixel([15.5, 5.5]) == [0; 4] && r.pixel([25.5, 5.5])[3] > 0,
        "Concave equal division lost disconnected components",
    )?;
    let divided = crate::frame_ops::divide(&d, id, 2, true)?;
    check(
        d.snapshot() == before && divided.scene.objects.len() == 2,
        "Frame division preview changed original",
    )?;
    d.commit_state(divided)?;
    let ids = (0..2)
        .map(|n| d.scene.catalog.objects.id(n))
        .collect::<Vec<_>>();
    let original = d.snapshot();
    check(
        Renderer::new(&d).pixel([15.5, 30.5]) == [255, 0, 0, 255],
        "New dividing border missing",
    )?;
    d.commit_state(crate::frame_ops::hide_overlaps(&d, &ids, true)?)?;
    check(
        Renderer::new(&d).pixel([15.5, 30.5]) == [20, 80, 160, 255],
        "Coincident frame border remained visible",
    )?;
    let restored = crate::frame_ops::hide_overlaps(&d, &ids, false)?;
    check(
        restored == original,
        "Restored borders changed geometry/content",
    )?;
    d.undo();
    check(d.snapshot() == original, "Border hiding was not one Undo")?;
    let merged = crate::frame_ops::merge(&d, &ids)?;
    d.commit_state(merged)?;
    check(
        d.scene.objects.len() == 1 && Renderer::new(&d).pixel([15.5, 30.5]) == [20, 80, 160, 255],
        "Compound frame merge retained internal edge",
    )?;
    let PageObject::Comic(o) = &d.scene.objects[0] else {
        unreachable!()
    };
    let Kind::Frame(f) = &o.kind else {
        unreachable!()
    };
    check(
        f.compound.len() == 1
            && f.children.len() == 1
            && crate::frame_ops::contains(f, [2., 2.])
            && crate::frame_ops::contains(f, [30., 60.]),
        "Compound frame union/content scope",
    )?;
    d.commit_state(crate::frame_ops::divide(&d, ids[0], 4, true)?)?;
    check(
        d.scene.objects.len() == 4 && d.scene.valid(),
        "Equal division of combined contours failed",
    )?;
    d.undo();
    d.undo();
    check(
        d.snapshot() == original,
        "Merge/division Undo lost original IDs or children",
    )?;
    d.undo();
    check(d.snapshot() == before, "First equal division Undo")?;
    let divided = crate::frame_ops::divide(&d, id, 2, true)?;
    let mut clipped = crate::layer_merge::preview(&divided, 0)?;
    let ids = (0..2)
        .map(|n| clipped.scene.catalog.objects.id(n))
        .collect::<Vec<_>>();
    clipped.scene.comics.masks.push(crate::comics::Mask {
        target: crate::layer_search::Key::Object(ids[0]),
        polygon: vec![[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
        space: Default::default(),
    });
    let combined = crate::frame_ops::merge(&clipped, &ids)?;
    let joined = crate::layer_merge::preview(&combined, 0)?;
    check(
        Renderer::new(&joined).pixel([5.5, 50.5]) == [0; 4]
            && Renderer::new(&joined).pixel([25.5, 50.5])[3] > 0,
        "Combining masked frames exposed clipped source content",
    )?;
    let quarters = crate::frame_ops::divide(&d, id, 4, true)?;
    let quarters = crate::layer_merge::preview(&quarters, 0)?;
    let subset = (2..4)
        .map(|n| quarters.scene.catalog.objects.id(n))
        .collect::<Vec<_>>();
    let subset = crate::frame_ops::merge(&quarters, &subset)?;
    let subset = crate::layer_merge::preview(&subset, 0)?;
    let r = Renderer::new(&subset);
    check(
        r.pixel([4.5, 30.5])[3] > 0 && r.pixel([27.5, 30.5])[3] > 0,
        "Combining only some sibling frames hid untouched source regions",
    )?;
    let maximum = crate::frame_ops::divide(&d, id, 32, true)?;
    check(
        maximum.scene.objects.len() == 32 && maximum.scene.valid(),
        "Maximum equal division count",
    )?;
    let max_doc = crate::layer_merge::preview(&maximum, 0)?;
    let max_ids = (0..32)
        .map(|n| max_doc.scene.catalog.objects.id(n))
        .collect::<Vec<_>>();
    let merged_max = crate::frame_ops::merge(&max_doc, &max_ids)?;
    check(
        merged_max.scene.objects.len() == 1 && merged_max.scene.valid(),
        "Maximum frame union count",
    )?;
    check(
        crate::frame_ops::divide(&d, id, 1, true).is_err()
            && crate::frame_ops::divide(&d, id, 33, false).is_err()
            && crate::frame_ops::merge(&d, &[]).is_err()
            && crate::frame_ops::merge(&d, &[id, id]).is_err(),
        "Invalid/duplicate frame operation accepted",
    )
}
fn png(path: &Path, color: [u8; 4]) -> Result<(), String> {
    let i = crate::references::LocalImage {
        width: 2,
        height: 3,
        pixels: vec![color; 6],
    };
    let d = i.document(B).map_err(|e| e.to_string())?;
    crate::png_export::export(&d, path, B).map_err(|e| e.to_string())
}
fn phys(bytes: &[u8], density: u32) -> Vec<u8> {
    let mut data = b"pHYs".to_vec();
    data.extend(density.to_be_bytes());
    data.extend(density.to_be_bytes());
    data.push(1);
    let mut crc = 0xffffffffu32;
    for b in &data {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    let mut chunk = 9u32.to_be_bytes().to_vec();
    chunk.extend(data);
    chunk.extend((!crc).to_be_bytes());
    let mut out = bytes[..33].to_vec();
    out.extend(chunk);
    out.extend(&bytes[33..]);
    out
}
pub fn files(root: &Path) -> Result<(), String> {
    let dir = root.join("page-structure-files");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut native = vec![];
        for n in 0..5 {
            let mut d = page();
            d.dpi = 150. + n as f64 * 150.;
            if n == 0 {
                d.scene.comics.page.trim_mm = [2., 4.];
                d.scene.comics.page.bleed_mm = 0.2;
                d.scene.comics.page.safe_mm = 0.1;
            }
            let path = dir.join(format!("native-{n}.drawer"));
            crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            native.push(path.canonicalize().unwrap());
        }
        let mut m = Model::new(Work::new("Binding".into(), native[..2].to_vec())?)?;
        m.work.right_binding = true;
        m.work.first_right = true;
        let original = m.work.clone();
        let count = m.counts();
        check(
            m.edit(Edit::Batch {
                paths: vec![native[2].clone(), native[2].clone()],
                remove: vec![],
                index: 1,
            })
            .is_err()
                && m.work == original
                && m.counts() == count,
            "Duplicate batch partially changed work",
        )?;
        m.edit(Edit::Batch {
            paths: native[2..4].to_vec(),
            remove: vec![original.page_ids[0]],
            index: 0,
        })?;
        check(
            m.work.pages.len() == 3 && m.work.right_binding && m.work.first_right,
            "Batch binding/order",
        )?;
        let batch = m.work.clone();
        m.undo();
        check(
            m.work.pages == original.pages && m.work.page_ids == original.page_ids,
            "Batch did not restore all pages with one Undo",
        )?;
        m.redo();
        check(
            m.work.pages == batch.pages && m.work.page_ids == batch.page_ids,
            "Batch Redo IDs changed",
        )?;
        m.undo();
        let replaced = original.page_ids[0];
        m.edit(Edit::Replace {
            id: replaced,
            path: native[4].clone(),
        })?;
        check(
            m.work.pages[0] == native[4] && m.work.page_ids[0] != replaced,
            "Replace reused the old source identity",
        )?;
        m.undo();
        check(
            m.work.pages == original.pages && m.work.page_ids == original.page_ids,
            "Replace Undo source/order",
        )?;
        let image2 = dir.join("image2.png");
        let image10 = dir.join("image10.png");
        png(&image2, [100, 40, 20, 128])?;
        png(&image10, [20, 40, 100, 255])?;
        let data = std::fs::read(&image2).unwrap();
        std::fs::write(&image2, phys(&data, 11811)).unwrap();
        check(
            (page_batch::image_dpi(&std::fs::read(&image2).unwrap(), 0)? - 300.).abs() < 0.01,
            "PNG physical DPI",
        )?;
        let bad = dir.join("broken.png");
        std::fs::write(&bad, b"broken").unwrap();
        let mut paths = vec![image10.clone(), image2.clone()];
        page_batch::natural_sort(&mut paths);
        check(
            paths == vec![image2.clone(), image10.clone()],
            "Natural page ordering",
        )?;
        let inputs = vec![
            image10.clone(),
            bad.clone(),
            native[0].clone(),
            image2.clone(),
            image2.clone(),
        ];
        let cancel = AtomicBool::new(false);
        let prepared = page_batch::prepare(
            &inputs,
            page_batch::Size::Original,
            B,
            &cancel,
            &mut |_, _, _| Err("PDF adapter not used".into()),
        )?;
        check(
            prepared.pages.len() == 3
                && prepared.rejected.len() == 2
                && prepared.pages[0].layers[0].pixels[0] == [20, 40, 100, 255]
                && prepared.pages[1].dpi == 150.
                && (prepared.pages[2].dpi - 300.).abs() < 0.01,
            "Manual order, corrupt/duplicate input or mixed DPI",
        )?;
        prepared.fresh()?;
        let fit = page_batch::prepare(
            std::slice::from_ref(&image2),
            page_batch::Size::Fit {
                width: 8,
                height: 12,
                dpi: 600.,
            },
            B,
            &cancel,
            &mut |_, _, _| unreachable!(),
        )?;
        check(
            fit.pages.len() == 1
                && fit.pages[0].width == 8
                && fit.pages[0].height == 12
                && fit.pages[0].dpi == 600.,
            "Batch fit dimensions/DPI",
        )?;
        let fitted = page_batch::prepare(
            std::slice::from_ref(&native[0]),
            page_batch::Size::Fit {
                width: 8,
                height: 12,
                dpi: 600.,
            },
            B,
            &cancel,
            &mut |_, _, _| unreachable!(),
        )?;
        check(
            fitted.pages.len() == 1
                && (fitted.pages[0].scene.comics.page.trim_mm[0] - 0.125).abs() < 1e-9
                && fitted.pages[0]
                    .scene
                    .comics
                    .page
                    .guides(&fitted.pages[0])
                    .is_ok(),
            "Fit import left original trim outside the new physical page",
        )?;
        std::fs::write(&image10, b"changed").unwrap();
        check(prepared.fresh().is_err(), "Changed source batch was fresh")?;
        check(
            page_batch::prepare(
                &inputs,
                page_batch::Size::Original,
                B,
                &AtomicBool::new(true),
                &mut |_, _, _| unreachable!(),
            )
            .is_err(),
            "Batch cancel ignored",
        )?;
        let tiny = page_batch::prepare(
            &[native[0].clone()],
            page_batch::Size::Original,
            8,
            &cancel,
            &mut |_, _, _| unreachable!(),
        )?;
        check(
            tiny.pages.is_empty() && !tiny.rejected.is_empty(),
            "Batch memory limit ignored",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
pub fn verify(root: &Path) -> Result<(), String> {
    frames()?;
    files(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn frame_union_equal_division_coincident_borders_child_masks_and_one_undo() {
        super::frames().unwrap();
    }
    #[test]
    fn atomic_page_replace_batch_order_dpi_fit_rejected_sources_cancel_and_reopen() {
        let root = crate::test_support::TempDir::new();
        super::files(&root.0).unwrap();
    }
}
