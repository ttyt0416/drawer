use super::*;
use drawer_core::{
    acquisition::{self as protocol, Device, Reply, Request},
    image_import::ImageSource,
    references::LocalImage,
};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Only this executable's explicit helper mode can access acquisition APIs.
pub(super) fn helper(directory: &Path) -> Result<(), String> {
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let request = protocol::request(&directory.join("request.json")).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let reply = match request {
        Request::Probe(ms) => {
            std::thread::sleep(Duration::from_millis(ms as u64));
            Reply::Devices {
                devices: vec![],
                warnings: vec![],
            }
        }
        Request::List => {
            let (devices, warnings) = drawer_platform::acquisition_list();
            Reply::Devices { devices, warnings }
        }
        Request::Capture(device) => match drawer_platform::acquisition_capture(&device, &directory)
        {
            Ok(path) => Reply::Image(path),
            Err(e) => Reply::Failure(e.chars().take(8000).collect()),
        },
    };
    #[cfg(not(windows))]
    let reply = {
        let _ = request;
        Reply::Failure("Local acquisition requires Windows".into())
    };
    protocol::write(&directory.join("response.json"), &reply).map_err(|e| e.to_string())
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(base: &Path) -> Result<Self, String> {
        fs::create_dir_all(base).map_err(|e| e.to_string())?;
        let base = base.canonicalize().map_err(|e| e.to_string())?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        for n in 0..16 {
            let path = base.join(format!("drawer-acquire-{}-{stamp}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Could not create private acquisition scratch directory".into())
    }
    fn size(&self) -> Result<u64, String> {
        let mut total = 0u64;
        let mut count = 0;
        for entry in fs::read_dir(&self.0).map_err(|e| e.to_string())? {
            count += 1;
            if count > 32 {
                return Err("Device created too many temporary outputs".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() || meta.is_dir() {
                return Err("Unexpected device output link/directory".into());
            }
            total = total.saturating_add(meta.len());
        }
        Ok(total)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // This exact canonical child was exclusively created above. Rust's
        // remove_dir_all does not follow symlinks. Never delete a supplied device path.
        if let (Some(parent), Some(name)) = (self.0.parent(), self.0.file_name())
            && name.to_string_lossy().starts_with("drawer-acquire-")
            && self
                .0
                .canonicalize()
                .is_ok_and(|p| p.parent() == Some(parent) && p == self.0)
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
pub(super) struct Acquired {
    pub image: LocalImage,
    pub original: Arc<Vec<u8>>,
    pub filename: String,
}
enum Value {
    Devices(Vec<Device>, Vec<String>),
    Image(Acquired),
}
fn acquired(
    directory: &Path,
    path: &Path,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Acquired, String> {
    let path = protocol::image_path(directory, path).map_err(|e| e.to_string())?;
    let source = ImageSource::read(&path, budget, cancel).map_err(|e| e.to_string())?;
    let image = source
        .decode(0, true, budget, cancel)
        .map_err(|e| e.to_string())?
        .image;
    if cancel.load(Ordering::Relaxed) {
        return Err("Acquisition cancelled".into());
    }
    let original = source.encoded_snapshot();
    Ok(Acquired {
        image,
        original,
        filename: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    })
}
fn execute(
    base: &Path,
    executable: &Path,
    request: &Request,
    budget: usize,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Value, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Acquisition cancelled".into());
    }
    let scratch = Scratch::new(base)?;
    protocol::write(&scratch.0.join("request.json"), request).map_err(|e| e.to_string())?;
    let mut command = Command::new(executable);
    command
        .arg("--acquire-helper")
        .arg(&scratch.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut process = Process(command.spawn().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    let _containment = drawer_platform::acquisition_child_job(&process.0)?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Acquisition cancelled".into());
        }
        if started.elapsed() > timeout {
            return Err("Device acquisition timed out; helper closed".into());
        }
        if scratch.size()? > protocol::MAX_BYTES + 1024 * 1024 {
            return Err("Acquired image exceeds 64 MiB; helper closed".into());
        }
        if let Some(status) = process.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Device helper failed or driver disconnected".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Acquisition cancelled".into());
    }
    match protocol::reply(&scratch.0.join("response.json")).map_err(|e| e.to_string())? {
        Reply::Devices { devices, warnings }
            if matches!(request, Request::List | Request::Probe(_)) =>
        {
            Ok(Value::Devices(devices, warnings))
        }
        Reply::Image(path) if matches!(request, Request::Capture(_)) => {
            acquired(&scratch.0, &path, budget, cancel).map(Value::Image)
        }
        Reply::Failure(e) => Err(e),
        _ => Err("Unexpected device helper response".into()),
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Value, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct Acquisition {
    pub open: bool,
    devices: Vec<Device>,
    warnings: Vec<String>,
    selected: Option<usize>,
    job: Option<Job>,
    pub ready: Option<Acquired>,
    texture: Option<egui::TextureHandle>,
    pub error: Option<String>,
    #[cfg(test)]
    buttons: [Option<egui::Rect>; 3],
}
impl Acquisition {
    fn start(&mut self, ctx: &egui::Context, base: PathBuf, request: Request, budget: usize) {
        if self.job.is_some() {
            return;
        }
        self.ready = None;
        self.texture = None;
        self.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (ctx, sender, receiver) = {
            let (tx, rx) = mpsc::channel();
            (ctx.clone(), tx, rx)
        };
        std::thread::spawn(move || {
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    execute(
                        &base,
                        &exe,
                        &request,
                        budget,
                        &signal,
                        if matches!(request, Request::List) {
                            Duration::from_secs(30)
                        } else {
                            Duration::from_secs(300)
                        },
                    )
                });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.job = Some(Job { cancel, receiver });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
        let cancelled = job.cancel.load(Ordering::Relaxed);
        let result = match job.receiver.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Device worker stopped".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            self.job = None;
            if cancelled {
                self.error = Some("Acquisition cancelled".into());
                return;
            }
            match result {
                Ok(Value::Devices(devices, warnings)) => {
                    self.devices = devices;
                    self.warnings = warnings;
                    self.selected = (!self.devices.is_empty()).then_some(0);
                }
                Ok(Value::Image(image)) => {
                    self.ready = Some(image);
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
    fn close(&mut self) {
        self.open = false;
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.ready = None;
        self.texture = None;
    }
}
impl Drawer {
    pub(super) fn acquisition_window(&mut self, ctx: &egui::Context) {
        self.acquisition.poll(ctx);
        if !self.acquisition.open {
            return;
        }
        let mut open = true;
        let mut refresh = false;
        let mut capture = false;
        let mut accept = None;
        let mut save = false;
        let mut close = false;
        egui::Window::new("Local scanner and camera").open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label("Queries devices only when you select Refresh. Capture uses the selected local device. Your documents remain unchanged until you accept a preview.");
            if let Some(job)=&self.acquisition.job {ui.label("Waiting for local device. WIA may show a scanner window. Cancel closes the helper and discards partial output.");if crate::help::button(ui,"Cancel acquisition","acquisition").clicked(){job.cancel.store(true,Ordering::Relaxed);}}
            else {
                refresh=crate::help::button(ui,"Refresh local devices","acquisition").clicked();
                if self.acquisition.devices.is_empty(){ui.label("No devices listed. Refresh to find connected WIA scanners/cameras and video cameras.");}
                for (n,d) in self.acquisition.devices.iter().enumerate(){ui.selectable_value(&mut self.acquisition.selected,Some(n),format!("{:?} — {}",d.kind,d.name));}
                capture=ui.add_enabled(self.acquisition.selected.is_some_and(|n|n<self.acquisition.devices.len()),egui::Button::new("Acquire one image")).clicked();
            }
            for warning in &self.acquisition.warnings{ui.label(warning);}
            if let Some(error)=&self.acquisition.error{ui.colored_label(egui::Color32::LIGHT_RED,error);}
            if let Some(ready)=&self.acquisition.ready {
                ui.label(format!("{} — {}×{} · original {} bytes",ready.filename,ready.image.width,ready.image.height,ready.original.len()));
                if self.acquisition.texture.is_none(){let image=&ready.image;let scale=(360.0/image.width as f64).min(280.0/image.height as f64).min(1.0);let w=(image.width as f64*scale).round().max(1.0) as usize;let h=(image.height as f64*scale).round().max(1.0) as usize;let pixels=(0..h).flat_map(|y|(0..w).map(move |x|{let p=image.pixels[(y*image.height as usize/h)*image.width as usize+x*image.width as usize/w];egui::Color32::from_rgba_unmultiplied(p[0],p[1],p[2],p[3])})).collect();self.acquisition.texture=Some(ctx.load_texture("acquired-image",egui::ColorImage::new([w,h],pixels),egui::TextureOptions::LINEAR));}
                if let Some(texture)=&self.acquisition.texture{ui.image((texture.id(),texture.size_vec2()));}
                save=crate::help::button(ui,"Save acquired original…","acquisition").clicked();
                let a=crate::help::button(ui,"Use as new document","acquisition");if a.clicked(){accept=Some(false);}
                let b=ui.add_enabled(self.active.is_some(),egui::Button::new("Place as layer…"));if b.clicked(){accept=Some(true);}
                #[cfg(test)]{self.acquisition.buttons[0]=Some(a.rect);self.acquisition.buttons[1]=Some(b.rect);}
            }
            let cancel=crate::help::button(ui,"Close acquisition","acquisition");close=cancel.clicked();#[cfg(test)]{self.acquisition.buttons[2]=Some(cancel.rect);}
        });
        if !open || close {
            self.acquisition.close();
            return;
        }
        if refresh || capture {
            let request = if capture {
                self.acquisition
                    .selected
                    .and_then(|n| self.acquisition.devices.get(n))
                    .cloned()
                    .map(Request::Capture)
            } else {
                Some(Request::List)
            };
            if let Some(request) = request {
                let budget = DEFAULT_DOCUMENT_BUDGET
                    .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
                let base = self
                    .storage
                    .as_ref()
                    .map(|s| s.root().join("acquisition-work"))
                    .unwrap_or_else(|| std::env::temp_dir().join("Drawer-acquisition"));
                self.acquisition.start(ctx, base, request, budget);
            }
        }
        if save
            && let Some(ready) = &self.acquisition.ready
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Save acquired original to a new file")
                .set_file_name(&ready.filename)
                .save_file()
        {
            match protocol::save_original(&path, &ready.original) {
                Ok(()) => {
                    self.status = Some(format!("Acquired original saved: {}", path.display()))
                }
                Err(e) => self.acquisition.error = Some(e.to_string()),
            }
        }
        if let Some(layer) = accept
            && let Err(e) = self.accept_acquired(ctx, layer)
        {
            self.acquisition.error = Some(e);
        }
    }
    pub(super) fn accept_acquired(
        &mut self,
        ctx: &egui::Context,
        layer: bool,
    ) -> Result<(), String> {
        let ready = self
            .acquisition
            .ready
            .as_ref()
            .ok_or("No acquired image preview")?;
        if self.acquisition.job.is_some() {
            return Err("Wait for acquisition to finish".into());
        }
        // The receiving operation copies pixels. Check source+destination peak
        // explicitly while the original preview is still retained for retry/save.
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        if used
            .saturating_add(ready.original.len())
            .saturating_add(ready.image.pixels.len() * 12)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Not enough memory to retain preview and insert this image".into());
        }
        if layer {
            let reserved = ready.original.len() + ready.image.pixels.len() * 4;
            let packet = drawer_core::paste::ClipboardImage {
                color: None,
                image: ready.image.clone(),
                origin: None,
            };
            self.begin_paste_reserved(ctx, || Ok(packet), reserved)?;
        } else {
            let image = ready.image.clone();
            let title = format!("Acquired — {}", ready.filename);
            self.add_image_document(image, title)?;
        }
        self.acquisition.close();
        Ok(())
    }
}
pub(super) fn smoke_pixels(base: &Path) -> Result<(), String> {
    let scratch = Scratch::new(base)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    let spec = CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.0,
        background: Background::Transparent,
    };
    let mut fixture = Document::create(spec, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    fixture.layers[0].pixels.fill([10, 20, 30, 255]);
    let path = scratch.0.join("captured.png");
    drawer_core::png_export::export(&fixture, &path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let first = acquired(&scratch.0, &path, DEFAULT_DOCUMENT_BUDGET, &cancel)?;
    if first.image.pixels != fixture.layers[0].pixels {
        return Err("Acquired decoder changed fixture pixels".into());
    }
    let saved = scratch.0.join("saved-original.png");
    protocol::save_original(&saved, &first.original).map_err(|e| e.to_string())?;
    if protocol::save_original(&saved, &first.original).is_ok()
        || fs::read(&saved).map_err(|e| e.to_string())? != *first.original
    {
        return Err("Acquired original was overwritten".into());
    }
    app.acquisition.ready = Some(first);
    app.accept_acquired(&ctx, false)?;
    if app.documents.len() != 1 || app.documents[0].layers[0].pixels != fixture.layers[0].pixels {
        return Err("Acquired new document was not copied".into());
    }
    let before = app.documents[0].snapshot();
    fixture.layers[0].pixels.fill([40, 180, 70, 255]);
    let second_path = scratch.0.join("second.png");
    drawer_core::png_export::export(&fixture, &second_path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    app.acquisition.ready = Some(acquired(
        &scratch.0,
        &second_path,
        DEFAULT_DOCUMENT_BUDGET,
        &cancel,
    )?);
    app.accept_acquired(&ctx, true)?;
    if app.documents[0].snapshot() != before {
        return Err("Acquisition placement preview changed the document".into());
    }
    app.accept_paste()?;
    let after = app.documents[0].snapshot();
    if app.documents[0].history_counts() != (1, 0)
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Acquired layer was not one reversible edit".into());
    }
    cancel.store(true, Ordering::Relaxed);
    if acquired(&scratch.0, &path, DEFAULT_DOCUMENT_BUDGET, &cancel).is_ok() {
        return Err("Cancelled acquisition decoded an image".into());
    }
    Ok(())
}
#[cfg(windows)]
pub(super) fn smoke_broker(base: &Path) -> Result<(), String> {
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let names = || -> Result<std::collections::BTreeSet<_>, String> {
        fs::read_dir(base)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|p| p.file_name()).map_err(|e| e.to_string()))
            .collect()
    };
    let before = names()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    if !matches!(execute(base,&exe,&Request::Probe(0),DEFAULT_DOCUMENT_BUDGET,&cancel,Duration::from_secs(10))?,Value::Devices(d,w) if d.is_empty() && w.is_empty())
    {
        return Err("Device protocol probe failed".into());
    }
    if names() != Ok(before.clone()) {
        return Err("Device protocol probe left temporary output".into());
    }
    if execute(
        base,
        &exe,
        &Request::Probe(2000),
        DEFAULT_DOCUMENT_BUDGET,
        &cancel,
        Duration::ZERO,
    )
    .is_ok()
    {
        return Err("Device helper timeout was ignored".into());
    }
    let signal = Arc::new(AtomicBool::new(false));
    let delayed = signal.clone();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        delayed.store(true, Ordering::Relaxed);
    });
    let result = execute(
        base,
        &exe,
        &Request::Probe(2000),
        DEFAULT_DOCUMENT_BUDGET,
        &signal,
        Duration::from_secs(10),
    );
    let _ = setter.join();
    if result.is_ok() || names() != Ok(before.clone()) {
        return Err("Device helper cancellation left output".into());
    }
    // Closing the host's job handle must itself terminate the contained helper.
    let scratch = Scratch::new(base)?;
    protocol::write(&scratch.0.join("request.json"), &Request::Probe(2000))
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(&exe);
    command.arg("--acquire-helper").arg(&scratch.0);
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut process = Process(command.spawn().map_err(|e| e.to_string())?);
    let job = drawer_platform::acquisition_child_job(&process.0)?;
    drop(job);
    let start = Instant::now();
    loop {
        if process.0.try_wait().map_err(|e| e.to_string())?.is_some() {
            if scratch.0.join("response.json").exists() {
                return Err("Device helper escaped its job".into());
            }
            break;
        }
        if start.elapsed() > Duration::from_secs(1) {
            return Err("Device job did not terminate helper".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(process);
    drop(scratch);
    if names() != Ok(before) {
        return Err("Device containment check left temporary output".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn acquired_pixels_original_new_document_layer_and_undo_are_independent() {
        let dir = TestDirectory::new();
        smoke_pixels(&dir.0).unwrap();
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    }
    #[test]
    fn preview_close_empty_devices_and_worker_failure_leave_user_document_unchanged() {
        let dir = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        app.acquisition.open = true;
        frame(&ctx, &mut app, vec![]);
        assert!(app.acquisition.job.is_none());
        assert!(app.acquisition.devices.is_empty());
        app.acquisition.ready = Some(Acquired {
            image: LocalImage::from_rgba(2, 2, &[90; 16], DEFAULT_DOCUMENT_BUDGET).unwrap(),
            original: Arc::new(vec![1, 2, 3]),
            filename: "test.png".into(),
        });
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert!(app.acquisition.texture.is_some());
        let close = app.acquisition.buttons[2].unwrap();
        click(&ctx, &mut app, close);
        assert!(!app.acquisition.open);
        assert!(app.acquisition.ready.is_none());
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].dirty, dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        let (tx, rx) = mpsc::channel();
        tx.send(Err("Device disconnected".into())).unwrap();
        app.acquisition.open = true;
        app.acquisition.job = Some(Job {
            cancel: Arc::new(AtomicBool::new(false)),
            receiver: rx,
        });
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.acquisition.error.as_deref(),
            Some("Device disconnected")
        );
        assert!(app.acquisition.job.is_none());
        assert!(app.documents[0].snapshot() == before);
        let scratch = Scratch::new(&dir.0).unwrap();
        let foreign = dir.0.join("outside.png");
        fs::write(&foreign, "outside").unwrap();
        assert!(
            acquired(
                &scratch.0,
                &foreign,
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(fs::read(&foreign).unwrap(), b"outside");
    }
}
