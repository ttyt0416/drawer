//! Local raster page interchange. No source document or archive member is modified.
use crate::{document::Document, references::LocalImage};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
pub const MAX_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PAGES: usize = 512;
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub image: LocalImage,
    pub dpi: f64,
}
impl Page {
    pub fn valid(&self) -> bool {
        self.image.valid() && self.dpi.is_finite() && (1. ..=4800.).contains(&self.dpi)
    }
    pub fn document(&self, budget: usize) -> Result<Document, String> {
        if !self.valid() {
            return Err("Invalid raster page".into());
        }
        let mut d = self.image.document(budget).map_err(|e| e.to_string())?;
        d.dpi = self.dpi;
        Ok(d)
    }
}
pub fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Page operation cancelled; source documents unchanged".into())
    } else {
        Ok(())
    }
}
pub fn validate(pages: &[Page], budget: usize) -> Result<(), String> {
    if pages.is_empty()
        || pages.len() > MAX_PAGES
        || pages.iter().any(|p| !p.valid())
        || pages
            .iter()
            .try_fold(0usize, |n, p| n.checked_add(p.image.memory_bytes()))
            .is_none_or(|n| n > budget.min(MAX_BYTES))
    {
        return Err("Raster pages exceed size, DPI, count or memory limits".into());
    }
    Ok(())
}
/// One-based ascending selection, with duplicates removed. Empty input means all.
pub fn selection(s: &str, count: usize) -> Result<Vec<usize>, String> {
    if count == 0 || count > MAX_PAGES || s.len() > 4096 {
        return Err("Page count or range limit".into());
    }
    if s.trim().is_empty() {
        return Ok((0..count).collect());
    }
    let mut set = std::collections::BTreeSet::new();
    for part in s.split(',') {
        let v = part
            .trim()
            .split('-')
            .map(|n| {
                n.trim()
                    .parse::<usize>()
                    .map_err(|_| "Use page numbers such as 1,3-5")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (a, b) = match v.as_slice() {
            [n] => (*n, *n),
            [a, b] => (*a, *b),
            _ => return Err("Use page numbers such as 1,3-5".into()),
        };
        if a == 0 || a > b || b > count {
            return Err("Page range is outside the book".into());
        }
        set.extend(a - 1..b);
    }
    Ok(set.into_iter().collect())
}
pub fn compose(
    docs: &[&Document],
    dpi: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    if docs.is_empty()
        || docs.len() > MAX_PAGES
        || !dpi.is_finite()
        || !(1. ..=4800.).contains(&dpi)
    {
        return Err("Choose 1..512 pages and 1..4800 DPI".into());
    }
    let mut out = vec![];
    let mut used = 0usize;
    for d in docs {
        check_cancel(cancel)?;
        if !d.scene.valid()
            || !d.color.valid_layers(&d.layers)
            || d.layers.iter().any(|l| !l.valid(d.width, d.height))
        {
            return Err("Invalid source page; no output prepared".into());
        }
        let r = crate::render::Renderer::for_export(d)?;
        if let Some(e) = r.error() {
            return Err(e.into());
        }
        let (w, h, pixels) = r.output(dpi, budget.min(MAX_BYTES).saturating_sub(used))?;
        used = used.saturating_add(pixels.len() * 4);
        if used > budget.min(MAX_BYTES) {
            return Err("Page composition memory limit".into());
        }
        out.push(Page {
            image: LocalImage {
                width: w,
                height: h,
                pixels,
            },
            dpi,
        });
    }
    validate(&out, budget)?;
    check_cancel(cancel)?;
    Ok(out)
}
pub fn decode(bytes: &[u8], budget: usize) -> Result<LocalImage, String> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(
        r.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err("Image books support PNG/JPEG pages".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(budget as u64);
    r.limits(limits);
    use image::ImageDecoder;
    let mut decoder = r.into_decoder().map_err(|e| e.to_string())?;
    let gray = matches!(
        decoder.color_type(),
        image::ColorType::L8
            | image::ColorType::La8
            | image::ColorType::L16
            | image::ColorType::La16
    );
    let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
    let mut rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    if let Some(profile) = profile {
        crate::image_import::convert_profile(&mut rgba, &profile, gray, &AtomicBool::new(false))
            .map_err(|e| e.to_string())?;
    }

    LocalImage::from_rgba(
        rgba.width() as usize,
        rgba.height() as usize,
        rgba.as_raw(),
        budget,
    )
    .map_err(|e| e.to_string())
}
pub fn read_local(path: &Path, budget: usize) -> Result<Vec<u8>, String> {
    if path.is_absolute() && !crate::font_library::local(path) {
        return Err("Choose a local drive file".into());
    }
    let p = path.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&p) || !p.is_file() {
        return Err("Choose a regular local file".into());
    }
    let mut data = vec![];
    std::fs::File::open(p)
        .map_err(|e| e.to_string())?
        .take(budget.min(MAX_BYTES) as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > budget.min(MAX_BYTES) {
        return Err("Input file exceeds page memory limit".into());
    }
    Ok(data)
}
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn publish(
    path: &Path,
    bytes: &[u8],
    cancel: &AtomicBool,
    writer: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Output size limit".into());
    }
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !crate::font_library::local(&dir) {
        return Err("Choose a local output directory".into());
    }
    let dest = dir.join(path.file_name().ok_or("Choose an output filename")?);
    if std::fs::symlink_metadata(&dest).is_ok() {
        return Err("Output exists; choose another filename".into());
    }
    let temporary = dir.join(format!(
        ".drawer-book-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let temp = Temp(temporary);
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    check_cancel(cancel)?;
    writer(&temp.0, &dest)
}
/// Printer adapters implement this contract; a failed page always aborts the job.
pub trait PrintSink {
    fn begin(&mut self, pages: usize) -> Result<(), String>;
    fn page(&mut self, index: usize, page: &Page) -> Result<(), String>;
    fn finish(&mut self) -> Result<(), String>;
    fn abort(&mut self);
}
pub fn print(pages: &[Page], sink: &mut impl PrintSink, cancel: &AtomicBool) -> Result<(), String> {
    validate(pages, MAX_BYTES)?;
    check_cancel(cancel)?;
    sink.begin(pages.len())?;
    let result = (|| {
        for (i, p) in pages.iter().enumerate() {
            check_cancel(cancel)?;
            sink.page(i, p)?;
        }
        check_cancel(cancel)?;
        sink.finish()
    })();
    if result.is_err() {
        sink.abort();
    }
    result
}
