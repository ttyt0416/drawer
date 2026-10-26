//! Deterministic raster book fixtures and failure boundaries, also run installed.
use crate::{
    book_epub,
    book_io::{self as io, Page, PrintSink},
    book_pdf,
    references::LocalImage,
};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn pages() -> Vec<Page> {
    (0..3)
        .map(|n| Page {
            image: LocalImage {
                width: 16 + n * 4,
                height: 24 + n * 4,
                pixels: vec![
                    [40 + n as u8 * 50, 80, 160, if n == 2 { 128 } else { 255 }];
                    ((16 + n * 4) * (24 + n * 4)) as usize
                ],
            },
            dpi: 96. + n as f64 * 24.,
        })
        .collect()
}
fn mutate(bytes: &[u8], target: &str, value: &[u8]) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut dst = zip::ZipWriter::new(Cursor::new(vec![]));
    let mut found = false;
    for i in 0..src.len() {
        let mut file = src.by_index(i).unwrap();
        let name = file.name().to_string();
        let mut data = vec![];
        file.read_to_end(&mut data).unwrap();
        if name == target {
            data = value.to_vec();
            found = true;
        }
        dst.start_file(
            &name,
            zip::write::FileOptions::default().compression_method(if i == 0 {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            }),
        )
        .unwrap();
        dst.write_all(&data).unwrap();
    }
    if !found {
        dst.start_file(target, zip::write::FileOptions::default())
            .unwrap();
        dst.write_all(value).unwrap();
    }
    dst.finish().unwrap().into_inner()
}
fn member(bytes: &[u8], target: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut b = vec![];
    z.by_name(target).unwrap().read_to_end(&mut b).unwrap();
    b
}
pub fn ranges_pdf() -> Result<(), String> {
    check(
        io::selection("3,1-2,2", 3)? == vec![0, 1, 2] && io::selection("", 512)?.len() == 512,
        "Page order or duplicate range",
    )?;
    for s in ["0", "1-4", "3-1", "1,", "-1", "1-2-3", "x"] {
        check(io::selection(s, 3).is_err(), "Invalid page range accepted")?;
    }
    let pages = pages();
    let mut source = pages[0].document(io::MAX_BYTES)?;
    let state = source.snapshot();
    let history = source.history_counts();
    source.view.flipped = true;
    source.view.rotate(37.);
    let composed = io::compose(&[&source], 192., io::MAX_BYTES, &AtomicBool::new(false))?;
    check(
        composed[0].image.width == 32
            && composed[0].image.height == 48
            && source.snapshot() == state
            && source.history_counts() == history
            && source.view.flipped,
        "Composite DPI or source/view/Undo separation",
    )?;
    check(
        io::compose(&[&source], 192., 16, &AtomicBool::new(false)).is_err(),
        "Composite allocation limit ignored",
    )?;
    let bytes = book_pdf::encode(&pages, &AtomicBool::new(false))?;
    check(book_pdf::validate(&bytes)? == 3, "PDF page count")?;
    let d = lopdf::Document::load_mem(&bytes).map_err(|e| e.to_string())?;
    for (index, id) in d.get_pages().values().enumerate() {
        let p = d.get_object(*id).unwrap().as_dict().unwrap();
        let bounds = p.get(b"MediaBox").unwrap().as_array().unwrap();
        check(
            (bounds[2].as_float().unwrap() as f64
                - pages[index].image.width as f64 * 72. / pages[index].dpi)
                .abs()
                < 0.001,
            "PDF physical page size",
        )?;
    }
    for (key, value) in [
        (
            "OpenAction",
            lopdf::Object::Dictionary(
                lopdf::dictionary! {"S"=>"JavaScript","JS"=>lopdf::Object::string_literal("throw 1")},
            ),
        ),
        ("AA", lopdf::Object::Dictionary(lopdf::dictionary! {})),
        (
            "URI",
            lopdf::Object::string_literal("https://example.invalid"),
        ),
    ] {
        let mut d = d.clone();
        let root = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
        d.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(key, value);
        let mut b = vec![];
        d.save_to(&mut b).unwrap();
        check(
            book_pdf::validate(&b).is_err(),
            "Active/remote PDF accepted",
        )?;
    }
    for password in ["", "secret"] {
        let mut encrypted = d.clone();
        encrypted.trailer.set(
            "ID",
            vec![
                lopdf::Object::string_literal("drawer-fixture-id"),
                lopdf::Object::string_literal("drawer-fixture-id"),
            ],
        );
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V1 {
            document: &encrypted,
            owner_password: "owner",
            user_password: password,
            permissions: lopdf::Permissions::PRINTABLE,
        })
        .map_err(|e| e.to_string())?;
        encrypted.encrypt(&state).map_err(|e| e.to_string())?;
        let mut b = vec![];
        encrypted.save_to(&mut b).unwrap();
        check(
            book_pdf::validate(&b).is_err(),
            "Empty-password encrypted PDF accepted",
        )?;
    }
    check(
        book_pdf::encode(&pages, &AtomicBool::new(true)).is_err(),
        "Cancelled PDF output succeeded",
    )?;
    let mut bad = pages.clone();
    bad[1].dpi = f64::NAN;
    check(
        book_pdf::encode(&bad, &AtomicBool::new(false)).is_err(),
        "Partial invalid page accepted",
    )
}
pub fn epub() -> Result<(), String> {
    let pages = pages();
    let cancel = AtomicBool::new(false);
    let bytes = book_epub::encode(&pages, "로컬 & <image book>", &cancel)?;
    check(
        book_epub::decode(&bytes, "", io::MAX_BYTES, &cancel)? == pages,
        "EPUB exact pixels, alpha, dimensions, order or DPI roundtrip",
    )?;
    check(
        book_epub::decode(&bytes, "2-3", io::MAX_BYTES, &cancel)? == pages[1..],
        "Selected EPUB page range",
    )?;
    let opf = String::from_utf8(member(&bytes, "EPUB/package.opf")).unwrap();
    let html = String::from_utf8(member(&bytes, "EPUB/page-0002.xhtml")).unwrap();
    for (target, value) in [
        (
            "EPUB/package.opf",
            opf.replace("pre-paginated", "reflowable").into_bytes(),
        ),
        (
            "EPUB/page-0002.xhtml",
            html.replace("</head>", "<script>throw 1</script></head>")
                .into_bytes(),
        ),
        (
            "EPUB/page-0002.xhtml",
            html.replace("images/page-0002.png", "https://example.invalid/image.png")
                .into_bytes(),
        ),
        (
            "EPUB/page-0002.xhtml",
            html.replace("images/page-0002.png", "%한").into_bytes(),
        ),
        (
            "EPUB/page-0002.xhtml",
            html.replace("width=20", "width=21").into_bytes(),
        ),
        ("EPUB/images/page-0002.png", b"broken PNG".to_vec()),
        ("META-INF/encryption.xml", b"<encryption/>".to_vec()),
        ("../escape", b"no extraction".to_vec()),
        (
            "EPUB/page-0002.xhtml",
            b"<!DOCTYPE html [<!ENTITY attack SYSTEM 'file:///private'>]><html>&attack;</html>"
                .to_vec(),
        ),
    ] {
        let bad = mutate(&bytes, target, &value);
        check(
            book_epub::decode(&bad, "", io::MAX_BYTES, &cancel).is_err(),
            "Unsafe, unsupported or corrupt EPUB accepted",
        )?;
    }
    for bad in [
        mutate(&bytes, "EPUB/unlisted.xhtml", b"<script/>"),
        mutate(
            &bytes,
            "EPUB/package.opf",
            opf.replace(
                "</metadata>",
                "<meta property=\"rendition:layout\">reflowable</meta></metadata>",
            )
            .as_bytes(),
        ),
    ] {
        check(
            book_epub::decode(&bad, "", io::MAX_BYTES, &cancel).is_err(),
            "Unlisted resources or contradictory EPUB layout accepted",
        )?;
    }
    check(
        book_epub::decode(&bytes, "", 10, &cancel).is_err(),
        "EPUB budget ignored",
    )?;
    cancel.store(true, Ordering::Relaxed);
    check(
        book_epub::decode(&bytes, "", io::MAX_BYTES, &cancel).is_err()
            && book_epub::encode(&pages, "book", &cancel).is_err(),
        "EPUB cancellation ignored",
    )
}
#[derive(Default)]
struct Printer {
    pages: Vec<usize>,
    fail: Option<usize>,
    abort: bool,
    finish: bool,
    count: usize,
}
impl PrintSink for Printer {
    fn begin(&mut self, n: usize) -> Result<(), String> {
        self.count = n;
        Ok(())
    }
    fn page(&mut self, n: usize, _: &Page) -> Result<(), String> {
        if self.fail == Some(n) {
            return Err("paper failure".into());
        }
        self.pages.push(n);
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.finish = true;
        Ok(())
    }
    fn abort(&mut self) {
        self.abort = true;
    }
}
pub fn printer() -> Result<(), String> {
    let p = pages();
    let mut sink = Printer::default();
    io::print(&p[1..], &mut sink, &AtomicBool::new(false))?;
    check(
        sink.count == 2 && sink.pages == vec![0, 1] && sink.finish && !sink.abort,
        "Selected printer page contract",
    )?;
    let mut fail = Printer {
        fail: Some(1),
        ..Default::default()
    };
    check(
        io::print(&p, &mut fail, &AtomicBool::new(false)).is_err()
            && fail.abort
            && !fail.finish
            && fail.pages == vec![0],
        "Partial printer failure did not abort",
    )?;
    let mut cancelled = Printer::default();
    check(
        io::print(&p, &mut cancelled, &AtomicBool::new(true)).is_err() && cancelled.count == 0,
        "Cancelled job started printer",
    )
}
pub fn files(root: &Path) -> Result<(), String> {
    let dir = root.join("raster-book-files");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = dir.join("로컬-pages.pdf");
        let bytes = book_pdf::encode(&pages(), &AtomicBool::new(false))?;
        io::publish(&path, &bytes, &AtomicBool::new(false), &mut |a, b| {
            std::fs::hard_link(a, b).map_err(|e| e.to_string())
        })?;
        check(
            io::read_local(&path, io::MAX_BYTES)? == bytes,
            "Output publication bytes changed",
        )?;
        check(
            io::publish(&path, b"overwrite", &AtomicBool::new(false), &mut |_, _| {
                Ok(())
            })
            .is_err()
                && std::fs::read(&path).unwrap() == bytes,
            "Output overwrote existing file",
        )?;
        let race = dir.join("race.pdf");
        check(
            io::publish(&race, &bytes, &AtomicBool::new(false), &mut |a, b| {
                std::fs::write(b, b"foreign").map_err(|e| e.to_string())?;
                std::fs::hard_link(a, b).map_err(|e| e.to_string())
            })
            .is_err()
                && std::fs::read(&race).unwrap() == b"foreign",
            "Racing output overwrote a foreign file",
        )?;
        check(
            io::publish(
                &dir.join("cancel.pdf"),
                &bytes,
                &AtomicBool::new(true),
                &mut |_, _| Ok(()),
            )
            .is_err(),
            "Cancelled file output succeeded",
        )?;
        check(
            std::fs::read_dir(&dir).unwrap().count() == 2,
            "Failed/cancelled output left staging files",
        )
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
pub fn verify(root: &Path) -> Result<(), String> {
    ranges_pdf()?;
    epub()?;
    printer()?;
    files(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn selected_ranges_pdf_composites_and_active_encrypted_rejection() {
        super::ranges_pdf().unwrap();
    }
    #[test]
    fn local_fixed_image_epub_exact_roundtrip_and_unsafe_partial_failure() {
        super::epub().unwrap();
    }
    #[test]
    fn selected_print_pages_order_failure_and_cancel_abort() {
        super::printer().unwrap();
    }
    #[test]
    fn local_book_publication_collision_race_cancel_and_cleanup() {
        let root = crate::test_support::TempDir::new();
        super::files(&root.0).unwrap();
    }
}
