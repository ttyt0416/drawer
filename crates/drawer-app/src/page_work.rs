use super::*;
use drawer_core::{
    page_work::{self as core, Edit, Model, Navigation},
    story_text::Work,
    workspaces::Dock,
};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    book: Option<Model>,
    manifest: Option<PathBuf>,
    loaded: BTreeMap<u64, u64>,
    previews: BTreeMap<u64, Document>,
    stamps: BTreeMap<u64, (Option<std::time::SystemTime>, u64)>,
    pub scroll: f32,
    images: BTreeMap<u64, (u64, u64, u32, egui::TextureHandle)>,
    title: String,
    right_binding: bool,
    first_right: bool,
    number: usize,
    move_to: usize,
    last_scroll: Option<u64>,
    notice: String,
    pub next: Option<egui::Rect>,
    pub delete: Option<egui::Rect>,
    pub undo: Option<egui::Rect>,
    pub redo: Option<egui::Rect>,
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::page_work_verify::verify(root)?;
    let dir = root.join("page-ui-fixture");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut paths = vec![];
        for n in 0..3 {
            let mut d = Document::create(
                CanvasSpec {
                    width: 2,
                    height: 2,
                    dpi: 300.,
                    background: Background::Transparent,
                },
                DEFAULT_DOCUMENT_BUDGET,
            )
            .map_err(|e| e.to_string())?;
            d.layers[0].pixels.fill([n as u8 * 40, 80, 120, 255]);
            let path = dir.join(format!("page-{n}.drawer"));
            drawer_core::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            paths.push(path.canonicalize().map_err(|e| e.to_string())?);
        }
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.open_path(&paths[0])?;
        app.documents[0].edit_pixel(0, 0, 0, [200, 100, 80, 255]);
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        app.page_work.book = Some(Model::new(Work::new("Pages".into(), paths.clone())?)?);
        app.page_work.title = "Pages".into();
        app.workspaces.store.current.page_view.facing = false;
        app.workspaces.store.current.page_view.dock = Dock::Left;
        app.execute_command(&ctx, "page-manager");
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            input_replay::frame(&ctx, app, *time, events);
        };
        let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                step(
                    app,
                    time,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        let r = app.page_work.next.ok_or("Page navigation button missing")?;
        click(&mut app, &mut time, r);
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app.page_work.book.as_ref().unwrap().current() != 1 || app.documents.len() != 2 {
            return Err(format!("Page navigation failed: {}", app.page_work.notice));
        }
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != counts
            || !app.documents[0].dirty
        {
            return Err("Page navigation discarded unsaved data or changed drawing history".into());
        }
        app.workspaces.store.current.page_view.facing = true;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        let loads = app.page_work.book.as_ref().unwrap().loads;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app.page_work.book.as_ref().unwrap().loads != loads || loads > 2 {
            return Err("Facing preview eagerly reloaded closed pages".into());
        }
        let ids = app.page_work.book.as_ref().unwrap().work.page_ids.clone();
        let r = app.page_work.delete.ok_or("Page delete button missing")?;
        click(&mut app, &mut time, r);
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app.page_work.book.as_ref().unwrap().work.pages.len() != 2 || app.documents.len() != 2 {
            return Err("Page removal closed an open drawing or removed the wrong page".into());
        }
        let r = app.page_work.undo.ok_or("Page Undo button missing")?;
        click(&mut app, &mut time, r);
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app.page_work.book.as_ref().unwrap().work.page_ids != ids {
            return Err("Page GUI Undo lost page IDs/order".into());
        }
        let snapshot = core::snapshot(
            app.page_work.book.as_ref().unwrap(),
            &dir.join("shared.drawer-work"),
            &[(1, &app.documents[0]), (2, &app.documents[1])],
            &mut |_, _| Ok(()),
        )?;
        app.page_work.book.as_mut().unwrap().work = snapshot;
        let snapshot_path = app.page_work.book.as_ref().unwrap().work.pages[0].clone();
        if app.page_work_document_id(&snapshot_path) != Some(app.documents[0].id) {
            return Err("Project snapshot detached live page identity".into());
        }
        app.smoke_project_story_binding(app.documents[0].id)?;
        let many = (0..512)
            .map(|i| {
                if i == 0 {
                    paths[0].clone()
                } else if i == 511 {
                    paths[2].clone()
                } else {
                    dir.join(format!("unloaded-{i}.drawer"))
                }
            })
            .collect();
        app.page_work.book = Some(Model::new(Work::new("512 pages".into(), many)?)?);
        app.page_work.loaded.clear();
        app.page_work.images.clear();
        app.page_work.previews.clear();
        app.page_work.last_scroll = None;
        app.workspaces.store.current.page_view.facing = false;
        app.workspaces.store.current.page_view.auto_scroll = true;
        let mut panel = std::mem::take(&mut *app.page_work);
        app.page_open(&mut panel, 512, false)?;
        *app.page_work = panel;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        let offset = app.page_work.scroll;
        if offset < 1000. {
            return Err(format!("Active thumbnail did not auto-scroll: {offset}"));
        }
        app.workspaces.store.current.page_view.auto_scroll = false;
        let mut panel = std::mem::take(&mut *app.page_work);
        app.page_open(&mut panel, 1, false)?;
        *app.page_work = panel;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if (app.page_work.scroll - offset).abs() > 2. {
            return Err("Auto-scroll off did not retain list position".into());
        }
        app.workspaces.store.current.page_view.auto_scroll = true;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app.page_work.scroll > 2. {
            return Err("Re-enabling auto-scroll did not reveal the active page".into());
        }
        let tabs = app.tabs.len();
        let docs = app.documents.len();
        let mut panel = std::mem::take(&mut *app.page_work);
        app.page_open(&mut panel, 1, true)?;
        *app.page_work = panel;
        if app.documents.len() != docs
            || app.tabs.len() != tabs + 1
            || app.tabs.last().unwrap().document_id != app.documents[0].id
        {
            return Err("New page view did not share the original drawing data".into());
        }
        for dock in [Dock::Right, Dock::Floating, Dock::Left] {
            app.workspaces.store.current.page_view.dock = dock;
            for _ in 0..3 {
                step(&mut app, &mut time, vec![]);
            }
        }
        if app.documents[0].snapshot() != before || !app.documents[0].dirty {
            return Err("Dock/view operations modified the original page".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
impl Drawer {
    pub(crate) fn active_page_work(&self) -> Option<Work> {
        self.page_work.book.as_ref().map(|b| b.work.clone())
    }
    pub(crate) fn page_work_document_id(&self, path: &std::path::Path) -> Option<u64> {
        let book = self.page_work.book.as_ref()?;
        let key = |p: &std::path::Path| {
            p.to_string_lossy()
                .replace('/', "\\")
                .trim_start_matches("\\\\?\\")
                .to_lowercase()
        };
        let target = key(path);
        let index = book
            .work
            .pages
            .iter()
            .position(|p| key(p) == target)
            .or_else(|| {
                book.work
                    .pages
                    .iter()
                    .position(|p| p.canonicalize().ok().as_deref() == Some(path))
            })?;
        self.page_work
            .loaded
            .get(&book.work.page_ids[index])
            .copied()
            .filter(|id| self.documents.iter().any(|d| d.id == *id))
    }
    pub(super) fn begin_page_work(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed() || self.inference_unconfirmed() {
            return Err("Apply or cancel pending changes first".into());
        }
        if self.page_work.book.is_none()
            && let Some(path) = self.workspaces.store.current.page_view.project.clone()
        {
            let mut book = Model::load(&path)?;
            if let Some(id) = self.workspaces.store.current.page_view.selected
                && book.index(id).is_ok()
            {
                book.selected = id;
            }
            self.page_work.title = book.work.title.clone();
            self.page_work.right_binding = book.work.right_binding;
            self.page_work.first_right = book.work.first_right;
            self.page_work.book = Some(book);
            self.page_work.manifest = Some(path);
        }
        self.page_work.open = true;
        Ok(())
    }
    fn page_image(
        &mut self,
        p: &mut Panel,
        ctx: &egui::Context,
        id: u64,
    ) -> Result<egui::TextureHandle, String> {
        let book = p.book.as_mut().ok_or("Open a project")?;
        let i = book.index(id)?;
        let path = &book.work.pages[i];
        let live = p
            .loaded
            .get(&id)
            .and_then(|id| self.documents.iter().find(|d| d.id == *id))
            .or_else(|| {
                self.documents
                    .iter()
                    .find(|d| d.path.as_ref() == Some(path))
            });
        if live.is_none() {
            let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
            let stamp = (meta.modified().ok(), meta.len());
            if p.stamps.get(&id) != Some(&stamp) {
                p.previews.remove(&id);
                p.images.remove(&id);
                p.stamps.insert(id, stamp);
            }
        }
        if live.is_none() && !p.previews.contains_key(&id) {
            if p.previews.len() >= 4 {
                p.previews.clear();
            }
            let available = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
                self.documents
                    .iter()
                    .map(Document::resident_bytes)
                    .sum::<usize>()
                    + p.previews
                        .values()
                        .map(Document::resident_bytes)
                        .sum::<usize>(),
            );
            p.previews.insert(
                id,
                book.open_page(id, available.min(DEFAULT_DOCUMENT_BUDGET / 8))?,
            );
        }
        let d = live
            .or_else(|| p.previews.get(&id))
            .ok_or("Page preview unavailable")?;
        let key = (d.id, d.revision, d.current_frame);
        if let Some((a, b, c, image)) = p.images.get(&id)
            && (*a, *b, *c) == key
        {
            return Ok(image.clone());
        }
        let renderer = drawer_core::render::Renderer::new(d);
        if let Some(error) = renderer.error() {
            return Err(error.into());
        }
        let scale = (96. / d.width as f64).min(128. / d.height as f64);
        let size = [
            (d.width as f64 * scale).round().max(1.) as usize,
            (d.height as f64 * scale).round().max(1.) as usize,
        ];
        let mut pixels = Vec::with_capacity(size[0] * size[1]);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let c = renderer.pixel([
                    (x as f64 + 0.5) * d.width as f64 / size[0] as f64,
                    (y as f64 + 0.5) * d.height as f64 / size[1] as f64,
                ]);
                pixels.push(egui::Color32::from_rgba_unmultiplied(
                    c[0], c[1], c[2], c[3],
                ));
            }
        }
        let image = ctx.load_texture(
            format!("page-preview-{id}"),
            egui::ColorImage::new(size, pixels),
            egui::TextureOptions::LINEAR,
        );
        if p.images.len() > 8 {
            p.images.clear();
        }
        p.images.insert(id, (key.0, key.1, key.2, image.clone()));
        Ok(image)
    }
    fn page_open(&mut self, p: &mut Panel, id: u64, new_view: bool) -> Result<(), String> {
        let book = p.book.as_mut().ok_or("Open a project")?;
        let i = book.index(id)?;
        let expected = if let Some(n) = p
            .loaded
            .get(&id)
            .and_then(|v| self.documents.iter().position(|d| d.id == *v))
            .or_else(|| {
                self.documents
                    .iter()
                    .position(|d| d.path.as_ref() == Some(&book.work.pages[i]))
            }) {
            let tab = self
                .tabs
                .iter()
                .position(|t| t.document_id == self.documents[n].id)
                .ok_or("Page view is missing")?;
            let expected = self.documents[n].id;
            self.select_tab(tab);
            expected
        } else {
            if self.tabs.len() >= 256 {
                return Err("The workspace supports up to256 shared views".into());
            }
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            let d = book.open_page(id, DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))?;
            let expected = d.id;
            self.add_document(d);
            expected
        };
        let n = self.active.ok_or("Page did not open")?;
        if self.documents[n].id != expected {
            return Err("Finish the pending path before changing the page view".into());
        }
        if new_view && self.tabs.len() >= 256 {
            return Err("The workspace supports up to256 shared views".into());
        }
        p.loaded.insert(id, self.documents[n].id);
        book.selected = id;
        if new_view {
            self.duplicate_view();
        }
        p.previews.remove(&id);
        Ok(())
    }
    fn page_new(&mut self, p: &mut Panel, duplicate: bool) -> Result<(), String> {
        if p.book.as_ref().is_some_and(|b| b.work.pages.len() >= 512) {
            return Err("Project page limit512".into());
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Drawer page", &["drawer"])
            .set_file_name(if duplicate {
                "page-copy.drawer"
            } else {
                "page.drawer"
            })
            .save_file()
        else {
            return Ok(());
        };
        if path.exists() {
            return Err("Choose a new page filename".into());
        }
        if duplicate {
            let id = p.book.as_ref().ok_or("Open a project")?.selected;
            self.page_open(p, id, false)?;
            core::duplicate(&self.documents[self.active.unwrap()], &path)?;
        } else {
            let spec = self
                .active
                .map(|n| {
                    let d = &self.documents[n];
                    CanvasSpec {
                        width: d.width,
                        height: d.height,
                        dpi: d.dpi,
                        background: Background::Transparent,
                    }
                })
                .unwrap_or(CanvasSpec {
                    width: 1024,
                    height: 1024,
                    dpi: 300.,
                    background: Background::Transparent,
                });
            let color = self
                .active
                .map(|n| self.documents[n].color.clone())
                .unwrap_or_default();
            let mut d = Document::create_with_color(spec, DEFAULT_DOCUMENT_BUDGET / 8, color)
                .map_err(|e| e.to_string())?;
            drawer_core::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if let Some(book) = &mut p.book {
            book.edit(Edit::Add {
                path: path.clone(),
                index: book.current() + 1,
            })?;
            book.selected = book.work.page_ids[book.current() + 1];
        } else {
            p.book = Some(Model::new(Work::new(
                "Local work".into(),
                vec![path.clone()],
            )?)?);
            p.title = "Local work".into();
            p.first_right = true;
        }
        let id = p.book.as_ref().unwrap().selected;
        self.page_open(p, id, false)
    }
    fn page_save_snapshot(&mut self, p: &mut Panel) -> Result<(), String> {
        let book = p.book.as_ref().ok_or("Open a project")?;
        let destination = if let Some(path) = &p.manifest {
            path.clone()
        } else {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("Drawer project", &["drawer-work"])
                .set_file_name("project.drawer-work")
                .save_file()
            else {
                return Ok(());
            };
            path
        };
        let refs: Vec<_> = book
            .work
            .page_ids
            .iter()
            .zip(&book.work.pages)
            .filter_map(|(id, path)| {
                p.loaded
                    .get(id)
                    .and_then(|id| self.documents.iter().find(|d| d.id == *id))
                    .or_else(|| {
                        self.documents
                            .iter()
                            .find(|d| d.path.as_ref() == Some(path))
                    })
                    .map(|d| (*id, d))
            })
            .collect();
        let work = core::snapshot(book, &destination, &refs, &mut |_, _| Ok(()))?;
        p.book.as_mut().unwrap().work = work;
        p.manifest = Some(destination);
        p.previews.clear();
        p.images.clear();
        self.workspaces.store.current.page_view.project = p.manifest.clone();
        self.persist_workspace()?;
        p.notice="Project snapshot saved. Original page files and their individual save state are preserved.".into();
        Ok(())
    }
    fn page_work_ui(&mut self, ui: &mut egui::Ui) {
        let mut p = std::mem::take(&mut *self.page_work);
        let old_prefs = self.workspaces.store.current.page_view.clone();
        let mut action = 0;
        let mut open_id = None;
        let mut navigation = None;
        let mut result = Ok(());
        ui.heading("Page manager");
        ui.horizontal(|ui| {
            for dock in Dock::ALL {
                ui.radio_value(
                    &mut self.workspaces.store.current.page_view.dock,
                    dock,
                    format!("{dock:?}"),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.workspaces.store.current.page_view.auto_scroll,
                "Auto-scroll",
            );
            ui.checkbox(
                &mut self.workspaces.store.current.page_view.facing,
                "Facing pages",
            );
        });
        ui.checkbox(
            &mut self.workspaces.store.current.page_view.new_view,
            "Open in a new shared view",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open project…").clicked() {
                action = 1;
            }
            if ui.button("Use saved open pages").clicked() {
                action = 2;
            }
            if ui.button("New blank page…").clicked() {
                action = 3;
            }
            if ui.button("Add native page…").clicked() {
                action = 4;
            }
            if ui.button("Save project snapshot…").clicked() {
                action = 5;
            }
        });
        if let Some(book) = &mut p.book {
            if let Some(n) = self.active {
                let d = &self.documents[n];
                if let Some(id) =
                    book.work
                        .page_ids
                        .iter()
                        .zip(&book.work.pages)
                        .find_map(|(id, path)| {
                            (p.loaded.get(id) == Some(&d.id) || d.path.as_ref() == Some(path))
                                .then_some(*id)
                        })
                {
                    book.selected = id;
                    p.loaded.insert(id, d.id);
                }
            }
            ui.label(format!(
                "{} · {} pages",
                book.work.title,
                book.work.pages.len()
            ));
            ui.add(egui::TextEdit::singleline(&mut p.title).char_limit(64));
            ui.checkbox(&mut p.right_binding, "Right binding");
            ui.checkbox(&mut p.first_right, "First page on right");
            if ui.button("Apply work information").clicked() {
                action = 6;
            }
            ui.horizontal_wrapped(|ui| {
                for (label, n) in [
                    ("First", Navigation::First),
                    ("Previous", Navigation::Previous),
                    ("Next", Navigation::Next),
                    ("Last", Navigation::Last),
                ] {
                    let r = ui.button(label);
                    if label == "Next" {
                        p.next = Some(r.rect);
                    }
                    if r.clicked() {
                        navigation = Some(n);
                    }
                }
                ui.add(egui::DragValue::new(&mut p.number).range(0..=513));
                if ui.button("Go to page").clicked() {
                    navigation = Some(Navigation::Number(p.number));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open selected").clicked() {
                    open_id = Some(book.selected);
                }
                if ui.button("Duplicate page…").clicked() {
                    action = 7;
                }
                let r = ui.add_enabled(
                    book.work.pages.len() > 1,
                    egui::Button::new("Remove from work"),
                );
                p.delete = Some(r.rect);
                if r.clicked() {
                    action = 8;
                }
                if ui.button("Move earlier").clicked() {
                    action = 9;
                }
                if ui.button("Move later").clicked() {
                    action = 10;
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut p.move_to).range(1..=512));
                if ui.button("Move to position").clicked() {
                    action = 13;
                }
                let (a, b) = book.counts();
                let r = ui.add_enabled(a > 0, egui::Button::new("Undo page change"));
                p.undo = Some(r.rect);
                if r.clicked() {
                    action = 11;
                }
                let r = ui.add_enabled(b > 0, egui::Button::new("Redo page change"));
                p.redo = Some(r.rect);
                if r.clicked() {
                    action = 12;
                }
            });
            let selected = book.selected;
            let current = book.current();
            let face = book.facing(current);
            let pages = book.work.pages.clone();
            let ids = book.work.page_ids.clone();
            let facing = self.workspaces.store.current.page_view.facing;
            let mut thumbs = vec![];
            for id in std::iter::once(selected).chain(face.filter(|_| facing).map(|n| ids[n])) {
                match self.page_image(&mut p, ui.ctx(), id) {
                    Ok(image) => thumbs.push((id, image)),
                    Err(error) => p.notice = error,
                }
            }
            if facing && face.is_some() {
                let right = p.book.as_ref().unwrap().work.first_right == current.is_multiple_of(2);
                if right {
                    thumbs.reverse();
                }
            }
            ui.horizontal(|ui| {
                for (id, image) in &thumbs {
                    ui.vertical(|ui| {
                        ui.image((image.id(), image.size_vec2()));
                        ui.label(format!(
                            "Page {}",
                            ids.iter().position(|v| v == id).unwrap() + 1
                        ));
                    });
                }
            });
            let follow = self.workspaces.store.current.page_view.auto_scroll
                && p.last_scroll != Some(selected);
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("page-manager-list")
                .max_height(ui.available_height().max(120.) - 45.)
                .vertical_scroll_offset(self.workspaces.store.current.page_view.scroll);
            if follow {
                scroll = scroll.vertical_scroll_offset(current as f32 * 44.);
            }
            let output = scroll.show_rows(ui, 44., pages.len(), |ui, range| {
                for n in range {
                    ui.horizontal(|ui| {
                        if ids[n] == selected
                            && let Some((_, image)) = thumbs.iter().find(|(id, _)| *id == selected)
                        {
                            ui.image((image.id(), egui::vec2(24., 32.)));
                        }
                        if ui
                            .selectable_label(
                                ids[n] == selected,
                                format!(
                                    "{} · {}",
                                    n + 1,
                                    pages[n].file_name().unwrap_or_default().to_string_lossy()
                                ),
                            )
                            .clicked()
                        {
                            open_id = Some(ids[n]);
                        }
                    });
                }
            });
            p.scroll = output.state.offset.y;
            self.workspaces.store.current.page_view.scroll = p.scroll;
            p.last_scroll = if self.workspaces.store.current.page_view.auto_scroll {
                Some(selected)
            } else {
                None
            };
            self.workspaces.store.current.page_view.selected = Some(selected);
        } else {
            ui.label("Open a local project, use saved pages, or create a blank page. Only viewed pages are loaded; closed pages remain on disk.");
        }
        if !p.notice.is_empty() {
            ui.label(&p.notice);
        }
        if ui.button("Close page manager").clicked() {
            p.open = false;
        }
        let operation = (|| -> Result<(), String> {
            match action {
                1 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drawer project", &["drawer-work"])
                        .pick_file()
                    {
                        let book = Model::load(&path)?;
                        p.title = book.work.title.clone();
                        p.right_binding = book.work.right_binding;
                        p.first_right = book.work.first_right;
                        p.book = Some(book);
                        p.manifest = Some(path);
                        self.workspaces.store.current.page_view.project = p.manifest.clone();
                        p.loaded.clear();
                        p.previews.clear();
                        p.images.clear();
                        p.last_scroll = None;
                    }
                }
                2 => {
                    let paths = self
                        .documents
                        .iter()
                        .map(|d| d.path.clone().ok_or("Save all open pages first"))
                        .collect::<Result<Vec<_>, _>>()?;
                    let book = Model::new(Work::new(
                        if p.title.trim().is_empty() {
                            "Local work".into()
                        } else {
                            p.title.clone()
                        },
                        paths,
                    )?)?;
                    p.title = book.work.title.clone();
                    p.first_right = book.work.first_right;
                    p.right_binding = book.work.right_binding;
                    p.book = Some(book);
                    p.manifest = None;
                    p.loaded.clear();
                    p.previews.clear();
                    p.images.clear();
                }
                3 => self.page_new(&mut p, false)?,
                4 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drawer page", &["drawer"])
                        .pick_file()
                    {
                        if let Some(book) = &mut p.book {
                            book.edit(Edit::Add {
                                path,
                                index: book.current() + 1,
                            })?;
                        } else {
                            p.book = Some(Model::new(Work::new(
                                "Local work".into(),
                                vec![path.canonicalize().map_err(|e| e.to_string())?],
                            )?)?);
                            p.title = "Local work".into();
                        }
                    }
                }
                5 => self.page_save_snapshot(&mut p)?,
                6 => p.book.as_mut().ok_or("Open a work")?.edit(Edit::Metadata {
                    title: p.title.clone(),
                    right_binding: p.right_binding,
                    first_right: p.first_right,
                })?,
                7 => self.page_new(&mut p, true)?,
                8 => {
                    let book = p.book.as_mut().ok_or("Open a work")?;
                    book.edit(Edit::Delete(book.selected))?;
                    p.notice="Page removed from the project list. Its file and open views are retained; Undo restores its ID and order.".into();
                }
                9 | 10 | 13 => {
                    let book = p.book.as_mut().ok_or("Open a work")?;
                    let n = if action == 9 {
                        book.current().saturating_sub(1)
                    } else if action == 10 {
                        (book.current() + 1).min(book.work.pages.len() - 1)
                    } else {
                        p.move_to.checked_sub(1).ok_or("Page position starts at1")?
                    };
                    book.edit(Edit::Move {
                        id: book.selected,
                        index: n,
                    })?;
                }
                11 => {
                    p.book.as_mut().ok_or("Open a work")?.undo();
                }
                12 => {
                    p.book.as_mut().ok_or("Open a work")?.redo();
                }
                _ => {}
            }
            if let Some(nav) = navigation {
                let book = p.book.as_mut().ok_or("Open a work")?;
                let old = book.selected;
                let id = book.navigate(nav)?;
                if let Err(error) =
                    self.page_open(&mut p, id, self.workspaces.store.current.page_view.new_view)
                {
                    p.book.as_mut().unwrap().selected = old;
                    return Err(error);
                }
            }
            if let Some(id) = open_id {
                self.page_open(&mut p, id, self.workspaces.store.current.page_view.new_view)?;
            }
            Ok(())
        })();
        if let Err(error) = operation {
            result = Err(error);
        }
        if old_prefs != self.workspaces.store.current.page_view
            && let Err(error) = self.persist_workspace()
        {
            result = Err(error);
        }
        if let Err(error) = result {
            p.notice = error;
        }
        *self.page_work = p;
    }
    pub(super) fn page_work_dock(&mut self, ui: &mut egui::Ui) {
        if !self.page_work.open {
            return;
        }
        let prefs = self.workspaces.store.current.page_view.clone();
        let panel = match prefs.dock {
            Dock::Left => egui::Panel::left("work-pages-left"),
            Dock::Right => egui::Panel::right("work-pages-right"),
            Dock::Floating => return,
        };
        let r = panel
            .resizable(true)
            .default_size(prefs.width)
            .size_range(180. ..=800.)
            .show(ui, |ui| self.page_work_ui(ui));
        if !ui.input(|i| i.pointer.any_down()) {
            self.workspaces.store.current.page_view.width =
                r.response.rect.width().clamp(180., 800.);
            if *self.workspaces.store.current.page_view != *prefs
                && let Err(error) = self.persist_workspace()
            {
                self.status = Some(error);
            }
        }
    }
    pub(super) fn page_work_window(&mut self, ctx: &egui::Context) {
        if !self.page_work.open || self.workspaces.store.current.page_view.dock != Dock::Floating {
            return;
        }
        let prefs = self.workspaces.store.current.page_view.clone();
        let mut open = true;
        let r = egui::Window::new("Page manager")
            .id(egui::Id::new("work-pages-floating"))
            .open(&mut open)
            .default_pos(egui::pos2(prefs.position[0], prefs.position[1]))
            .default_size(prefs.size)
            .show(ctx, |ui| self.page_work_ui(ui));
        self.page_work.open &= open;
        if let Some(r) = r
            && !ctx.input(|i| i.pointer.any_down())
        {
            let rect = r.response.rect;
            let next = &mut self.workspaces.store.current.page_view;
            next.position = [rect.left(), rect.top()];
            next.size = [
                rect.width().clamp(180., 2000.),
                rect.height().clamp(180., 2000.),
            ];
            if *self.workspaces.store.current.page_view != *prefs
                && let Err(error) = self.persist_workspace()
            {
                self.status = Some(error);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn page_manager_pointer_navigation_delete_undo_shared_views_dock_and_lazy_scroll() {
        let root = std::env::temp_dir().join(format!("drawer-page-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::smoke(&root).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
