//! Only this app's current GL context and owned GDI surfaces are accessed.
use glow::HasContext;
use raw_window_handle::HasWindowHandle;
use std::ffi::c_void;
#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglGetCurrentContext() -> isize;
}
#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglMakeCurrent(dc: isize, context: isize) -> i32;
}
pub fn detach_current_for_verification() -> Result<(), String> {
    // SAFETY: only detaches this verification thread's current context; no other process/window is accessed.
    unsafe {
        if wglGetCurrentContext() == 0 || wglMakeCurrent(0, 0) == 0 {
            return Err("Cannot detach owned verification context".into());
        }
    }
    Ok(())
}
pub fn health(gl: &glow::Context) -> Result<(), String> {
    // SAFETY: current-thread context query and error query do not borrow user memory.
    unsafe {
        if wglGetCurrentContext() == 0 {
            return Err("OpenGL context is unavailable".into());
        }
        let code = gl.get_error();
        if code != glow::NO_ERROR {
            return Err(format!("OpenGL display error0x{code:04x}"));
        }
    }
    Ok(())
}
pub fn identity(gl: &glow::Context) -> Result<[String; 3], String> {
    health(gl)?;
    // SAFETY: health verified a current context on this thread; returned strings are owned.
    unsafe {
        Ok([
            gl.get_parameter_string(glow::VENDOR),
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION),
        ])
    }
}
#[repr(C)]
struct Header {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bits: u16,
    compression: u32,
    bytes: u32,
    x: i32,
    y: i32,
    colors: u32,
    important: u32,
}
#[repr(C)]
struct Info {
    header: Header,
    color: [u32; 1],
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetDC(window: *mut c_void) -> *mut c_void;
    fn ReleaseDC(window: *mut c_void, dc: *mut c_void) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn StretchDIBits(
        dc: *mut c_void,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        bits: *const c_void,
        info: *const Info,
        usage: u32,
        operation: u32,
    ) -> i32;
    fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
    fn CreateDIBSection(
        dc: *mut c_void,
        info: *const Info,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> *mut c_void;
    fn SelectObject(dc: *mut c_void, object: *mut c_void) -> *mut c_void;
    fn DeleteDC(dc: *mut c_void) -> i32;
    fn DeleteObject(object: *mut c_void) -> i32;
}
fn info(size: [usize; 2]) -> Result<Info, String> {
    let count = size[0]
        .checked_mul(size[1])
        .filter(|n| *n <= 16_777_216)
        .ok_or("GDI surface limit")?;
    if size.contains(&0) || size.iter().any(|n| *n > 32768) {
        return Err("Invalid GDI surface".into());
    }
    Ok(Info {
        header: Header {
            size: 40,
            width: size[0] as i32,
            height: -(size[1] as i32),
            planes: 1,
            bits: 32,
            compression: 0,
            bytes: (count * 4) as u32,
            x: 0,
            y: 0,
            colors: 0,
            important: 0,
        },
        color: [0],
    })
}
fn copy(dc: *mut c_void, size: [usize; 2], pixels: &[u32]) -> Result<(), String> {
    let info = info(size)?;
    if pixels.len() != size[0] * size[1] {
        return Err("GDI pixel length mismatch".into());
    }
    // SAFETY: header dimensions and slice length agree; GDI consumes the live borrowed slice synchronously.
    let result = unsafe {
        StretchDIBits(
            dc,
            0,
            0,
            size[0] as i32,
            size[1] as i32,
            0,
            0,
            size[0] as i32,
            size[1] as i32,
            pixels.as_ptr().cast(),
            &info,
            0,
            0x00cc0020,
        )
    };
    if result < 0 {
        return Err("GDI software presentation failed".into());
    }
    Ok(())
}
pub fn present(
    window: &winit::window::Window,
    size: [usize; 2],
    pixels: &[u32],
) -> Result<(), String> {
    let raw = window.window_handle().map_err(|e| e.to_string())?;
    let raw_window_handle::RawWindowHandle::Win32(raw) = raw.as_raw() else {
        return Err("Software presentation requires a Windows window".into());
    };
    let handle = raw.hwnd.get() as *mut c_void;
    // SAFETY: the window owner remains borrowed for the entire synchronous DC operation.
    let dc = unsafe { GetDC(handle) };
    if dc.is_null() {
        return Err("Window GDI context unavailable".into());
    }
    let result = copy(dc, size, pixels);
    // SAFETY: release the DC obtained above to the same live window.
    unsafe {
        ReleaseDC(handle, dc);
    }
    result
}
pub fn verify_memory_surface() -> Result<(), String> {
    let info = info([2, 2])?;
    // SAFETY: creates only an owned memory DC, never a desktop DC.
    let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    if dc.is_null() {
        return Err("Memory DC unavailable".into());
    }
    let mut bits = std::ptr::null_mut();
    // SAFETY: dimensions are bounded2x2, output pointer is live and initialized by GDI.
    let bitmap = unsafe { CreateDIBSection(dc, &info, 0, &mut bits, std::ptr::null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        // SAFETY: dc was just created here.
        unsafe {
            DeleteDC(dc);
        }
        return Err("Memory bitmap unavailable".into());
    }
    // SAFETY: select the owned bitmap into the owned DC; keep the old selection for restoration.
    let old = unsafe { SelectObject(dc, bitmap) };
    let pixels = [0xffff0000, 0xff00ff00, 0xff0000ff, 0xffffffff];
    let result = copy(dc, [2, 2], &pixels).and_then(|()| {
        // SAFETY: the live2x2 DIB section above owns exactly4u32 pixels; no window/desktop pixels are read.
        if unsafe { std::slice::from_raw_parts(bits.cast::<u32>(), 4) } == pixels {
            Ok(())
        } else {
            Err("GDI own-memory readback differs".into())
        }
    });
    // SAFETY: deselect before deleting the owned bitmap/DC, in inverse acquisition order.
    unsafe {
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn only_owned_dib_surface_roundtrips_exact_bgra() {
        super::verify_memory_surface().unwrap();
    }
}
