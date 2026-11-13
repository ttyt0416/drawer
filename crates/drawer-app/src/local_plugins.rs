use super::*;
use drawer_core::{plugin_runtime as runtime, sizing::DocumentState};
use std::{
    io::{Read, Write},
    sync::atomic::AtomicBool,
};

const EXAMPLE: &str =
    r#"{"version":1,"name":"Local invert","instructions":[{"op":"Invert"},{"op":"End"}]}"#;
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    text: String,
    prepared: Option<(u64, u64, u32, DocumentState)>,
    image: Option<egui::TextureHandle>,
    preview: Option<egui::Rect>,
    adopt: Option<egui::Rect>,
}
fn read(path: &std::path::Path) -> Result<String, String> {
    let path = drawer_core::file_browser::checked(path)?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular local plugin file".into());
    }
    let mut bytes = Vec::new();
    file.take(runtime::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    runtime::parse(&bytes)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
impl Drawer {
    pub(super) fn begin_local_plugins(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.active.is_none() {
            return Err("Open a document first".into());
        }
        self.local_plugins.open = true;
        self.local_plugins.prepared = None;
        self.local_plugins.image = None;
        if self.local_plugins.text.is_empty() {
            self.local_plugins.text = EXAMPLE.into();
        }
        Ok(())
    }
    fn install_local_plugin(&mut self) -> Result<(), String> {
        runtime::parse(self.local_plugins.text.as_bytes())?;
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local user storage unavailable")?;
        let dir = drawer_core::file_browser::new_path(&storage.root().join("plugins"))?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let bytes = self.local_plugins.text.as_bytes();
        let path = drawer_core::file_browser::new_path(
            &dir.join(format!("{}.json", drawer_infer::hash(bytes))),
        )?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(bytes)
                    .and_then(|()| f.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        if read(&path)?.as_bytes() != bytes {
            return Err("Existing plugin copy differs; preserved".into());
        }
        self.status = Some(format!("Plugin stored locally: {}", path.display()));
        Ok(())
    }
    fn preview_local_plugin(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.local_plugins.prepared = None;
        self.local_plugins.image = None;
        let program = runtime::parse(self.local_plugins.text.as_bytes())?;
        let d = &self.documents[self.active.ok_or("Open a document first")?];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let cost = d
            .memory_bytes()
            .saturating_mul(5)
            .saturating_add(16 * 1024 * 1024);
        if used.saturating_add(cost) > self.resource_budget()
            || (d.width as u64) * (d.height as u64) > runtime::MAX_PIXELS as u64
        {
            return Err("Plugin preview exceeds the local memory/one-megapixel limit".into());
        }
        let cancel = AtomicBool::new(false);
        let composite = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        let input = drawer_core::local_inference::snapshot(&composite, false, &cancel)?;
        let output = runtime::run(
            self.local_plugins.text.as_bytes(),
            &input,
            self.resource_budget().saturating_sub(used + cost),
            runtime::MAX_FUEL,
            &cancel,
        )?;
        let mut state = drawer_core::paste::prepare(
            d,
            &output,
            [0, 0],
            self.resource_budget().saturating_sub(used),
        )?;
        state
            .layers
            .last_mut()
            .ok_or("Plugin output layer missing")?
            .name = format!("Plugin: {}", program.name);
        drawer_core::layer_catalog::reconcile(d, &mut state)?;
        let scale = (160.0 / output.width.max(output.height) as f32).min(1.0);
        let w = (output.width as f32 * scale).round().max(1.) as usize;
        let h = (output.height as f32 * scale).round().max(1.) as usize;
        let pixels = (0..h)
            .flat_map(|y| {
                let out = &output;
                (0..w).map(move |x| {
                    let p = out.pixels[y * out.height as usize / h * out.width as usize
                        + x * out.width as usize / w];
                    egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
                })
            })
            .collect();
        self.local_plugins.image = Some(ctx.load_texture(
            "restricted-plugin",
            egui::ColorImage::new([w, h], pixels),
            egui::TextureOptions::NEAREST,
        ));
        self.local_plugins.prepared = Some((d.id, d.revision, d.current_frame, state));
        Ok(())
    }
    fn adopt_local_plugin(&mut self) -> Result<(), String> {
        let (id, revision, frame, state) = self
            .local_plugins
            .prepared
            .as_ref()
            .ok_or("Preview a plugin result first")?;
        let d = &mut self.documents[self.active.ok_or("Document closed")?];
        if (d.id, d.revision, d.current_frame) != (*id, *revision, *frame) {
            return Err("Document changed; preview again".into());
        }
        d.commit_state(state.clone())?;
        self.local_plugins.prepared = None;
        self.local_plugins.image = None;
        self.local_plugins.open = false;
        self.sync_layer_identity();
        Ok(())
    }
    pub(super) fn local_plugins_window(&mut self, ctx: &egui::Context) {
        if !self.local_plugins.open {
            return;
        }
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut load, mut install, mut preview, mut adopt, mut cancel) =
            (false, false, false, false, false);
        egui::Window::new(crate::i18n::text("Restricted local plugins")).id(egui::Id::new("Restricted local plugins")).order(egui::Order::Foreground)
            .default_width(540.).collapsible(false).open(&mut open).show(ctx,|ui|{
                ui.label(crate::i18n::text("Pixel bytecode v1 · no network, files, processes or native code"));
                ui.small("256 KiB program · 4096 instructions · 1 megapixel · 2,097,152 execution units. Only a frozen page composite and its dimensions are exposed.");
                if ui.add(egui::TextEdit::multiline(&mut self.local_plugins.text).char_limit(runtime::MAX_BYTES).desired_rows(6)).changed() {
                    self.local_plugins.prepared=None;self.local_plugins.image=None;
                }
                load=ui.button(crate::i18n::text("Choose local plugin…")).clicked();
                install=ui.button(crate::i18n::text("Keep a verified copy in local user data")).clicked();
                let r=ui.button(crate::i18n::text("Preview on frozen page"));self.local_plugins.preview=Some(r.rect);preview=r.clicked();
                if let Some(image)=&self.local_plugins.image {ui.add(egui::Image::new(image));}
                let r=ui.add_enabled(self.local_plugins.prepared.is_some(),egui::Button::new(crate::i18n::text("Adopt as new layer · one Undo")));
                self.local_plugins.adopt=Some(r.rect);adopt=r.clicked();
                cancel=ui.button(crate::i18n::text("Discard / close")).clicked();
            });
        if !open || cancel {
            self.local_plugins.open = false;
            self.local_plugins.prepared = None;
            self.local_plugins.image = None;
            return;
        }
        let result = (|| -> Result<(), String> {
            if load
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Restricted plugin", &["json"])
                    .pick_file()
            {
                // Invalidate the previous result even when the next import fails.
                self.local_plugins.prepared = None;
                self.local_plugins.image = None;
                self.local_plugins.text = read(&path)?;
            }
            if install {
                self.install_local_plugin()?;
            }
            if preview {
                self.preview_local_plugin(ctx)?;
            }
            if adopt {
                self.adopt_local_plugin()?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.status = Some(e);
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let storage = UserStorage::at(root.join("plugin-user")).map_err(|e| e.to_string())?;
    let mut app = Box::new(Drawer::new(&ctx, None).with_storage(storage.clone()));
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.simple_view = true;
    app.documents[0].layers[0].pixels.fill([255; 4]);
    app.execute_command(&ctx, "local-plugins");
    if !app.local_plugins.open {
        return Err("Plugin command unavailable".into());
    }
    app.install_local_plugin()?;
    let path = storage
        .root()
        .join("plugins")
        .join(format!("{}.json", drawer_infer::hash(EXAMPLE.as_bytes())));
    if read(&path)? != EXAMPLE {
        return Err("Local plugin copy/reopen failed".into());
    }
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    for _ in 0..3 {
        time += 0.1;
        crate::input_replay::frame(&ctx, &mut app, time, vec![]);
    }
    let preview = app
        .local_plugins
        .preview
        .ok_or("Plugin preview control missing")?;
    click(&ctx, &mut app, &mut time, preview);
    if app.local_plugins.prepared.is_none() {
        return Err(format!("Plugin preview control failed: {:?}", app.status));
    }
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != counts {
        return Err("Plugin preview changed document".into());
    }
    time += 0.1;
    crate::input_replay::frame(&ctx, &mut app, time, vec![]);
    let adopt = app
        .local_plugins
        .adopt
        .ok_or("Plugin adoption control missing")?;
    click(&ctx, &mut app, &mut time, adopt);
    let after = app.documents[0].snapshot();
    if after == before
        || app.documents[0].history_counts().0 != counts.0 + 1
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Plugin adoption/Undo/Redo failed".into());
    }
    app.begin_local_plugins()?;
    app.preview_local_plugin(&ctx)?;
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    if app.adopt_local_plugin().is_ok() {
        return Err("Stale plugin result adopted".into());
    }
    let stable = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    for op in [
        r#"{"op":"Jump","instruction":0}"#,
        r#"{"op":"Abort"}"#,
        r#"{"op":"network"}"#,
        r#"{"op":"Set","index":4294967295,"rgba":[1,2,3,255]}"#,
    ] {
        app.local_plugins.text =
            format!(r#"{{"version":1,"name":"Failure","instructions":[{op}]}}"#);
        if app.preview_local_plugin(&ctx).is_ok()
            || app.local_plugins.prepared.is_some()
            || app.documents[0].snapshot() != stable
            || app.documents[0].history_counts() != counts
        {
            return Err("Plugin failure retained output or changed host document".into());
        }
    }
    app.local_plugins.text = EXAMPLE.into();
    app.preview_local_plugin(&ctx)?;
    crate::input_replay::frame(
        &ctx,
        &mut app,
        1.,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.local_plugins.prepared.is_some() || app.documents[0].snapshot() != stable {
        return Err("Plugin cancellation changed document".into());
    }
    Ok(())
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, rect: egui::Rect) {
    for pressed in [true, false] {
        *time += 0.1;
        crate::input_replay::frame(
            ctx,
            app,
            *time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn restricted_plugin_install_preview_adopt_undo_failure_stale_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
