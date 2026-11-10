//! User-invoked native printing. Automated verification never submits physical jobs.
use drawer_core::{
    book_io::{Page, PrintSink},
    printing::{self, Placement, Settings},
};
use std::sync::atomic::AtomicBool;
use windows::{
    Win32::{Foundation::GlobalFree, Graphics::Gdi::*, Storage::Xps::*, UI::Controls::Dialogs::*},
    core::w,
};
#[link(name = "gdi32")]
unsafe extern "system" {
    fn SetICMMode(dc: HDC, mode: i32) -> i32;
}
struct Dialog(PRINTDLGW);
impl Drop for Dialog {
    fn drop(&mut self) {
        // SAFETY: these are exactly the owned handles returned by PrintDlgW; the sink is already dropped.
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
    pages: Vec<Placement>,
    pixels_mm: [f64; 2],
    offset: [i32; 2],
}
fn coordinate(v: f64) -> Result<i32, String> {
    if !v.is_finite() || !(i32::MIN as f64..=i32::MAX as f64).contains(&v) {
        return Err("Print placement exceeds driver coordinate limits".into());
    }
    Ok(v.round() as i32)
}
impl Printer {
    fn rect(&self, a: printing::Area) -> Result<[i32; 4], String> {
        Ok([
            coordinate(a.x * self.pixels_mm[0] - self.offset[0] as f64)?,
            coordinate(a.y * self.pixels_mm[1] - self.offset[1] as f64)?,
            coordinate((a.x + a.width) * self.pixels_mm[0] - self.offset[0] as f64)?,
            coordinate((a.y + a.height) * self.pixels_mm[1] - self.offset[1] as f64)?,
        ])
    }
}
impl PrintSink for Printer {
    fn begin(&mut self, _: usize) -> Result<(), String> {
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: w!("Drawer reviewed print pages"),
            ..Default::default()
        };
        // SAFETY: the dialog owns a live DC and the static UTF-16 DOCINFO string outlives this call.
        if unsafe { StartDocW(self.dc, &info) } <= 0 {
            return Err("Printer spooler could not start the job".into());
        }
        self.started = true;
        Ok(())
    }
    fn page(&mut self, index: usize, page: &Page) -> Result<(), String> {
        let p = self.pages.get(index).ok_or("Print layout changed")?;
        let dest = self.rect(p.image)?;
        let clip = self.rect(p.clip)?;
        let (width, height) = (
            dest[2].checked_sub(dest[0]).ok_or("Print width overflow")?,
            dest[3]
                .checked_sub(dest[1])
                .ok_or("Print height overflow")?,
        );
        if width <= 0 || height <= 0 {
            return Err(
                "Print scale is below this printer's pixel resolution; increase scale".into(),
            );
        }
        let image = &page.image;
        let mut bgra = Vec::with_capacity(image.memory_bytes());
        for p in &image.pixels {
            let c = |n: usize| {
                ((p[n] as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
            };
            bgra.extend([c(2), c(1), c(0), 0]);
        }
        let info = BITMAPV4HEADER {
            bV4Size: std::mem::size_of::<BITMAPV4HEADER>() as u32,
            bV4Width: image.width as i32,
            bV4Height: -(image.height as i32),
            bV4Planes: 1,
            bV4BitCount: 32,
            bV4V4Compression: BI_RGB,
            bV4CSType: 0x73524742,
            ..Default::default()
        };
        // SAFETY: validated top-down BGRA matches the V4 header. The header declares sRGB,
        // has no palette, and is passed through GDI's documented BITMAPINFO/V4-header ABI.
        // Coordinates were checked; the saved DC restores the margin clip before ending the page.
        let ok = unsafe {
            if StartPage(self.dc) <= 0 {
                return Err("Printer could not begin page".into());
            }
            let saved = SaveDC(self.dc);
            if saved == 0 {
                return Err("Printer could not save page clip".into());
            }
            let region = IntersectClipRect(self.dc, clip[0], clip[1], clip[2], clip[3]);
            SetStretchBltMode(self.dc, HALFTONE);
            let copied = if region.0 == 0 {
                0
            } else {
                StretchDIBits(
                    self.dc,
                    dest[0],
                    dest[1],
                    width,
                    height,
                    0,
                    0,
                    image.width as i32,
                    image.height as i32,
                    Some(bgra.as_ptr().cast()),
                    (&info as *const BITMAPV4HEADER).cast::<BITMAPINFO>(),
                    DIB_RGB_COLORS,
                    SRCCOPY,
                )
            };
            let restored = RestoreDC(self.dc, saved).as_bool();
            copied > 0 && restored && EndPage(self.dc) > 0
        };
        if ok {
            Ok(())
        } else {
            Err("Printer failed while drawing/ending a page; spool job aborted".into())
        }
    }
    fn finish(&mut self) -> Result<(), String> {
        // SAFETY: only the live job begun on this owned DC is ended here.
        if unsafe { EndDoc(self.dc) } <= 0 {
            return Err("Printer could not finish the spool job".into());
        }
        self.started = false;
        Ok(())
    }
    fn abort(&mut self) {
        if self.started {
            // SAFETY: abort only this DC's job; no other application or queue is affected.
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
pub fn dialog_error(code: u32) -> String {
    if code == 0x1008 {
        "No default local printer is installed. Install/select a local queue or export a virtual preview PDF.".into()
    } else {
        format!("Windows printer selection failed: {code}; no job submitted")
    }
}
pub fn show(pages: &[Page], cancel: &AtomicBool) -> Result<bool, String> {
    show_with_settings(pages, &Settings::default(), cancel)
}
pub fn show_with_settings(
    pages: &[Page],
    settings: &Settings,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    let plan = printing::plan(pages, settings)?;
    drawer_core::book_io::check_cancel(cancel)?;
    let mut dialog = Dialog(PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        Flags: PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE,
        nMinPage: 1,
        nMaxPage: plan.pages.len() as u16,
        nFromPage: 1,
        nToPage: plan.pages.len() as u16,
        nCopies: 1,
        ..Default::default()
    });
    // SAFETY: synchronous OS dialog, correctly sized structure, no hooks or borrowed callbacks.
    if !unsafe { PrintDlgW(&mut dialog.0) }.as_bool() {
        // SAFETY: this queries only the current thread's common-dialog result.
        let error = unsafe { CommDlgExtendedError() }.0;
        return if error == 0 {
            Ok(false)
        } else {
            Err(dialog_error(error))
        };
    }
    drawer_core::book_io::check_cancel(cancel)?;
    // SAFETY: GetDeviceCaps reads the live printer DC owned by dialog; it starts no spool job.
    let (dpi, physical, offset, printable) = unsafe {
        (
            [
                GetDeviceCaps(Some(dialog.0.hDC), LOGPIXELSX),
                GetDeviceCaps(Some(dialog.0.hDC), LOGPIXELSY),
            ],
            [
                GetDeviceCaps(Some(dialog.0.hDC), PHYSICALWIDTH),
                GetDeviceCaps(Some(dialog.0.hDC), PHYSICALHEIGHT),
            ],
            [
                GetDeviceCaps(Some(dialog.0.hDC), PHYSICALOFFSETX),
                GetDeviceCaps(Some(dialog.0.hDC), PHYSICALOFFSETY),
            ],
            [
                GetDeviceCaps(Some(dialog.0.hDC), HORZRES),
                GetDeviceCaps(Some(dialog.0.hDC), VERTRES),
            ],
        )
    };
    if dpi
        .iter()
        .chain(physical.iter())
        .chain(printable.iter())
        .any(|n| *n <= 0)
        || offset.iter().any(|n| *n < 0)
    {
        return Err("Chosen printer has no valid paper/printable metrics".into());
    }
    let paper = [0, 1].map(|n| physical[n] as f64 / dpi[n] as f64 * 25.4);
    let margins = [
        offset[0] as f64 / dpi[0] as f64 * 25.4,
        offset[1] as f64 / dpi[1] as f64 * 25.4,
        (physical[0] - offset[0] - printable[0]) as f64 / dpi[0] as f64 * 25.4,
        (physical[1] - offset[1] - printable[1]) as f64 / dpi[1] as f64 * 25.4,
    ];
    printing::driver_layout(settings, paper, margins)?;
    // SAFETY: configure color matching on this owned DC before StartDoc. SDK ICM_ON/OFF are2/1.
    if unsafe { SetICMMode(dialog.0.hDC, if settings.icm { 2 } else { 1 }) } == 0 {
        return Err(
            "Printer cannot apply the selected Windows ICM mode; review driver color settings"
                .into(),
        );
    }
    let mut sink = Printer {
        dc: dialog.0.hDC,
        started: false,
        pages: plan.pages,
        pixels_mm: [dpi[0] as f64 / 25.4, dpi[1] as f64 / 25.4],
        offset,
    };
    printing::send(pages, settings, &mut sink, cancel)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    #[test]
    fn absent_printer_guidance_is_explicit_and_not_a_print_job() {
        assert!(super::dialog_error(0x1008).contains("No default"));
        assert!(super::dialog_error(123).contains("no job submitted"));
    }
}
