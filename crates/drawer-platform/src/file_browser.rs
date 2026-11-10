//! Local drive checks and recycle-only Shell operations; no permanent-delete API.
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        UI::Shell::*,
    },
    core::{Error, HRESULT, PCWSTR, Ref, Result as WinResult, implement},
};
pub fn local_disk(p: &Path) -> Result<(), String> {
    drawer_core::file_browser::lexical(p)?;
    let text = p
        .to_str()
        .ok_or("Non-Unicode path")?
        .trim_start_matches(r"\\?\");
    let root = [text.as_bytes()[0] as u16, b':' as u16, b'\\' as u16, 0];
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    // SAFETY: the four UTF-16 units form a checked local drive root and remain live during this call.
    match unsafe { GetDriveTypeW(root.as_ptr()) } {
        2 | 3 | 5 | 6 => Ok(()),
        4 => Err("Mapped network drives are not read by the local browser".into()),
        _ => Err("Local drive unavailable or disconnected".into()),
    }
}
fn allow_recycle(flags: u32, cancel: bool) -> bool {
    !cancel && flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0
}
#[derive(Default)]
struct State {
    post: bool,
    pre_flags: Option<u32>,
    error: Option<i32>,
    recycled: Option<String>,
}
#[implement(IFileOperationProgressSink)]
struct Sink {
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    dry_run: bool,
}
fn abort() -> Error {
    Error::from_hresult(HRESULT(0x80004004u32 as i32))
}
fn name(item: &IShellItem, kind: SIGDN) -> WinResult<String> {
    // SAFETY: item is a live COM interface; GetDisplayName returns an owned CoTaskMem string.
    let value = unsafe { item.GetDisplayName(kind) }?;
    // SAFETY: Shell guarantees a terminated UTF-16 string; copy it before releasing its allocation.
    let result = unsafe { value.to_string() };
    // SAFETY: value is exactly the Shell allocation returned above and is freed once.
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    result.map_err(|_| abort())
}
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> WinResult<()> {
        Ok(())
    }
    fn FinishOperations(&self, hr: HRESULT) -> WinResult<()> {
        if hr.is_err() {
            self.state.lock().map_err(|_| abort())?.error = Some(hr.0);
        }
        Ok(())
    }
    fn PreDeleteItem(&self, flags: u32, _item: Ref<'_, IShellItem>) -> WinResult<()> {
        self.state.lock().map_err(|_| abort())?.pre_flags = Some(flags);
        if !self.dry_run && allow_recycle(flags, self.cancel.load(Ordering::Relaxed)) {
            Ok(())
        } else {
            Err(abort())
        }
    }
    fn PostDeleteItem(
        &self,
        _flags: u32,
        _item: Ref<'_, IShellItem>,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        let mut state = self.state.lock().map_err(|_| abort())?;
        state.post = true;
        if hr.is_err() {
            state.error = Some(hr.0);
        } else if let Some(item) = new.as_ref() {
            state.recycled = Some(name(item, SIGDN_DESKTOPABSOLUTEPARSING)?);
        }
        Ok(())
    }
    fn PreRenameItem(&self, _f: u32, _i: Ref<'_, IShellItem>, _n: &PCWSTR) -> WinResult<()> {
        Err(abort())
    }
    fn PostRenameItem(
        &self,
        _f: u32,
        _i: Ref<'_, IShellItem>,
        _n: &PCWSTR,
        _h: HRESULT,
        _a: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _f: u32,
        _i: Ref<'_, IShellItem>,
        _d: Ref<'_, IShellItem>,
        _n: &PCWSTR,
    ) -> WinResult<()> {
        Err(abort())
    }
    fn PostMoveItem(
        &self,
        _f: u32,
        _i: Ref<'_, IShellItem>,
        _d: Ref<'_, IShellItem>,
        _n: &PCWSTR,
        _h: HRESULT,
        _a: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _f: u32,
        _i: Ref<'_, IShellItem>,
        _d: Ref<'_, IShellItem>,
        _n: &PCWSTR,
    ) -> WinResult<()> {
        Err(abort())
    }
    fn PostCopyItem(
        &self,
        _f: u32,
        _i: Ref<'_, IShellItem>,
        _d: Ref<'_, IShellItem>,
        _n: &PCWSTR,
        _h: HRESULT,
        _a: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreNewItem(&self, _f: u32, _d: Ref<'_, IShellItem>, _n: &PCWSTR) -> WinResult<()> {
        Err(abort())
    }
    fn PostNewItem(
        &self,
        _f: u32,
        _d: Ref<'_, IShellItem>,
        _n: &PCWSTR,
        _t: &PCWSTR,
        _a: u32,
        _h: HRESULT,
        _i: Ref<'_, IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _total: u32, _done: u32) -> WinResult<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(abort())
        } else {
            Ok(())
        }
    }
    fn ResetTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> WinResult<()> {
        Ok(())
    }
}
struct Apartment;
impl Apartment {
    fn new() -> WinResult<Self> {
        // SAFETY: each operation runs on its own owned STA thread, balanced by this guard.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balanced successful initialization on this same thread, after its interfaces drop.
        unsafe { CoUninitialize() };
    }
}
fn item(text: &str) -> WinResult<IShellItem> {
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let wide = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(abort());
    }
    // SAFETY: the terminated path stays live during the synchronous Shell parse; COM owns the result.
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
}
fn recycle_sta(
    path: &Path,
    cancel: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    dry_run: bool,
) -> WinResult<Option<String>> {
    let _sta = Apartment::new()?;
    let source = item(&path.to_string_lossy())?;
    let bound = name(&source, SIGDN_FILESYSPATH)?;
    let expected = path.to_string_lossy();
    let expected = expected.trim_start_matches(r"\\?\").replace('/', "\\");
    let actual = bound.trim_start_matches(r"\\?\").replace('/', "\\");
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(abort());
    }

    let sink: IFileOperationProgressSink = Sink {
        state: state.clone(),
        cancel: cancel.clone(),
        dry_run,
    }
    .into();
    // SAFETY: the STA is initialized, all interfaces and parameters outlive the synchronous operation.
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOperationFlags(
            FOFX_RECYCLEONDELETE
                | FOFX_ADDUNDORECORD
                | FOF_ALLOWUNDO
                | FOF_NOERRORUI
                | FOFX_EARLYFAILURE
                | FOF_NOCONFIRMATION
                | FOF_SILENT
                | FOF_NO_CONNECTED_ELEMENTS,
        )?;
        operation.DeleteItem(&source, &sink)?;
        operation.PerformOperations()?;
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Err(abort());
        }
    }
    let state = state.lock().map_err(|_| abort())?;
    if let Some(hr) = state.error {
        return Err(Error::from_hresult(HRESULT(hr)));
    }
    if !state.post {
        return Err(abort());
    }
    Ok(state.recycled.clone())
}
pub fn recycle(path: &Path, cancel: Arc<AtomicBool>) -> Result<(), String> {
    local_disk(path)?;
    let path = drawer_core::file_browser::checked(path)?;
    drawer_core::file_browser::reject_tree_links(&path, &cancel)?;
    let worker = path.clone();
    std::thread::Builder::new()
        .name("recycle-only-sta".into())
        .spawn(move || {
            recycle_sta(
                &worker,
                cancel,
                Arc::new(Mutex::new(State::default())),
                false,
            )
        })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "Recycle worker stopped".to_owned())?
        .map_err(|e| format!("Recycle unavailable/failed; no permanent-delete fallback: {e}"))?;
    if path.exists() {
        return Err("Recycle did not remove the source; no permanent fallback".into());
    }
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    if allow_recycle(0, false) || allow_recycle(128, true) || !allow_recycle(128, false) {
        return Err("Recycle callback accepted a permanent/canceled operation".into());
    }
    local_disk(root)?;
    if local_disk(Path::new(r"\\server\share\file")).is_ok() {
        return Err("Network path accepted".into());
    }
    // The boundary test never submits any delete job: it exercises callback refusal directly.
    let state = Arc::new(Mutex::new(State::default()));
    let sink: IFileOperationProgressSink = Sink {
        state: state.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
        dry_run: true,
    }
    .into();
    // SAFETY: the owned sink is live; a null item is valid for this refusal test and is never dereferenced.
    if unsafe { sink.PreDeleteItem(0, None::<&IShellItem>) }.is_ok() {
        return Err("Shell pre-delete callback did not veto permanent deletion".into());
    }
    let dir = root.join("native-recycle-guard");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = dir.join("guard-only.txt");
        std::fs::write(&path, b"owned guard fixture").map_err(|e| e.to_string())?;
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let worker = path.clone();
        let outcome = std::thread::Builder::new()
            .name("verify-recycle-veto".into())
            .spawn(move || recycle_sta(&worker, Arc::new(AtomicBool::new(false)), shared, true))
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "Recycle guard worker stopped".to_owned())?;
        if outcome.is_ok()
            || std::fs::read(&path).map_err(|e| e.to_string())? != b"owned guard fixture"
        {
            return Err("Dry-run Shell guard did not preserve its exclusive fixture".into());
        }
        let flags = state
            .lock()
            .map_err(|_| "Recycle verification state poisoned")?
            .pre_flags;
        eprintln!(
            "Shell recycle pre-delete veto: flags={flags:?}; source_unchanged=true; no_deletion_performed=true"
        );
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_drive_and_recycle_only_callback_refuse_permanent_delete() {
        super::verify(&std::env::temp_dir()).unwrap();
    }
}
