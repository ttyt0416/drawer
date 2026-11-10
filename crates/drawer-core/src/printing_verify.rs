use super::*;
use crate::book_io::PrintSink;
use std::sync::atomic::Ordering;
fn check(ok: bool, note: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(note.into()) }
}
pub fn fixture() -> Vec<Page> {
    vec![
        Page {
            image: LocalImage {
                width: 100,
                height: 100,
                pixels: vec![[255, 0, 0, 128]; 10000],
            },
            dpi: 100.,
        },
        Page {
            image: LocalImage {
                width: 200,
                height: 100,
                pixels: vec![[0, 0, 255, 255]; 20000],
            },
            dpi: 200.,
        },
    ]
}
struct Sink<'a> {
    begin: usize,
    pages: usize,
    finish: bool,
    abort: bool,
    fail_begin: bool,
    fail_page: bool,
    cancel: Option<&'a AtomicBool>,
}
impl PrintSink for Sink<'_> {
    fn begin(&mut self, n: usize) -> Result<(), String> {
        if self.fail_begin {
            return Err("No local printer".into());
        }
        self.begin = n;
        Ok(())
    }
    fn page(&mut self, _: usize, _: &Page) -> Result<(), String> {
        self.pages += 1;
        if let Some(c) = self.cancel {
            c.store(true, Ordering::Relaxed);
        }
        if self.fail_page {
            Err("Spool failure".into())
        } else {
            Ok(())
        }
    }
    fn finish(&mut self) -> Result<(), String> {
        self.finish = true;
        Ok(())
    }
    fn abort(&mut self) {
        self.abort = true;
    }
}
fn sink() -> Sink<'static> {
    Sink {
        begin: 0,
        pages: 0,
        finish: false,
        abort: false,
        fail_begin: false,
        fail_page: false,
        cancel: None,
    }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("printing-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let pages = fixture();
    let original = pages.clone();
    let c = AtomicBool::new(false);
    let mut s = Settings {
        fit: false,
        ..Default::default()
    };
    let p = plan(&pages, &s)?;
    check(
        p.paper_mm == [210., 297.]
            && (p.pages[0].image.width - 25.4).abs() < 1e-9
            && (p.pages[1].image.width - 25.4).abs() < 1e-9,
        "Print physical dimensions/DPI",
    )?;
    check(
        (p.pages[0].image.x - (210. - 25.4) / 2.).abs() < 1e-9 && !p.pages[0].clipped,
        "Print centering",
    )?;
    let raster = virtual_pages(&pages[..1], &s, 100., 32 * 1024 * 1024, &c)?;
    let r = &raster[0].image;
    check(
        (r.width, r.height) == (827, 1169)
            && r.pixels[(584 * r.width + 413) as usize] == [255, 127, 127, 255]
            && r.pixels[0] == [255; 4],
        "Virtual paper geometry/alpha on white",
    )?;
    let pdf = crate::book_pdf::encode(&raster, &c)?;
    check(
        crate::book_pdf::validate(&pdf)? == 1,
        "Virtual PDF proof pages",
    )?;
    s.landscape = true;
    s.fit = true;
    s.percent = 400.;
    s.range = "2,1,2".into();
    let p = plan(&pages, &s)?;
    check(
        p.paper_mm == [297., 210.]
            && p.pages.len() == 2
            && p.pages.iter().all(|p| !p.clipped)
            && s.percent == 400.,
        "Fit/landscape/range preserved scale",
    )?;
    s.paper = Paper::Custom;
    s.custom_mm = [20., 20.];
    s.margins = [1.; 4];
    s.fit = false;
    s.range.clear();
    check(
        plan(&pages, &s)?.pages.iter().any(|p| p.clipped),
        "Oversized print cropping not reported",
    )?;
    s = Settings::default();
    driver_layout(&s, [210.02, 297.01], [3., 4., 3., 4.])?;
    check(
        driver_layout(&s, [215.9, 279.4], [0.; 4]).is_err()
            && driver_layout(&s, [210., 297.], [11., 0., 0., 0.]).is_err()
            && driver_layout(&s, [0., 0.], [0.; 4]).is_err(),
        "Driver paper/margin/no-printer mismatch accepted",
    )?;
    for bad in [
        Settings {
            percent: 0.,
            ..s.clone()
        },
        Settings {
            margins: [150.; 4],
            ..s.clone()
        },
        Settings {
            custom_mm: [0., 297.],
            ..s.clone()
        },
        Settings {
            range: "0".into(),
            ..s.clone()
        },
    ] {
        check(plan(&pages, &bad).is_err(), "Invalid print input accepted")?;
    }
    let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
    s.save(&storage)?;
    check(Settings::load(&storage)? == s, "Print preferences restart")?;
    let file = storage.root().join("settings/printing.json");
    let bytes = fs::read(&file).map_err(|e| e.to_string())?;
    let temp = file.with_extension("json.print-write.tmp");
    fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    check(
        s.save(&storage).is_err()
            && fs::read(&file).map_err(|e| e.to_string())? == bytes
            && fs::read(&temp).map_err(|e| e.to_string())? == b"foreign",
        "Print preference failure overwrote previous/foreign file",
    )?;
    fs::remove_file(temp).map_err(|e| e.to_string())?;
    let mut ok = sink();
    send(&pages, &s, &mut ok, &c)?;
    check(
        ok.begin == 2 && ok.pages == 2 && ok.finish && !ok.abort,
        "Print job sequence",
    )?;
    let mut none = sink();
    none.fail_begin = true;
    check(
        send(&pages, &s, &mut none, &c).is_err() && none.pages == 0 && !none.finish,
        "No-printer path started a page",
    )?;
    let mut fail = sink();
    fail.fail_page = true;
    check(
        send(&pages, &s, &mut fail, &c).is_err() && fail.abort && !fail.finish,
        "Spool failure did not abort",
    )?;
    let mut stopped = sink();
    stopped.cancel = Some(&c);
    check(
        send(&pages, &s, &mut stopped, &c).is_err()
            && stopped.pages == 1
            && stopped.abort
            && !stopped.finish,
        "Print cancellation did not abort pending pages",
    )?;
    check(
        virtual_pages(&pages, &s, 96., 32 * 1024 * 1024, &c).is_err() && pages == original,
        "Canceled virtual proof changed sources",
    )?;
    crate::asset_catalog::cleanup_verification(root, &dir)
}
#[cfg(test)]
mod tests {
    #[test]
    fn physical_layout_virtual_pdf_driver_limits_preferences_spool_and_cancel() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
