use crate::{
    book_epub,
    document::DEFAULT_DOCUMENT_BUDGET as B,
    epub_metadata::{self, Metadata},
    shot_exchange as shots,
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
fn invalid_cover(bytes: &[u8]) -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(vec![]));
    for n in 0..input.len() {
        let mut f = input.by_index(n).unwrap();
        let name = f.name().to_string();
        let mut data = vec![];
        f.read_to_end(&mut data).unwrap();
        if name == "EPUB/package.opf" {
            let s=String::from_utf8(data).unwrap().replace(" properties=\"cover-image\"","").replace("id=\"page3\" href=\"page-0003.xhtml\" media-type=\"application/xhtml+xml\"/>","id=\"page3\" href=\"page-0003.xhtml\" media-type=\"application/xhtml+xml\" properties=\"cover-image\"/>");
            data = s.into_bytes();
        }
        output
            .start_file(
                name,
                zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        output.write_all(&data).unwrap();
    }
    output.finish().unwrap().into_inner()
}
pub fn epub() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let mut pages = crate::book_verify::pages();
    epub_metadata::blank_after(&mut pages, 1, B)?;
    let metadata = Metadata {
        title: "한글 <책> & 제목".into(),
        author: "저자, \"윤\"".into(),
        language: "ko-KR".into(),
        identifier: String::new(),
        cover: Some(2),
        right_binding: true,
    };
    let bytes = book_epub::encode_metadata(&pages, &metadata, &cancel)?;
    let decoded = epub_metadata::decode(&bytes, "", B, &cancel)?;
    check(
        decoded.pages == pages
            && decoded.metadata.title == metadata.title
            && decoded.metadata.author == metadata.author
            && decoded.metadata.language == metadata.language
            && decoded.metadata.cover == Some(2)
            && decoded.metadata.right_binding,
        "EPUB metadata/cover/blank/direction round trip",
    )?;
    check(
        decoded.pages[1].image.pixels.iter().all(|p| *p == [255; 4])
            && decoded.metadata.identifier.starts_with("urn:sha256:")
            && decoded.metadata.identifier.len() == 75,
        "EPUB blank or local identifier",
    )?;
    let again = book_epub::encode_metadata(&decoded.pages, &decoded.metadata, &cancel)?;
    check(
        again == bytes,
        "EPUB identifier changes after unchanged reimport",
    )?;
    let partial = epub_metadata::decode(&bytes, "3-4", B, &cancel)?;
    check(
        partial.metadata.cover == Some(0) && partial.pages == pages[2..],
        "Cover remapping for selected pages",
    )?;
    check(
        epub_metadata::decode(&bytes, "1-2", B, &cancel)?
            .metadata
            .cover
            .is_none(),
        "Excluded cover remains selected",
    )?;
    check(
        epub_metadata::decode(&invalid_cover(&bytes), "", B, &cancel).is_err(),
        "XHTML declared as a cover image accepted",
    )?;
    let mut bad = metadata.clone();
    bad.cover = Some(99);
    check(
        book_epub::encode_metadata(&pages, &bad, &cancel).is_err(),
        "Missing EPUB cover accepted",
    )?;
    bad = metadata.clone();
    bad.language = "ko/remote".into();
    check(
        book_epub::encode_metadata(&pages, &bad, &cancel).is_err(),
        "Invalid language accepted",
    )?;
    check(
        epub_metadata::blank_after(&mut pages, 99, B).is_err(),
        "Invalid blank insertion accepted",
    )?;
    check(
        epub_metadata::blank_after(&mut pages, 0, 1).is_err(),
        "Blank page ignores memory budget",
    )?;
    cancel.store(true, Ordering::Relaxed);
    check(
        book_epub::encode_metadata(&pages, &metadata, &cancel).is_err(),
        "Cancelled EPUB publication accepted",
    )?;
    Ok(())
}
pub fn fixture(root: &Path) -> Result<(std::path::PathBuf, shots::File), String> {
    use image::ImageEncoder;
    for (name, pixel) in [
        ("셀,하나.png", [180u8, 20, 70, 255]),
        ("셀둘.png", [10u8, 100, 210, 255]),
    ] {
        let mut bytes = vec![];
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixel.repeat(12), 3, 4, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        std::fs::write(root.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let fps = crate::animation::Fps {
        numerator: 30000,
        denominator: 1001,
    };
    let first = shots::Image {
        frame: 0,
        name: "셀,하나.png".into(),
        path: "셀,하나.png".into(),
        offset: [1, 2],
    };
    let file = shots::File {
        format: "Drawer shots".into(),
        version: 1,
        width: 12,
        height: 14,
        dpi: 300.,
        shots: vec![
            shots::Shot {
                id: 11,
                name: "같은 이름".into(),
                start: 0,
                end: 4,
                fps,
                dialogue: "한글, \"대사\"\n둘째 줄".into(),
                cells: vec![first.clone(), shots::Image { frame: 2, ..first }],
            },
            shots::Shot {
                id: 22,
                name: "같은 이름".into(),
                start: 4,
                end: 8,
                fps,
                dialogue: String::new(),
                cells: vec![shots::Image {
                    frame: 4,
                    name: "셀둘.png".into(),
                    path: "셀둘.png".into(),
                    offset: [0, 0],
                }],
            },
        ],
    };
    let path = root.join("입력.drawer-shots");
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).map_err(|e| e.to_string())?;
    Ok((path, file))
}
pub fn exchange(root: &Path) -> Result<(), String> {
    let dir = root.join("publication-shot-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let (path, file) = fixture(&dir)?;
        let original = std::fs::read(&path).unwrap();
        let cancel = AtomicBool::new(false);
        let csv = shots::csv(&file)?;
        check(
            shots::from_csv(std::str::from_utf8(&csv).unwrap())? == file,
            "Own CSV Unicode/quotes/newlines/empty dialogue round trip",
        )?;
        let p = shots::prepare(&path, B, &cancel)?;
        p.fresh()?;
        let mut d = p.document;
        let imported = d.snapshot();
        let a = d.animation.as_ref().unwrap();
        check(
            a.fps == file.shots[0].fps
                && a.tracks.len() == 2
                && a.tracks[0].cells.len() == 1
                && a.tracks[0].exposures.len() == 2,
            "Shot IDs/rational FPS/shared cell reference lost",
        )?;
        check(
            crate::render::Renderer::new(&d).pixel([1.5, 2.5]) == [180, 20, 70, 255],
            "Imported shot cell not rendered",
        )?;
        check(
            d.undo()
                && d.animation.is_none()
                && d.scene.shots.is_default()
                && d.redo()
                && d.snapshot() == imported,
            "Shot batch one drawing Undo/Redo",
        )?;
        let native = dir.join("native.drawer");
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, B)
                .map_err(|e| e.to_string())?
                .snapshot()
                == imported,
            "Native95 shot/cell round trip",
        )?;
        let mut bytes = std::fs::read(&native).unwrap();
        bytes[8..12].copy_from_slice(&94u32.to_le_bytes());
        let old = dir.join("old.drawer");
        std::fs::write(&old, bytes).unwrap();
        check(
            crate::files::open(&old, B).is_err(),
            "Native94 accepted new shot metadata",
        )?;
        let refreshed = shots::from_tracks(&d)?;
        check(
            refreshed.scene.shots.entries[0].shot.id == 11
                && refreshed.scene.shots.entries[0].shot.dialogue == file.shots[0].dialogue,
            "Refreshing metadata erased shot ID/dialogue",
        )?;
        let rows = vec![
            (11, "고친 이름".into(), "새 대사\n,\"따옴표\"".into()),
            (22, "같은 이름".into(), String::new()),
        ];
        d.commit_state(shots::properties(&d, &rows)?)?;
        let edited = d.snapshot();
        let writer = &mut |a: &Path, b: &Path| std::fs::hard_link(a, b).map_err(|e| e.to_string());
        for ext in ["drawer-shots", "csv"] {
            let out = dir.join(format!("출력.{ext}"));
            shots::export(&d, &out, B, &cancel, writer)?;
            let read = shots::prepare(&out, B, &cancel)?;
            check(
                read.document.animation == d.animation
                    && read
                        .document
                        .scene
                        .shots
                        .entries
                        .iter()
                        .map(|b| b.shot.id)
                        .collect::<Vec<_>>()
                        == vec![11, 22]
                    && read.document.scene.shots.entries[0].shot.dialogue == rows[0].2,
                "Exported shot file lost timeline/IDs/dialogue",
            )?;
        }
        check(
            d.snapshot() == edited,
            "Shot export changed source document",
        )?;
        let write =
            |f: &shots::File| std::fs::write(&path, serde_json::to_vec(f).unwrap()).unwrap();
        let mut independent = file.clone();
        independent.shots[0].cells[1].name = "independent alias".into();
        std::fs::write(&path, serde_json::to_vec(&independent).unwrap()).unwrap();
        let alias = shots::prepare(&path, B, &cancel)?;
        check(
            alias.document.animation.as_ref().unwrap().tracks[0]
                .cells
                .len()
                == 2,
            "Different cell names silently became a shared mutable cell",
        )?;
        let cell_path = dir.join("셀둘.png");
        let hidden = dir.join("temporarily-hidden.png");
        std::fs::rename(&cell_path, &hidden).unwrap();
        let stored = crate::files::open(&native, B).map_err(|e| e.to_string());
        std::fs::rename(&hidden, &cell_path).unwrap();
        check(
            stored?.animation == imported.animation,
            "Native shot depends on original external cell files",
        )?;
        let mut bad = file.clone();
        bad.shots[1].id = 11;
        write(&bad);
        check(
            shots::prepare(&path, B, &cancel).is_err(),
            "Duplicate shot ID accepted",
        )?;
        bad = file.clone();
        bad.shots[0].start = 5;
        write(&bad);
        check(
            shots::prepare(&path, B, &cancel).is_err(),
            "Reversed shot range accepted",
        )?;
        bad = file.clone();
        bad.shots[1].cells[0].path = "missing.png".into();
        write(&bad);
        check(
            shots::prepare(&path, B, &cancel).is_err(),
            "Missing local cell accepted",
        )?;
        bad = file.clone();
        bad.shots[1].cells[0].path = "https://invalid.example/cell.png".into();
        write(&bad);
        check(
            shots::prepare(&path, B, &cancel).is_err(),
            "Remote shot cell accepted",
        )?;
        bad = file.clone();
        bad.shots[1].fps.numerator = 24;
        write(&bad);
        check(
            shots::prepare(&path, B, &cancel).is_err(),
            "Mixed document FPS silently resampled",
        )?;
        let text = String::from_utf8(csv).unwrap();
        check(
            shots::from_csv(&text.replace(",0,4,", ",-1,4,")).is_err(),
            "Negative CSV frame accepted",
        )?;
        check(
            shots::from_csv("foreign,format\n1,2\n").is_err(),
            "Unknown external CSV accepted",
        )?;
        check(
            shots::prepare(&dir.join("external.xdts"), B, &cancel).is_err(),
            "Unknown proprietary animation file accepted",
        )?;
        std::fs::write(&path, &original).unwrap();
        let prepared = shots::prepare(&path, B, &cancel)?;
        std::fs::write(&path, b"changed").unwrap();
        check(
            prepared.fresh().is_err(),
            "Changed sheet accepted after preview",
        )?;
        std::fs::write(&path, &original).unwrap();
        let names = || {
            std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        check(
            shots::export(&d, &dir.join("failed.csv"), B, &cancel, &mut |_, _| {
                Err("MOCK publication failure".into())
            })
            .is_err()
                && names() == before,
            "Failed shot output leaked resources",
        )?;
        cancel.store(true, Ordering::Relaxed);
        check(
            shots::prepare(&path, B, &cancel).is_err()
                && shots::export(&d, &dir.join("cancel.csv"), B, &cancel, writer).is_err()
                && names() == before,
            "Cancelled shot operation leaked results",
        )?;
        check(
            std::fs::read(&path).unwrap() == original,
            "Shot source overwritten",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn epub_bibliography_cover_blank_direction_and_identifier() {
        super::epub().unwrap();
    }
    #[test]
    fn local_shot_json_csv_native_cells_and_failure_boundaries() {
        let root = std::env::temp_dir().join(format!("drawer-publication-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::exchange(&root);
        std::fs::remove_dir(root).unwrap();
        r.unwrap();
    }
}
