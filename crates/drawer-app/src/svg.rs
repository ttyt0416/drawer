use super::*;
use drawer_core::{svg_export, svg_import};
use std::{io::Read, path::Path};
#[derive(Default)]
pub(crate) struct Panel {
    draft: Option<Box<Draft>>,
}
pub(crate) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::svg_verify::verify(root)?;
    thread_local! {static COPIED:std::cell::RefCell<String>=const{std::cell::RefCell::new(String::new())};}
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 64,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let text = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><linearGradient id="g"><stop stop-color="#e02050"/><stop offset="1" stop-color="#2080e0"/></linearGradient></defs><rect x="8" y="8" width="40" height="30" fill="url(#g)"/></svg>"##;
    let source = root.join("source.svg");
    std::fs::write(&source, text).map_err(|e| e.to_string())?;
    app.execute_command(&ctx, "svg-import");
    app.load_svg_path(&source)?;
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    app.prepare_svg(&ctx)?;
    if app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != history
        || app.svg.draft.as_ref().is_none_or(|e| e.images.is_none())
    {
        return Err("SVG app preview changed original or has no before/after image".into());
    }
    let mut time = 0.;
    for _ in 0..2 {
        time += 0.1;
        crate::input_replay::frame(&ctx, &mut app, time, vec![]);
    }
    app.apply_svg()?;
    if app.documents[0].history_counts().0 != history.0 + 1 {
        return Err("SVG app import one Undo".into());
    }
    let native = root.join("svg-app.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("SVG app native reopen".into());
    }
    let imported = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("drawer-text-clipboard"),
            crate::edit::Clipboard {
                write: |text| {
                    COPIED.with(|c| *c.borrow_mut() = text.into());
                    Ok(())
                },
                read: || COPIED.with(|c| Ok(c.borrow().clone())),
            },
        )
    });
    app.copy_svg(&ctx)?;
    let output = root.join("app-export.svg");
    app.save_svg_to(&output)?;
    let saved = std::fs::read_to_string(&output).map_err(|e| e.to_string())?;
    if !COPIED.with(|c| *c.borrow() == saved)
        || drawer_core::svg_verify::reference(&saved, 64, 64)?
            .iter()
            .all(|p| p[3] == 0)
        || app.documents[0].snapshot() != imported
        || app.documents[0].history_counts() != count
        || std::fs::read_to_string(&source).map_err(|e| e.to_string())? != text
    {
        return Err("SVG copy/file policy, independent reader, source SVG preservation or export Undo isolation".into());
    }
    app.paste_svg(&ctx)?;
    if app
        .svg
        .draft
        .as_ref()
        .is_none_or(|e| e.path.is_some() || e.text != saved)
    {
        return Err("SVG clipboard paste did not select text source".into());
    }
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("drawer-text-clipboard"),
            crate::edit::Clipboard {
                write: |_| Err("Clipboard unavailable".into()),
                read: || Err("Clipboard unavailable".into()),
            },
        )
    });
    let text_before = app.svg.draft.as_ref().unwrap().text.clone();
    if app.paste_svg(&ctx).is_ok()
        || app.svg.draft.as_ref().unwrap().text != text_before
        || app.copy_svg(&ctx).is_ok()
        || app.documents[0].snapshot() != imported
    {
        return Err("SVG clipboard failure changed source/draft or was hidden".into());
    }
    app.svg.draft.as_mut().unwrap().text = "<svg>".into();
    if app.prepare_svg(&ctx).is_ok() || app.documents[0].snapshot() != imported {
        return Err("Malformed SVG app import added partial objects".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("SVG app import Undo restore".into());
    }
    Ok(())
}

struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    text: String,
    path: Option<PathBuf>,
    import: Option<svg_import::Prepared>,
    export: Option<svg_export::Export>,
    images: Option<[egui::TextureHandle; 2]>,
    notices: Vec<String>,
}
impl Draft {
    fn new(d: &Document) -> Self {
        Self {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            text: String::new(),
            path: None,
            import: None,
            export: None,
            images: None,
            notices: vec![],
        }
    }
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
}
impl Drawer {
    pub(crate) fn begin_svg(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if self.svg.draft.as_ref().is_some_and(|e| e.fresh(d)) {
            return Ok(());
        }
        let mut next = Draft::new(d);
        if let Some(previous) = self.svg.draft.take() {
            next.text = previous.text;
            next.path = previous.path;
        }
        self.svg.draft = Some(Box::new(next));
        Ok(())
    }
    pub(crate) fn begin_svg_export(&mut self) -> Result<(), String> {
        self.begin_svg()?;
        self.generate_svg()
    }
    fn load_svg_path(&mut self, path: &Path) -> Result<(), String> {
        let mut bytes = vec![];
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(svg_import::MAX_TEXT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > svg_import::MAX_TEXT {
            return Err("SVG source limit2MiB".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "SVG source must be UTF-8")?;
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        let e = self.svg.draft.as_mut().ok_or("Open SVG import first")?;
        e.text = text;
        e.path = Some(canonical);
        e.import = None;
        e.images = None;
        e.notices.clear();
        Ok(())
    }
    fn paste_svg(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let text = (crate::edit::clipboard(ctx).read)()?;
        if text.len() > svg_import::MAX_TEXT {
            return Err("Clipboard SVG exceeds2MiB".into());
        }
        let e = self.svg.draft.as_mut().ok_or("Open SVG import first")?;
        e.text = text;
        e.path = None;
        e.import = None;
        e.images = None;
        e.notices.clear();
        Ok(())
    }
    fn prepare_svg(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self.svg.draft.as_mut().ok_or("SVG window closed")?;
        if !e.fresh(d) {
            return Err("Page/frame changed; reload the current page first".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("SVG preview exceeds workspace memory".into());
        }
        let prepared = svg_import::prepare(d, &e.text, e.path.as_deref().and_then(Path::parent))?;
        let after = crate::drawing::bucket::preview(d, prepared.state.clone())?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "svg-before"),
            crate::layer_merge::image(ctx, &after, "svg-after"),
        ]);
        e.import = Some(prepared);
        Ok(())
    }
    fn apply_svg(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self.svg.draft.as_mut().ok_or("SVG window closed")?;
        if !e.fresh(&self.documents[n]) {
            return Err("Page/frame changed; reload the current page first".into());
        }
        let p = e.import.take().ok_or("Prepare SVG import first")?;
        if p.paths + p.images == 0 {
            e.import = Some(p);
            return Err("No supported SVG objects to import; review the item notices".into());
        }
        self.documents[n].commit_state(p.state)?;
        if let Some(id) = p.layer {
            self.panels.targets.insert(
                self.documents[n].id,
                drawer_core::thumbnail::Target::Vector(id),
            );
        } else if p.images > 0 {
            self.panels.targets.insert(
                self.documents[n].id,
                drawer_core::thumbnail::Target::Raster(self.documents[n].layers.len() - 1),
            );
        }
        e.revision = self.documents[n].revision;
        e.frame = self.documents[n].current_frame;
        e.images = None;
        e.export = None;
        e.notices = p.warnings.clone();
        self.status = Some(format!(
            "SVG imported: {} editable outlines, {} embedded images; {} conversion notices",
            p.paths,
            p.images,
            p.warnings.len()
        ));
        Ok(())
    }
    fn generate_svg(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self.svg.draft.as_mut().ok_or("SVG window closed")?;
        if !e.fresh(d) {
            return Err("Page/frame changed; reload the current page first".into());
        }
        e.export = Some(svg_export::generate(d)?);
        Ok(())
    }
    pub(crate) fn copy_svg(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.begin_svg()?;
        self.generate_svg()?;
        let e = self.svg.draft.as_ref().unwrap();
        let out = e.export.as_ref().unwrap();
        (crate::edit::clipboard(ctx).write)(&out.text)?;
        self.status = Some(format!(
            "SVG text copied; {} conversion notices in the SVG window",
            out.warnings.len()
        ));
        Ok(())
    }
    fn save_svg_to(&mut self, path: &Path) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self.svg.draft.as_ref().ok_or("SVG window closed")?;
        if !e.fresh(d) {
            return Err("Page/frame changed; prepare the export again".into());
        }
        svg_export::write(
            path,
            &e.export.as_ref().ok_or("Prepare SVG export first")?.text,
        )
    }
    pub(crate) fn svg_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.svg.draft.take() else {
            return;
        };
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let fresh = e.fresh(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (
            mut load,
            mut prepare,
            mut apply,
            mut export,
            mut save,
            mut copy,
            mut reload,
            mut paste,
        ) = (false, false, false, false, false, false, false, false);
        egui::Window::new("Local SVG import and export").open(&mut open).default_width(470.).default_height(600.).vscroll(true).default_pos(egui::pos2(30.,55.)).show(ctx,|ui|{
            ui.horizontal(|ui|{load=ui.button("Load SVG file…").clicked();paste=ui.button("Paste SVG text").clicked();reload=ui.button("Reload current page").clicked();});
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame changed. Reload before preparing or applying.");}
            ui.label("Paste SVG text here, or load a local UTF-8 file (2MiB maximum).");
            let changed=egui::ScrollArea::vertical().max_height(155.).show(ui,|ui|ui.add(egui::TextEdit::multiline(&mut e.text).code_editor().desired_rows(8).desired_width(f32::INFINITY).char_limit(svg_import::MAX_TEXT))).inner.changed();
            if changed{e.import=None;e.images=None;e.notices.clear();}
            if e.path.is_some() && ui.button("Use text source without a local folder").clicked(){e.path=None;e.import=None;e.images=None;}
            ui.small(if e.path.is_some(){"Local PNG/JPEG resources are restricted to the selected SVG folder. Imports freeze their pixels."}else{"Clipboard/text SVG has no local resource directory. Network references, scripts, text and reusable instances are skipped with notices."});
            prepare=ui.add_enabled(fresh && !e.text.is_empty() && e.text.len()<=svg_import::MAX_TEXT,egui::Button::new("Prepare SVG import preview")).clicked();
            if let Some(p)=&e.import{ui.label(format!("SVG {}×{} pixels · {} editable outlines · {} embedded images",p.size[0],p.size[1],p.paths,p.images));for warning in &p.warnings{ui.small(warning);}}
            else if !e.notices.is_empty(){ui.label("Last import notices");for warning in &e.notices{ui.small(warning);}}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}
            apply=ui.add_enabled(fresh && e.import.as_ref().is_some_and(|p|p.paths+p.images>0),egui::Button::new("Import as one Undo")).clicked();
            ui.small("SVG strokes become editable filled outlines, with caps, joins and dashes baked. Gradients and clipping remain local vector data. Vector recoloring replaces an imported gradient with a solid fill.");
            ui.separator();export=ui.add_enabled(fresh,egui::Button::new("Prepare current page SVG export")).clicked();
            if let Some(out)=&e.export{ui.label(format!("{} SVG bytes · {} conversion notices",out.text.len(),out.warnings.len()));for warning in &out.warnings{ui.small(warning);}ui.horizontal(|ui|{save=ui.add_enabled(fresh,egui::Button::new("Save SVG…")).clicked();copy=ui.add_enabled(fresh,egui::Button::new("Copy SVG text")).clicked();});}
            ui.small("Variable widths and nonround tips become vector outlines. Unsupported image/texture or compositing sources use embedded PNG. The .drawer source, its selection and Undo remain unchanged by export/copy.");
        });
        if reload {
            let mut next = Draft::new(d);
            next.text = e.text;
            next.path = e.path;
            e = Box::new(next);
        }
        if copy && let Some(out) = &e.export {
            self.status = Some(match (crate::edit::clipboard(ctx).write)(&out.text) {
                Ok(()) => "SVG text copied with the prepared export policy".into(),
                Err(error) => error,
            });
        }
        if open {
            self.svg.draft = Some(e);
        }
        if paste && let Err(error) = self.paste_svg(ctx) {
            self.status = Some(error);
        }
        if load
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("SVG", &["svg"])
                .pick_file()
            && let Err(error) = self.load_svg_path(&path)
        {
            self.status = Some(error);
        }
        if prepare && let Err(error) = self.prepare_svg(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_svg() {
            self.status = Some(error);
        }
        if export && let Err(error) = self.generate_svg() {
            self.status = Some(error);
        }
        if save
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("SVG", &["svg"])
                .set_file_name("drawing.svg")
                .save_file()
            && let Err(error) = self.save_svg_to(&path)
        {
            self.status = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn svg_app_preview_commit_clipboard_failures_source_preservation_save_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
