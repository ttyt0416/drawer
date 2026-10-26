//! User-invoked print dialog and GDI spool adapter. Tests never submit paper jobs.
use drawer_core::book_io::{self, Page, PrintSink};
use std::sync::atomic::AtomicBool;
use windows::{
    Win32::{Foundation::GlobalFree, Graphics::Gdi::*, Storage::Xps::*, UI::Controls::Dialogs::*},
    core::w,
};
struct Dialog(PRINTDLGW);
impl Drop for Dialog {
    fn drop(&mut self) {
        // SAFETY: only handles returned by PrintDlgW are owned here, released once
        // after the borrowed printer sink is dropped, including cancellation/errors.
        unsafe {
            if !self.0.hDC.is_invalid() {
                let _ = DeleteDC(self.0.hDC);
            }
            if !self.0.hDevMode.is_invalid() {
                let _ = GlobalFree(Some(self.0.hDevMode));
            }
            if !self.0.hDevNames.is_invalid() {
                let _ = GlobalFree(Some(self.0.hDevNames));
            }
        }
    }
}
struct Printer {
    dc: HDC,
    started: bool,
}
impl PrintSink for Printer {
    fn begin(&mut self, _: usize) -> Result<(), String> {
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: w!("Drawer selected pages"),
            ..Default::default()
        };
        // SAFETY: dc is live for the entire dialog owner scope; UTF-16 literal and
        // correctly-sized DOCINFO are valid for this synchronous GDI call.
        if unsafe { StartDocW(self.dc, &info) } <= 0 {
            return Err("Printer could not start the job".into());
        }
        self.started = true;
        Ok(())
    }
    fn page(&mut self, _: usize, page: &Page) -> Result<(), String> {
        let image = &page.image;
        let mut bgra = Vec::with_capacity(image.memory_bytes());
        for p in &image.pixels {
            let c = |n: usize| {
                ((p[n] as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
            };
            bgra.extend([c(2), c(1), c(0), 0]);
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image.width as i32,
                biHeight: -(image.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: live printer DC, validated dimensions and matching top-down BGRA
        // allocation. GDI reads bytes synchronously; it never retains pointers.
        let ok = unsafe {
            let pw = GetDeviceCaps(Some(self.dc), HORZRES);
            let ph = GetDeviceCaps(Some(self.dc), VERTRES);
            if pw <= 0 || ph <= 0 || StartPage(self.dc) <= 0 {
                return Err("Printer could not begin page".into());
            }
            let ratio = (pw as f64 / image.width as f64).min(ph as f64 / image.height as f64);
            let w = (image.width as f64 * ratio).round() as i32;
            let h = (image.height as f64 * ratio).round() as i32;
            SetStretchBltMode(self.dc, HALFTONE);
            let copied = StretchDIBits(
                self.dc,
                (pw - w) / 2,
                (ph - h) / 2,
                w,
                h,
                0,
                0,
                image.width as i32,
                image.height as i32,
                Some(bgra.as_ptr().cast()),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            copied > 0 && EndPage(self.dc) > 0
        };
        if ok {
            Ok(())
        } else {
            Err("Printer failed to draw/end the page; job aborted".into())
        }
    }
    fn finish(&mut self) -> Result<(), String> {
        // SAFETY: a successful StartDoc owns this live printer job.
        if unsafe { EndDoc(self.dc) } <= 0 {
            return Err("Printer could not finish job".into());
        }
        self.started = false;
        Ok(())
    }
    fn abort(&mut self) {
        if self.started {
            // SAFETY: abort only the job started on this same owned DC.
            unsafe { AbortDoc(self.dc) };
            self.started = false;
        }
    }
}
impl Drop for Printer {
    fn drop(&mut self) {
        self.abort();
    }
}
pub fn show(pages: &[Page], cancel: &AtomicBool) -> Result<bool, String> {
    book_io::validate(pages, book_io::MAX_BYTES)?;
    book_io::check_cancel(cancel)?;
    let mut dialog = Dialog(PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        Flags: PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE,
        nMinPage: 1,
        nMaxPage: pages.len() as u16,
        nFromPage: 1,
        nToPage: pages.len() as u16,
        nCopies: 1,
        ..Default::default()
    });
    // SAFETY: initialized PRINTDLGW stays valid until the synchronous dialog exits.
    // No hooks or pointers to callbacks are supplied; the OS owns all UI dispatch.
    if !unsafe { PrintDlgW(&mut dialog.0) }.as_bool() {
        // SAFETY: query the same thread's last common-dialog result.
        let error = unsafe { CommDlgExtendedError() }.0;
        return if error == 0 {
            Ok(false)
        } else {
            Err(format!("Print dialog failed: {error}"))
        };
    }
    let mut sink = Printer {
        dc: dialog.0.hDC,
        started: false,
    };
    book_io::print(pages, &mut sink, cancel)?;
    Ok(true)
}
