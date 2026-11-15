use crate::*;
use drawer_core::{asset_catalog::hash, sizing::DocumentState};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    format: String,
    version: u32,
    case: String,
    source_sha256: String,
    parent_pid: u32,
}
fn write(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(&bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())
}
fn bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Owned graphics record exceeds4MiB".into());
    }
    Ok(bytes)
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut cases = vec![];
    for case in ["init", "context", "memory", "restart"] {
        let dir = root.join(format!("graphics-{case}"));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        if case == "restart" {
            let settings = dir.join("data/settings");
            std::fs::create_dir_all(&settings).map_err(|e| e.to_string())?;
            std::fs::copy(
                root.join("graphics-context/data/settings/graphics.json"),
                settings.join("graphics.json"),
            )
            .map_err(|e| e.to_string())?;
        }
        let mut doc = Document::create(
            CanvasSpec {
                width: 64,
                height: 48,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        doc.edit_pixel(0, 1, 1, [70, 90, 110, 255]);
        let source = dir.join("source.drawer");
        drawer_core::files::save(&mut doc, &source).map_err(|e| e.to_string())?;
        let source_hash = hash(&bytes(&source)?);
        let config = Config {
            format: "Drawer owned graphics case".into(),
            version: 1,
            case: case.into(),
            source_sha256: source_hash.clone(),
            parent_pid: std::process::id(),
        };
        let config_path = dir.join("config.json");
        write(&config_path, &config)?;
        let expected = hash(&bytes(&config_path)?);
        let errors = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("child.stderr"))
            .map_err(|e| e.to_string())?;
        let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .arg("--verify-graphics-case")
            .arg(&dir)
            .arg(&expected)
            .env("DRAWER_DATA_DIR", dir.join("data"))
            .stdout(Stdio::null())
            .stderr(errors);
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        let mut child = OwnedChild(command.spawn().map_err(|e| e.to_string())?);
        let start = Instant::now();
        loop {
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err(format!(
                        "Graphics {case} worker failed: {}",
                        String::from_utf8_lossy(&bytes(&dir.join("child.stderr"))?)
                    ));
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(120) {
                return Err(format!("Graphics {case} worker timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let result: serde_json::Value =
            serde_json::from_slice(&bytes(&dir.join("result.json"))?).map_err(|e| e.to_string())?;
        if result["passed"] != true
            || result["softwareDisplay"] != true
            || result["gpuFloatColorReferencePassed"] != (case != "restart")
            || result["freshProcessSkippedGpu"] != (case == "restart")
            || result["sourceSha256"] != source_hash
            || result["penPhysicalValidated"] != false
        {
            return Err("Incomplete graphics evidence".into());
        }
        if hash(&bytes(&source)?) != source_hash {
            return Err("Graphics recovery changed source file".into());
        }
        cases.push(serde_json::json!({"case":case,"workerPid":child.0.id(),"configSha256":expected,"result":result}));
    }
    Ok(
        serde_json::json!({"mode":"native GPU initialization + owned failure injection + actual CPU/GDI window","cases":cases,
        "criterionComplete":false,"gpuMaximumPermittedQuantizationError":1,
        "realDriverRemovalTested":false,"gpuOomExhaustionInduced":false,"hardwareScope":"GPU identities recorded per case; other GPU/driver combinations are untested"}),
    )
}
pub(super) fn cli(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let root = PathBuf::from(args.next().ok_or("Missing graphics root")?);
    let expected = args
        .next()
        .and_then(|s| s.into_string().ok())
        .ok_or("Missing graphics config hash")?;
    if args.next().is_some() {
        return Err("Unexpected graphics worker argument".into());
    }
    let root = drawer_core::file_browser::folder(&root)?;
    let data =
        PathBuf::from(std::env::var_os("DRAWER_DATA_DIR").ok_or("Missing owned graphics profile")?);
    if data.file_name() != Some(std::ffi::OsStr::new("data"))
        || drawer_core::file_browser::folder(data.parent().ok_or("Missing profile parent")?)?
            != root
    {
        return Err("Graphics profile outside owned case".into());
    }
    let config_bytes = bytes(&root.join("config.json"))?;
    if hash(&config_bytes) != expected {
        return Err("Graphics config hash mismatch".into());
    }
    let config: Config = serde_json::from_slice(&config_bytes).map_err(|e| e.to_string())?;
    if config.format != "Drawer owned graphics case"
        || config.version != 1
        || hash(&bytes(&root.join("source.drawer"))?) != config.source_sha256
    {
        return Err("Invalid owned graphics source".into());
    }
    write(
        &root.join("worker.json"),
        &serde_json::json!({"format":config.format,"version":1,"pid":std::process::id(),"parentPid":config.parent_pid,"case":config.case}),
    )?;
    let injection = match config.case.as_str() {
        "init" => Some(graphics::Injection::Init),
        "context" => Some(graphics::Injection::Context),
        "memory" => Some(graphics::Injection::Memory),
        "restart" => None,
        _ => return Err("Unknown graphics case".into()),
    };
    let source = root.join("source.drawer");
    let factory: graphics::Factory = Box::new(move |ctx| {
        let mut app = Drawer::new(ctx, None)
            .with_storage(UserStorage::from_environment().map_err(|e| e.to_string())?);
        app.open_path(&source)?;
        app.simple_view = true;
        app.set_tool(BrushTip::Pen);
        app.panels.settings.brush_size = 1.;
        app.workspaces.store.current.input.smoothing = [0.; 7];
        for t in &mut app.workspaces.store.current.pressure.tools {
            t.size = false;
            t.opacity = false;
        }
        for t in &mut app.workspaces.store.current.input.mouse_pressure {
            t.enabled = false;
        }
        app.references.session.workspace.color = [180, 20, 40, 255];
        if !app.documents[0].edit_pixel(0, 2, 1, [19, 87, 142, 255]) {
            return Err("Graphics seed edit unavailable".into());
        }
        app.documents[0].history_barrier();
        Ok(Box::new(app))
    });
    let input = drawer_platform::Input::default();
    let install = input.clone();
    let options = eframe::NativeOptions {
        event_loop_builder: Some(Box::new(move |b| install.install(b))),
        viewport: egui::ViewportBuilder::default()
            .with_visible(false)
            .with_inner_size([1100., 760.]),
        ..Default::default()
    };
    drawer_platform::graphics::verify_memory_surface()?;
    graphics::run(options, factory, input, false, false, injection, Some(root))
}
pub(super) struct Case {
    root: PathBuf,
    before: Option<DocumentState>,
    history: usize,
    pub finished: bool,
}
impl Case {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            before: None,
            history: 0,
            finished: false,
        }
    }
    pub fn input(
        &mut self,
        app: &mut Drawer,
        frame: u64,
        raw: &mut egui::RawInput,
    ) -> Result<(), String> {
        if frame == 2 {
            self.before = Some(app.documents[0].snapshot());
            self.history = app.documents[0].history_counts().0;
            if self.history != 1 || app.documents[0].layers[0].pixels[66] != [19, 87, 142, 255] {
                return Err("GPU handoff lost seed edit/history".into());
            }
        }
        if !(2..=4).contains(&frame) {
            return Ok(());
        }
        let rect = app
            .canvas_view
            .viewport_rect()
            .ok_or("Software canvas viewport missing")?;
        let d = &app.documents[0];
        let doc = if frame == 2 {
            [10.5, 20.5]
        } else {
            [35.5, 20.5]
        };
        let p = d.view.document_to_screen(
            doc,
            [d.width as f64, d.height as f64],
            [rect.center().x as f64, rect.center().y as f64],
            f64::from(
                raw.viewports[&egui::ViewportId::ROOT]
                    .native_pixels_per_point
                    .unwrap_or(1.),
            ) * f64::from(app.interface.settings.zoom),
        );
        let p = egui::pos2(p[0] as f32, p[1] as f32);
        raw.events.push(egui::Event::PointerMoved(p));
        if frame == 2 || frame == 4 {
            raw.events.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: frame == 2,
                modifiers: egui::Modifiers::NONE,
            });
        }
        Ok(())
    }
    pub fn completed(
        &mut self,
        app: &mut Drawer,
        frame: u64,
        pixels: &[u32],
        gpu: Option<[String; 3]>,
        gpu_color: bool,
        reason: Option<String>,
    ) -> Result<bool, String> {
        if frame < 4 {
            return Ok(false);
        }
        let before = self.before.as_ref().ok_or("Missing software baseline")?;
        let d = &mut app.documents[0];
        let after = d.snapshot();
        if after == *before || d.history_counts().0 != self.history + 1 {
            return Err(format!(
                "Software mouse stroke did not commit one command: changed={}, history={:?}, active={:?}, status={:?}, viewport={:?}",
                after != *before,
                d.history_counts(),
                app.active,
                app.status,
                app.canvas_view.viewport_rect()
            ));
        }
        if !d.undo() || d.snapshot() != *before || !d.redo() || d.snapshot() != after {
            return Err("Software recovery Undo/Redo mismatch".into());
        }
        app.save_to(0, &self.root.join("after-fallback.drawer"))?;
        if drawer_core::files::open(
            &self.root.join("after-fallback.drawer"),
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?
        .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Software saved document did not reopen exactly".into());
        }
        let storage = app
            .storage
            .as_ref()
            .ok_or("Missing graphics test profile")?;
        let policy = graphics_settings::Settings::read(storage)?;
        let restart = self.root.file_name() == Some(std::ffi::OsStr::new("graphics-restart"));
        if !policy.software
            || policy.last_gpu.is_none()
            || (restart && (gpu_color || gpu.is_some()))
            || (!restart && (!gpu_color || gpu.is_none()))
        {
            return Err("Graphics policy/GPU reference evidence incomplete".into());
        }
        let source_hash = hash(&bytes(&self.root.join("source.drawer"))?);
        let framebuffer = hash(
            &pixels
                .iter()
                .flat_map(|p| p.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        write(
            &self.root.join("result.json"),
            &serde_json::json!({"passed":true,"softwareDisplay":true,"cpuFrames":frame+1,"gpu":gpu,
            "gpuFloatColorReferencePassed":gpu_color,"failure":reason,"sourceSha256":source_hash,"framebufferSha256":framebuffer,
            "retainedUndoCommands":self.history,"newMouseStrokeUndoRedo":true,"saveReopenExact":true,"restartPreferencePersisted":true,
            "freshProcessSkippedGpu":restart,"recordedGpu":policy.last_gpu,
            "ownGdiMemoryReadback":true,"penPhysicalValidated":false}),
        )?;
        self.finished = true;
        Ok(true)
    }
}
