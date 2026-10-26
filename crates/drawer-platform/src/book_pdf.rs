//! Windows PDF rasterizer. All work uses an immutable local byte snapshot on MTA.
use drawer_core::book_io::{self, Page};
use std::sync::atomic::AtomicBool;
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
struct Runtime;
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: balances the successful initialization on this worker thread.
        unsafe { RoUninitialize() };
    }
}
pub fn render(
    bytes: &[u8],
    range: &str,
    dpi: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| render_mta(bytes, range, dpi, budget, cancel))
            .join()
            .map_err(|_| "PDF worker stopped".to_string())?
    })
}
fn render_mta(
    bytes: &[u8],
    range: &str,
    dpi: f64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    let count = drawer_core::book_pdf::validate(bytes)?;
    let selected = book_io::selection(range, count)?;
    if !dpi.is_finite() || !(1. ..=4800.).contains(&dpi) {
        return Err("Choose 1..4800 DPI".into());
    }
    book_io::check_cancel(cancel)?;
    let win = || -> windows::core::Result<Vec<Page>> {
        // SAFETY: this function is invoked on a dedicated worker/test thread and
        // keeps all WinRT objects within the balanced MTA guard's lifetime.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        let _runtime = Runtime;
        let input = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&input.GetOutputStreamAt(0)?)?;
        writer.WriteBytes(bytes)?;
        writer.StoreAsync()?.join()?;
        writer.DetachStream()?;
        drop(writer);
        input.Seek(0)?;
        let pdf = PdfDocument::LoadFromStreamAsync(&input)?.join()?;
        if pdf.IsPasswordProtected()? || pdf.PageCount()? as usize != count {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x8007000du32 as i32),
                "Encrypted PDF or page count mismatch",
            ));
        }
        let mut out = vec![];
        let mut used = bytes.len();
        for index in &selected {
            book_io::check_cancel(cancel).map_err(os_error)?;
            let page = pdf.GetPage(*index as u32)?;
            let size = page.Size()?;
            let w = (size.Width as f64 * dpi / 96.).round().max(1.);
            let h = (size.Height as f64 * dpi / 96.).round().max(1.);
            let remaining = budget.min(book_io::MAX_BYTES).saturating_sub(used);
            if !w.is_finite()
                || !h.is_finite()
                || w > 32768.
                || h > 32768.
                || w * h * 12. > remaining as f64
            {
                return Err(os_error("PDF raster dimensions or memory limit"));
            }
            let opts = PdfPageRenderOptions::new()?;
            opts.SetDestinationWidth(w as u32)?;
            opts.SetDestinationHeight(h as u32)?;
            opts.SetBackgroundColor(windows::UI::Color {
                A: 255,
                R: 255,
                G: 255,
                B: 255,
            })?;
            let stream = InMemoryRandomAccessStream::new()?;
            page.RenderWithOptionsToStreamAsync(&stream, &opts)?
                .join()?;
            page.Close()?;
            let len = stream.Size()?;
            if len > remaining as u64 / 2 {
                return Err(os_error("PDF encoded raster memory limit"));
            }
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            if reader.LoadAsync(len as u32)?.join()? as u64 != len {
                return Err(os_error("Incomplete PDF raster stream"));
            }
            let mut png = vec![0; len as usize];
            reader.ReadBytes(&mut png)?;
            let image =
                book_io::decode(&png, remaining.saturating_sub(png.len())).map_err(os_error)?;
            if image.width != w as u32 || image.height != h as u32 {
                return Err(os_error("PDF raster size mismatch"));
            }
            used = used.saturating_add(image.memory_bytes());
            out.push(Page { image, dpi });
        }
        Ok(out)
    };
    let out = win().map_err(|e| format!("Windows PDF raster import failed: {e}"))?;
    book_io::check_cancel(cancel)?;
    book_io::validate(&out, budget.saturating_sub(bytes.len()))?;
    Ok(out)
}
fn os_error(s: impl AsRef<str>) -> windows::core::Error {
    windows::core::Error::new(windows::core::HRESULT(0x8007000du32 as i32), s.as_ref())
}

pub fn verify() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let pages = drawer_core::book_verify::pages();
    let bytes = drawer_core::book_pdf::encode(&pages, &cancel)?;
    let before = bytes.clone();
    let result = render(&bytes, "1-3", 96., book_io::MAX_BYTES, &cancel)?;
    if result.len() != 3
        || result[0].image.width != 16
        || result[0].image.height != 24
        || result[1].image.width != 16
        || result[1].image.height != 22
        || bytes != before
    {
        return Err("Native PDF selection/DPI/page dimensions/source snapshot".into());
    }
    for (i, p) in result.iter().enumerate() {
        let source = pages[i].image.pixels[0];
        let expected = source.map(|c| c as i32);
        let pixel = p.image.pixels[(p.image.width * p.image.height / 2) as usize];
        for n in 0..3 {
            let expected =
                (expected[n] * source[3] as i32 + 255 * (255 - source[3] as i32) + 127) / 255;
            if (pixel[n] as i32 - expected).abs() > 2 {
                return Err("Native PDF image orientation, color or alpha flattening".into());
            }
        }
    }
    let doubled = render(&bytes, "2", 192., book_io::MAX_BYTES, &cancel)?;
    if doubled.len() != 1
        || doubled[0].image.width != 32
        || doubled[0].image.height != 45
        || render(&bytes, "0", 96., book_io::MAX_BYTES, &cancel).is_ok()
        || render(&bytes, "1", 4800., 1024, &cancel).is_ok()
        || render(&bytes, "1", 96., book_io::MAX_BYTES, &AtomicBool::new(true)).is_ok()
    {
        return Err("Native PDF range, DPI, allocation or cancellation boundary".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn windows_pdf_raster_page_selection_dpi_alpha_and_cancellation() {
        super::verify().unwrap();
    }
}
