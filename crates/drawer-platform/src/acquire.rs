//! Invoked only in the isolated acquisition helper, never during app startup.
use drawer_core::acquisition::{Device, Kind, MAX_BYTES};
use std::path::{Path, PathBuf};
use windows::{
    Devices::Enumeration::{DeviceClass, DeviceInformation},
    Media::{
        Capture::{
            MediaCapture, MediaCaptureInitializationSettings, PhotoCaptureSource,
            StreamingCaptureMode,
        },
        MediaProperties::ImageEncodingProperties,
    },
    Storage::Streams::{DataReader, InMemoryRandomAccessStream},
    Win32::{
        Devices::ImageAcquisition::{
            IWiaDevMgr2, WIA_DEVICE_DIALOG_SINGLE_IMAGE, WIA_DEVICE_DIALOG_USE_COMMON_UI,
            WIA_DEVINFO_ENUM_LOCAL, WIA_DIP_DEV_ID, WIA_DIP_DEV_NAME, WIA_DIP_DEV_TYPE, WiaDevMgr2,
        },
        Foundation::HWND,
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
                StructuredStorage::{
                    PROPSPEC, PROPSPEC_0, PROPVARIANT, PRSPEC_PROPID, PropVariantClear,
                    PropVariantToStringAlloc, PropVariantToUInt32,
                },
            },
            WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        },
        UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
            PeekMessageW, QS_ALLINPUT, TranslateMessage,
        },
    },
    core::{BSTR, HSTRING},
};
struct Com;
impl Com {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: helper main thread owns one STA initialization; guard balances
        // even S_FALSE and drops after all interfaces in each operation.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: balanced successful initialization on this same helper thread.
        unsafe { CoUninitialize() };
    }
}
struct Runtime;
impl Runtime {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: MediaCapture's first initialization must be on STA. No objects or
        // callbacks are moved to other threads. Guard is created before all objects.
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }?;
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: balanced successful WinRT initialization on the same thread.
        unsafe { RoUninitialize() };
    }
}
fn manager() -> windows::core::Result<IWiaDevMgr2> {
    // SAFETY: called with a live COM apartment; typed CLSID/interface pair, no
    // aggregation. Only the OS WIA service is instantiated, never a remote host.
    unsafe { CoCreateInstance(&WiaDevMgr2, None, CLSCTX_LOCAL_SERVER) }
}
struct Properties([PROPVARIANT; 3]);
impl Drop for Properties {
    fn drop(&mut self) {
        for p in &mut self.0 {
            // SAFETY: default-initialized outputs from ReadMultiple, each cleared once.
            let _ = unsafe { PropVariantClear(p) };
        }
    }
}
fn string(value: &PROPVARIANT) -> windows::core::Result<String> {
    // SAFETY: live initialized property; API returns an owned nul-terminated
    // CoTaskMem string, copied synchronously and freed on both conversion paths.
    let p = unsafe { PropVariantToStringAlloc(value) }?;
    // SAFETY: string allocation and termination are guaranteed by that API.
    let s = unsafe { p.to_string() };
    // SAFETY: this is exactly the allocation returned above, freed once.
    unsafe { CoTaskMemFree(Some(p.0.cast())) };
    Ok(s?)
}
fn wia_devices() -> windows::core::Result<Vec<Device>> {
    let _com = Com::new()?;
    let manager = manager()?;
    // SAFETY: apartment-owned service, local devices flag and typed result.
    let enumerator = unsafe { manager.EnumDeviceInfo(WIA_DEVINFO_ENUM_LOCAL as i32) }?;
    let specs = [WIA_DIP_DEV_ID, WIA_DIP_DEV_NAME, WIA_DIP_DEV_TYPE].map(|propid| PROPSPEC {
        ulKind: PRSPEC_PROPID,
        Anonymous: PROPSPEC_0 { propid },
    });
    let mut devices = Vec::new();
    for _ in 0..64 {
        let mut item = None;
        let mut fetched = 0;
        // SAFETY: capacity for exactly one initialized interface output and count.
        unsafe { enumerator.Next(1, &mut item, &mut fetched) }?;
        if fetched == 0 {
            break;
        }
        let Some(item) = item else {
            break;
        };
        let mut properties = Properties(std::array::from_fn(|_| PROPVARIANT::default()));
        // SAFETY: three PROPSPEC IDs and three initialized matching outputs, which
        // are released by the guard even on partial COM failure.
        unsafe { item.ReadMultiple(3, specs.as_ptr(), properties.0.as_mut_ptr()) }?;
        // SAFETY: live property output, conversion validates its numeric type.
        let kind = match unsafe { PropVariantToUInt32(&properties.0[2]) }? & 0xffff {
            1 => Kind::WiaScanner,
            2 => Kind::WiaCamera,
            _ => continue,
        };
        let d = Device {
            kind,
            id: string(&properties.0[0])?,
            name: string(&properties.0[1])?,
        };
        if d.valid() {
            devices.push(d);
        }
    }
    Ok(devices)
}
/// Pump the helper's STA while WinRT completes. The parent process independently
/// enforces cancellation/timeout; no blocking WinRT join can deadlock its UI.
fn wait(mut pending: impl FnMut() -> windows::core::Result<bool>) -> windows::core::Result<()> {
    while pending()? {
        // SAFETY: current helper thread's own queue; valid local MSG, no foreign
        // window hook/injection. A short message-aware wait preserves STA consent
        // and capture callbacks while the host remains an independent process.
        unsafe {
            MsgWaitForMultipleObjectsEx(None, 10, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}
fn cameras() -> windows::core::Result<Vec<Device>> {
    let _runtime = Runtime::new()?;
    let op = DeviceInformation::FindAllAsyncDeviceClass(DeviceClass::VideoCapture)?;
    wait(|| Ok(op.Status()?.0 == 0))?;
    let list = op.GetResults()?;
    let mut devices = Vec::new();
    for i in 0..list.Size()?.min(64) {
        let info = list.GetAt(i)?;
        let d = Device {
            kind: Kind::Camera,
            id: info.Id()?.to_string(),
            name: info.Name()?.to_string(),
        };
        if d.valid() {
            devices.push(d);
        }
    }
    Ok(devices)
}
pub fn acquisition_list() -> (Vec<Device>, Vec<String>) {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    for (label, result) in [("WIA", wia_devices()), ("Camera", cameras())] {
        match result {
            Ok(mut found) => devices.append(&mut found),
            Err(e) => warnings.push(format!("{label}: {e}")),
        }
    }
    if devices.len() > 64 {
        devices.truncate(64);
        warnings.push("Local device list limited to 64 entries".into());
    }
    (devices, warnings)
}
struct Capture(MediaCapture);
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
fn camera(device: &Device, directory: &Path) -> Result<PathBuf, String> {
    let result = (|| -> windows::core::Result<Vec<u8>> {
        let _runtime = Runtime::new()?;
        let capture = Capture(MediaCapture::new()?);
        let settings = MediaCaptureInitializationSettings::new()?;
        settings.SetVideoDeviceId(&HSTRING::from(&device.id))?;
        settings.SetStreamingCaptureMode(StreamingCaptureMode::Video)?; // no microphone
        settings.SetPhotoCaptureSource(PhotoCaptureSource::Auto)?;
        let init = capture.0.InitializeWithSettingsAsync(&settings)?;
        wait(|| Ok(init.Status()?.0 == 0))?;
        init.GetResults()?;
        let stream = InMemoryRandomAccessStream::new()?;
        let photo = capture
            .0
            .CapturePhotoToStreamAsync(&ImageEncodingProperties::CreatePng()?, &stream)?;
        wait(|| Ok(photo.Status()?.0 == 0))?;
        photo.GetResults()?;
        let size = stream.Size()?;
        if size == 0 || size > MAX_BYTES {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x8007000du32 as i32),
                "Camera image exceeds 64 MiB",
            ));
        }
        let input = stream.GetInputStreamAt(0)?;
        let reader = DataReader::CreateDataReader(&input)?;
        let load = reader.LoadAsync(size as u32)?;
        wait(|| Ok(load.Status()?.0 == 0))?;
        if load.GetResults()? as u64 != size {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8007000du32 as i32,
            )));
        }
        let mut bytes = vec![0; size as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    })()
    .map_err(|e| {
        format!(
            "Camera capture failed (check connection, availability and Windows camera access): {e}"
        )
    })?;
    let path = directory.join("camera.png");
    drawer_core::acquisition::save_original(&path, &result).map_err(|e| e.to_string())?;
    Ok(path)
}
struct FileNames {
    pointer: *mut BSTR,
    count: i32,
}
impl Drop for FileNames {
    fn drop(&mut self) {
        if self.pointer.is_null() {
            return;
        }
        if (0..=128).contains(&self.count) {
            for i in 0..self.count as usize {
                // SAFETY: successful/partial GetImageDlg output is an array of count
                // BSTRs. Each element is moved out once and its allocation released.
                drop(unsafe { self.pointer.add(i).read() });
            }
        }
        // SAFETY: GetImageDlg array uses CoTaskMem; element allocations freed above.
        // An invalid count is rejected and helper exits without reading the array.
        unsafe { CoTaskMemFree(Some(self.pointer.cast())) };
    }
}
fn wia(device: &Device, directory: &Path) -> Result<PathBuf, String> {
    let _com = Com::new().map_err(|e| e.to_string())?;
    let manager = manager().map_err(|e| e.to_string())?;
    let mut item: Option<windows::Win32::Devices::ImageAcquisition::IWiaItem2> = None;
    let mut names = FileNames {
        pointer: std::ptr::null_mut(),
        count: 0,
    };
    let id = BSTR::from(&device.id);
    let folder = BSTR::from(directory.to_string_lossy().as_ref());
    let filename = BSTR::from("scan");
    // SAFETY: typed WIA COM vtable and live STA, BSTR and HWND values. Outputs
    // match the public ABI and windows-rs transparent BSTR/interface layouts.
    // Call the vtable to retain S_FALSE: the generated Result<()> wrapper erases
    // cancellation, and WIA may return partial files even when the user cancels.
    // Output guards free partial arrays/interfaces before returning any failure.
    let hr = unsafe {
        use windows::core::Interface;
        (Interface::vtable(&manager).GetImageDlg)(
            Interface::as_raw(&manager),
            (WIA_DEVICE_DIALOG_SINGLE_IMAGE | WIA_DEVICE_DIALOG_USE_COMMON_UI) as i32,
            id.as_ptr().cast_mut().cast(),
            HWND::default(),
            folder.as_ptr().cast_mut().cast(),
            filename.as_ptr().cast_mut().cast(),
            &mut names.count,
            (&mut names.pointer as *mut *mut BSTR).cast(),
            (&mut item as *mut Option<_>).cast(),
        )
    };
    if hr.0 == 1 {
        return Err("Acquisition cancelled; partial files discarded".into());
    }
    if hr.0 > 1 {
        return Err("Unexpected WIA transfer status; output discarded".into());
    }
    hr.ok()
        .map_err(|e| format!("WIA device unavailable or acquisition failed: {e}"))?;
    if names.count == 0 {
        return Err("Acquisition cancelled".into());
    }
    if names.count < 0 || names.count > 128 || names.pointer.is_null() {
        return Err("Invalid WIA image response".into());
    }
    if names.count != 1 {
        return Err("Select one image for this acquisition".into());
    }
    // SAFETY: validated nonnull array of count initialized BSTRs; copy only first
    // filename. Guard owns all outputs; no borrowed pointer escapes this scope.
    let path = PathBuf::from(unsafe { &*names.pointer }.to_string());
    drawer_core::acquisition::image_path(directory, &path).map_err(|e| e.to_string())
}
pub fn acquisition_capture(device: &Device, directory: &Path) -> Result<PathBuf, String> {
    if !device.valid() {
        return Err("Invalid local device".into());
    }
    match device.kind {
        Kind::Camera => camera(device, directory),
        Kind::WiaScanner | Kind::WiaCamera => wia(device, directory),
    }
}
