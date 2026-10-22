use super::*;
use drawer_core::{
    panels::{Bar, Kind, Settings, Thumbnail},
    thumbnail::Target,
};
use std::collections::HashMap;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ThumbnailKey {
    document: u64,
    revision: u64,
    frame: u32,
    target: Target,
    size: usize,
    content: bool,
    checker: bool,
}
pub(super) struct Panels {
    pub settings: Settings,
    saved: Option<Settings>,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    pub open: bool,
    kind: Kind,
    pub targets: HashMap<u64, Target>,
    thumbnails: HashMap<ThumbnailKey, egui::TextureHandle>,
    strokes: Vec<egui::TextureHandle>,
    pub(super) previous_color: [u8; 4],
    #[cfg(test)]
    pub targets_buttons: HashMap<Target, egui::Rect>,
}
impl Panels {
    pub fn new(ctx: &egui::Context) -> Self {
        let strokes = BrushTip::ALL
            .iter()
            .map(|tip| {
                let mask = tip.mask(16);
                let mut pixels = vec![egui::Color32::TRANSPARENT; 64 * 20];
                for y in 0..20 {
                    for x in 0..64 {
                        let alpha = (8..=56)
                            .filter_map(|stamp| {
                                let mx = x as isize - stamp + 8;
                                let my = y as isize - 2;
                                if (0..16).contains(&mx) && (0..16).contains(&my) {
                                    Some(mask[my as usize * 16 + mx as usize])
                                } else {
                                    None
                                }
                            })
                            .max()
                            .unwrap_or(0);
                        pixels[y * 64 + x] = egui::Color32::from_white_alpha(alpha);
                    }
                }
                ctx.load_texture(
                    format!("{} stroke preview", tip.label()),
                    egui::ColorImage::new([64, 20], pixels),
                    egui::TextureOptions::LINEAR,
                )
            })
            .collect();
        Self {
            settings: Default::default(),
            saved: None,
            undo: Vec::new(),
            redo: Vec::new(),
            open: false,
            kind: Kind::Brush,
            targets: HashMap::new(),
            thumbnails: HashMap::new(),
            strokes,
            previous_color: [0, 0, 0, 255],
            #[cfg(test)]
            targets_buttons: HashMap::new(),
        }
    }
    pub fn persist(&mut self, storage: Option<&UserStorage>) -> Result<(), String> {
        if self.saved != Some(self.settings) {
            if let Some(storage) = storage {
                self.settings.save(storage).map_err(|e| e.to_string())?;
            }
            if let Some(previous) = self.saved {
                self.undo.push(previous);
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.saved = Some(self.settings);
        }
        Ok(())
    }
    pub fn history(&mut self, redo: bool, storage: Option<&UserStorage>) -> Result<(), String> {
        let Some(next) = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .copied() else {
            return Ok(());
        };
        if let Some(storage) = storage {
            next.save(storage).map_err(|e| e.to_string())?;
        }
        if redo {
            self.redo.pop();
            self.undo.push(self.settings);
        } else {
            self.undo.pop();
            self.redo.push(self.settings);
        }
        self.settings = next;
        self.saved = Some(next);
        Ok(())
    }
    fn texture(
        &mut self,
        ctx: &egui::Context,
        document: &Document,
        target: Target,
        kind: Kind,
    ) -> Option<egui::TextureHandle> {
        let cfg = self.settings.panels[kind.index()];
        let size = cfg.thumbnail.size();
        if size == 0 {
            return None;
        }
        let key = ThumbnailKey {
            document: document.id,
            revision: document.revision,
            frame: document.current_frame,
            target,
            size,
            content: cfg.content,
            checker: cfg.checker,
        };
        if let Some(texture) = self.thumbnails.get(&key) {
            return Some(texture.clone());
        }
        let pixels =
            drawer_core::thumbnail::image(document, target, size, cfg.content, cfg.checker).ok()?;
        let image = egui::ColorImage::new(
            [size, size],
            pixels
                .into_iter()
                .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                .collect(),
        );
        if self.thumbnails.len() >= 512 {
            self.thumbnails.clear();
        }
        let texture = ctx.load_texture("panel-thumbnail", image, egui::TextureOptions::NEAREST);
        self.thumbnails.insert(key, texture.clone());
        Some(texture)
    }
}
impl Drawer {
    pub(super) fn tool_shortcuts_blocked(&self, ctx: &egui::Context) -> bool {
        self.path_transition.is_some()
            || self.vector_cleanup.draft.is_some()
            || self.affine_transform.draft.is_some()
            || self.tone_mapping.draft.is_some()
            || self.gaussian.draft.is_some()
            || self.surface_effects.draft.is_some()
            || self.spatial_blur.draft.is_some()
            || self.batch_filters.draft.is_some()
            || self.local_color.draft.is_some()
            || self.detail_filter.draft.is_some()
            || self.color_corrections.draft.is_some()
            || self.adjustments.draft.is_some()
            || self.deform.draft.is_some()
            || self.mesh_deform.draft.is_some()
            || self.pending_close.is_some()
            || self.sizing_dialog.is_some()
            || self.import_dialog.is_some()
            || self.paste_dialog.is_some()
            || self.movie_dialog.is_some()
            || self.revert_pending.is_some()
            || self.show_new_canvas
            || self.cursors.open
            || self.ui_behavior.open
            || self.tool_input.open
            || self.native_input.open
            || self.pressure.open
            || self.replay.open
            || self.acquisition.open
            || self.properties.open
            || self.brush_presets.open
            || self.textures.modal()
            || self.lines.modal()
            || self.ruler_tools.open
            || self.selection_filter.draft.is_some()
            || self.color_range.draft.is_some()
            || self.selection_border.draft.is_some()
            || self.gradient.draft.is_some()
            || self.alignment.draft.is_some()
            || self.layer_selection.draft.is_some()
            || self.controls.open
            || self.controls.radial.is_some()
            || self.workspaces.staged.is_some()
            || ctx.egui_wants_keyboard_input()
            || super::preferences::composing(ctx)
            || self.controls.quick.is_some()
            || egui::Popup::is_any_open(ctx)
    }
    pub(super) fn basic_shortcuts(&mut self, ctx: &egui::Context) {
        if self.tool_shortcuts_blocked(ctx) {
            return;
        }
        let action = ctx.input_mut(|input| {
            super::workspaces::shortcut(input, &self.workspaces.store.current.bindings)
        });
        if let Some((action, key, repeat)) = action {
            if let Some(tip) = BrushTip::ALL.iter().find(|t| t.name() == action) {
                self.press_tool(ctx, key, *tip, repeat);
            } else if !repeat
                || !matches!(
                    action.as_str(),
                    "primary-color"
                        | "secondary-color"
                        | "swap-colors"
                        | "transparent-color"
                        | "screen-eyedropper"
                        | "select-rectangle"
                        | "select-lasso"
                        | "select-ellipse"
                        | "select-polygon"
                        | "selection-clear"
                        | "selection-invert"
                        | "selection-reselect"
                        | "selection-save"
                        | "duplicate-page"
                        | "Move"
                        | "move-selection-boundary"
                        | "move-selected-pixels"
                        | "copy-selection"
                        | "paste-selection"
                        | "bucket-tool"
                        | "lasso-fill-tool"
                        | "enclose-fill-tool"
                        | "fill-small-gaps"
                        | "fill-selection"
                        | "ruler-snap"
                        | "quick-mask"
                        | "wand-tool"
                        | "selection-brush-tool"
                        | "selection-erase-tool"
                )
            {
                self.execute_command(ctx, &action);
            }
        }
    }
    pub(super) fn show_panel(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let cfg = self.panels.settings.panels[kind.index()];
        let shown = self.panel_auto_hide(ui, kind);
        ui.push_id(kind.index(), |ui| {
            if self.workspaces.store.current.layout.titles {
                ui.horizontal_wrapped(|ui| {
                    ui.heading(kind.label());
                    let placement = &self.workspaces.store.current.placements[kind.index()];
                    let lock = self.workspaces.store.current.layout.panel_locks[kind.index()]
                        .position
                        || match placement.dock {
                            drawer_core::workspaces::Dock::Left => {
                                self.workspaces.store.current.layout.dock_locked(0)
                            }
                            drawer_core::workspaces::Dock::Right => {
                                self.workspaces.store.current.layout.dock_locked(1)
                            }
                            _ => false,
                        };
                    if lock {
                        ui.label("Dock locked");
                        return;
                    }
                    ui.menu_button("Dock", |ui| {
                        for dock in drawer_core::workspaces::Dock::ALL {
                            if ui
                                .selectable_label(
                                    self.workspaces.store.current.placements[kind.index()].dock
                                        == dock,
                                    format!("{dock:?}"),
                                )
                                .clicked()
                            {
                                self.workspaces.store.current.placements[kind.index()].dock = dock;
                                ui.close();
                            }
                        }
                    });
                });
            }
            if !shown {
                let reveal = crate::help::button(ui, "Show panel contents", "tools");
                if reveal.clicked() || reveal.has_focus() {
                    self.ui_behavior.hidden[kind.index()].reveal();
                } else {
                    self.ui_behavior.rectangles[kind.index()] = Some(ui.min_rect());
                    return;
                }
            }
            if cfg.properties == Bar::Top {
                self.panel_properties(ui, kind);
            }
            if cfg.commands == Bar::Top {
                self.panel_commands(ui, kind);
            }
            match kind {
                Kind::Brush => {
                    for (n, tip) in BrushTip::ALL.into_iter().enumerate() {
                        ui.horizontal(|ui| {
                            let label = if cfg.names {
                                format!(
                                    "{}  {}",
                                    tip.label(),
                                    self.workspaces.command_label(tip.name())
                                )
                            } else {
                                String::new()
                            };
                            let response =
                                crate::help::selectable(ui, self.selected == tip, label, "tools");
                            response
                                .clone()
                                .on_hover_text(super::preferences::tool_hint(
                                    self.workspaces.store.current.preferences.language,
                                    tip,
                                ));
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    ui.is_enabled(),
                                    self.selected == tip,
                                    tip.label(),
                                )
                            });
                            let response = response.on_hover_text(format!(
                                "{} ({})",
                                tip.label(),
                                self.workspaces.command_label(tip.name())
                            ));
                            let mut clicked = response.clicked();
                            if cfg.icons {
                                clicked |= ui
                                    .add(
                                        egui::Image::new(&self.previews[n])
                                            .fit_to_exact_size(egui::vec2(16.0, 16.0))
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(tip.label())
                                    .clicked();
                            }
                            if cfg.strokes {
                                clicked |= ui
                                    .add(
                                        egui::Image::new(&self.panels.strokes[n])
                                            .fit_to_exact_size(egui::vec2(64.0, 20.0))
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(tip.label())
                                    .clicked();
                            }
                            if clicked {
                                self.set_tool(tip);
                            }
                        });
                    }
                }
                Kind::Color => self.color_controls(ui),
                Kind::Layers | Kind::Timeline => self.panel_layers(ui, kind),
            }
            if cfg.properties == Bar::Bottom {
                self.panel_properties(ui, kind);
            }
            if cfg.commands == Bar::Bottom {
                self.panel_commands(ui, kind);
            }
            ui.separator();
            self.ui_behavior.rectangles[kind.index()] = Some(ui.min_rect());
        });
    }
    fn panel_layers(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let Some(n) = self.active else {
            ui.label("No document open");
            return;
        };
        let doc = &self.documents[n];
        let id = doc.id;
        let selected = *self.panels.targets.entry(id).or_insert(Target::Raster(0));
        let cfg = self.panels.settings.panels[kind.index()];
        let mut rows = Vec::new();
        if kind != Kind::Timeline || !cfg.animation_only {
            for index in drawer_core::layers::order(doc) {
                rows.push((Target::Raster(index), doc.layers[index].name.clone()));
            }
        }
        if let Some(animation) = &doc.animation {
            for (index, track) in animation.tracks.iter().enumerate() {
                rows.push((
                    Target::Track(index),
                    format!("{} · {} cells", track.name, track.cells.len()),
                ));
            }
        }
        if kind != Kind::Timeline {
            if ui.button("Search / select layers").clicked() {
                self.layer_search.open = true;
            }
            if let Ok(sources) = drawer_core::layer_catalog::rows(doc) {
                for row in sources.into_iter().filter(|r| {
                    matches!(
                        r.key,
                        drawer_core::layer_search::Key::Object(_)
                            | drawer_core::layer_search::Key::Gradient(_)
                            | drawer_core::layer_search::Key::World(_)
                            | drawer_core::layer_search::Key::Selection(_)
                    )
                }) {
                    rows.push((Target::Source(row.key), row.name));
                }
            }
            for layer in &doc.scene.lines.layers {
                rows.push((Target::Vector(layer.id), format!("Vector: {}", layer.name)));
            }
            for (n, folder) in doc
                .scene
                .paper
                .folders
                .iter()
                .enumerate()
                .filter(|(n, _)| doc.scene.paper.folder_active(*n))
            {
                rows.push((Target::Folder(n), format!("Folder: {}", folder.name)));
            }
        }
        if rows.is_empty() {
            ui.label("No animation tracks");
        }
        let row_height = cfg.thumbnail.size().max(22) as f32;
        egui::ScrollArea::vertical()
            .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
            .id_salt("panel-rows")
            .max_height(140.0)
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for index in range {
                    let (target, label) = &rows[index];
                    ui.horizontal(|ui| {
                        let texture =
                            self.panels
                                .texture(ui.ctx(), &self.documents[n], *target, kind);
                        let image_clicked = texture.is_some_and(|texture| {
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(egui::vec2(
                                        cfg.thumbnail.size() as f32,
                                        cfg.thumbnail.size() as f32,
                                    ))
                                    .sense(egui::Sense::click()),
                            )
                            .clicked()
                        });
                        super::layer_properties::row_mark(
                            ui,
                            &self.documents[n],
                            *target,
                            self.workspaces.store.current.layer_colors,
                            kind == Kind::Timeline,
                        );
                        let response =
                            crate::help::selectable(ui, selected == *target, label, "tools");
                        #[cfg(test)]
                        {
                            self.panels.targets_buttons.insert(*target, response.rect);
                        }
                        if response.clicked() || image_clicked {
                            self.panels.targets.insert(id, *target);
                        }
                    });
                }
            });
        if !rows.iter().any(|(target, _)| *target == selected) {
            ui.label("Selection is outside this list.");
        }
        if kind != Kind::Timeline
            && ui.button("Reference / draft roles").clicked()
            && let Err(e) = self.begin_layer_roles()
        {
            self.status = Some(e);
        }
        if ui.button("Linked layer groups").clicked()
            && let Err(e) = self.begin_layer_groups()
        {
            self.status = Some(e);
        }
        if ui.button("Transfer content down").clicked()
            && let Err(e) = self.begin_transfer_down()
        {
            self.status = Some(e);
        }
        if ui.button("Layer mask operations").clicked()
            && let Err(e) = self.begin_layer_masks()
        {
            self.status = Some(e);
        }
        if ui.button("Layer comps").clicked()
            && let Err(e) = self.begin_layer_comps()
        {
            self.status = Some(e);
        }
        if kind != Kind::Timeline {
            if ui.button("Rasterize / content color").clicked()
                && let Err(e) = self.begin_raster_conversion()
            {
                self.status = Some(e);
            }
            if ui.button("Local linked file layer").clicked()
                && let Err(e) = self.begin_file_objects()
            {
                self.status = Some(e);
            }
            for (label, edit) in [("New material layer", false), ("Edit material layer", true)] {
                if ui.button(label).clicked()
                    && let Err(e) = self.begin_material_layer(edit)
                {
                    self.status = Some(e);
                }
            }
            if ui.button("Display effects").clicked()
                && let Err(e) = self.begin_appearance()
            {
                self.status = Some(e);
            }
            if ui.button("New layer defaults").clicked()
                && let Err(e) = self.begin_layer_defaults()
            {
                self.status = Some(e);
            }
        }
    }
    fn panel_properties(&mut self, ui: &mut egui::Ui, kind: Kind) {
        match kind {
            Kind::Brush => {
                ui.collapsing("Keyboard editing", |ui| self.precision_controls(ui));
                ui.add_enabled_ui(self.drawing.mode == super::precision::Mode::Paint, |ui| {
                    self.tool_property_controls(ui)
                });
            }
            Kind::Color => {
                let [r, g, b, a] = self.references.session.workspace.color;
                ui.label(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"));
            }
            Kind::Layers | Kind::Timeline => {
                if let Some(n) = self.active {
                    let doc = &self.documents[n];
                    let target = self
                        .panels
                        .targets
                        .get(&doc.id)
                        .copied()
                        .unwrap_or(Target::Raster(0));
                    let label = match target {
                        Target::Raster(n) => doc.layers.get(n).map(|layer| {
                            let e = layer.extent(doc.width, doc.height);
                            format!(
                                "{} × {} · {}, {}",
                                e.width, e.height, e.offset[0], e.offset[1]
                            )
                        }),
                        Target::Track(n) => doc
                            .animation
                            .as_ref()
                            .and_then(|a| a.tracks.get(n))
                            .map(|track| {
                                format!("{} · {} exposures", track.name, track.exposures.len())
                            }),
                        Target::Folder(_) | Target::Vector(_) | Target::Source(_) => {
                            Some(drawer_core::layer_pick::label(doc, target))
                        }
                    };
                    ui.label(label.unwrap_or_else(|| "Selected item unavailable".into()));
                    if let Target::Raster(layer) = target
                        && let Some(raster) = self.documents[n].layers.get(layer)
                    {
                        let mut locked = raster.locked;
                        if ui.checkbox(&mut locked, "Lock raster layer").changed() {
                            self.finish_stroke();
                            let result = drawer_core::layer_properties::prepare_patch(
                                &self.documents[n],
                                &[drawer_core::layer_properties::raster(
                                    &self.documents[n],
                                    layer,
                                )],
                                drawer_core::layer_properties::Patch {
                                    locked: Some(locked),
                                    ..Default::default()
                                },
                            )
                            .and_then(|state| self.documents[n].commit_state(state));
                            if let Err(e) = result {
                                self.status = Some(e);
                            }
                        }
                    }
                }
            }
        }
    }
    fn panel_commands(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let mut layer_action = None;
        ui.horizontal_wrapped(|ui| match kind {
            Kind::Brush => {
                for tip in BrushTip::ALL {
                    if ui
                        .small_button(format!(
                            "{} {}",
                            tip.label(),
                            self.workspaces.command_label(tip.name())
                        ))
                        .on_hover_text(tip.label())
                        .clicked()
                    {
                        self.set_tool(tip);
                    }
                }
            }
            Kind::Color => {
                if ui.small_button("Previous color").clicked() {
                    self.choose_color(self.panels.previous_color);
                }
                if ui.small_button("Black").clicked() {
                    self.choose_color([0, 0, 0, 255]);
                }
            }
            Kind::Layers | Kind::Timeline => {
                if let Some(n) = self.active {
                    let doc = &mut self.documents[n];
                    let mut top = false;
                    if kind == Kind::Layers {
                        ui.menu_button("Layers…", |ui| {
                            let height = (ui.ctx().viewport_rect().height() * 0.65).max(100.);
                            egui::ScrollArea::vertical()
                                .id_salt("layer-command-list")
                                .max_height(height)
                                .show(ui, |ui| {
                                    top = ui.button("Top raster").clicked();
                                    for (label, command) in [
                                        ("Manage layers…", "manage-layers"),
                                        ("Properties / identification colors…", "layer-properties"),
                                        ("Alpha protection / clipping / solo…", "layer-clipping"),
                                        ("Layer blend modes…", "layer-blend"),
                                        ("Merge layers / flatten…", "merge-layers"),
                                        ("Image tiling / source transform…", "material-transform"),
                                        ("Refresh linked sources…", "refresh-file-layers"),
                                        ("Copy visible to new layer…", "merge-visible-copy"),
                                        ("Delete empty layers…", "delete-empty-layers"),
                                        ("Delete hidden layers…", "delete-hidden-layers"),
                                        ("Folders / grayscale masks…", "layer-folders"),
                                        ("Color correction layers…", "adjustment-layer"),
                                        ("Local reference colors…", "local-reference-color"),
                                        ("Local light / shadow colors…", "local-light-shadow"),
                                        ("Levels…", "color-levels"),
                                        ("Color balance…", "color-balance"),
                                        ("Posterize…", "color-posterize"),
                                        ("Threshold…", "color-threshold"),
                                        ("Invert layer pixels…", "invert-pixels"),
                                        ("Inversion correction layer…", "invert-correction-layer"),
                                        ("Import local image layer…", "import-image-layer"),
                                        ("New raster", "new-raster-layer"),
                                        ("New vector", "new-vector-layer"),
                                        ("Draw vector line", "vector-line-tool"),
                                        ("Draw vector curve", "vector-curve-tool"),
                                        ("Edit vector points", "vector-point-tool"),
                                        ("Edit vector pressure", "vector-pressure-tool"),
                                        ("Recolor vector strokes", "vector-color-tool"),
                                        ("Vector numeric editor…", "vector-edit-tools"),
                                        ("Refine vector strokes…", "vector-refine-tools"),
                                        ("Trace raster to vectors…", "raster-trace"),
                                        ("Scale and rotate…", "scale-rotate"),
                                        ("RGB and channel tone curves…", "tone-curves"),
                                        ("Luminance gradient map…", "gradient-map"),
                                        ("Mesh and puppet deformation…", "mesh-puppet"),
                                        ("Free / perspective / liquify…", "raster-deformation"),
                                        ("Gaussian blur…", "gaussian-blur"),
                                        ("Multiple-layer filters…", "batch-filters"),
                                        ("Fisheye…", "fisheye-filter"),
                                        ("Panorama…", "panorama-filter"),
                                        ("Wave…", "wave-filter"),
                                        ("Swirl…", "swirl-filter"),
                                        ("Drop shadow…", "drop-shadow"),
                                        ("Glow…", "glow-effect"),
                                        ("Emboss / raised surface…", "emboss-effect"),
                                        ("Crystallize…", "crystallize-effect"),
                                        ("Edit surface effects…", "edit-surface-effects"),
                                        ("Chromatic aberration…", "chromatic-aberration"),
                                        ("Film effect…", "film-effect"),
                                        ("Pencil effect…", "pencil-effect"),
                                        ("Mosaic…", "mosaic"),
                                        ("Generate noise…", "generate-noise"),
                                        ("Remove specks…", "remove-specks"),
                                        ("Adjust line width…", "adjust-line-width"),
                                        ("Sharpen…", "sharpen"),
                                        ("Unsharp mask…", "unsharp-mask"),
                                        ("Reduce noise…", "reduce-noise"),
                                        ("Motion blur…", "motion-blur"),
                                        ("Radial blur…", "radial-blur"),
                                        ("Spin blur…", "spin-blur"),
                                        ("Lens blur…", "lens-blur"),
                                        (
                                            "Remove short vector strokes/spans…",
                                            "vector-remove-short",
                                        ),
                                        ("Change vector tip…", "vector-tip-tools"),
                                        (
                                            "Vector selection, pivot and transfer…",
                                            "vector-scope-tools",
                                        ),
                                        (
                                            "Select overlapping vector strokes",
                                            "vector-select-overlap",
                                        ),
                                        (
                                            "Select contained vector strokes",
                                            "vector-select-contained",
                                        ),
                                        (
                                            "Reset selected vector pressure…",
                                            "vector-pressure-reset-selected",
                                        ),
                                        (
                                            "Reset vector layer pressure…",
                                            "vector-pressure-reset-layer",
                                        ),
                                        ("Import SVG…", "svg-import"),
                                        ("Export SVG…", "svg-export"),
                                        ("Copy SVG text", "svg-copy"),
                                        ("Show/hide vector paths", "vector-path-overlay"),
                                        ("Push vector span", "vector-push-tool"),
                                        ("Redraw vector span", "vector-redraw-tool"),
                                        ("Select vector strokes", "vector-stroke-tool"),
                                        ("Delete selected strokes", "vector-delete-strokes"),
                                        ("Vector eraser…", "vector-eraser-tool"),
                                        ("Add vector node", "vector-node-add-tool"),
                                        ("Delete vector node", "vector-node-delete-tool"),
                                        ("Corner vector node", "vector-node-corner-tool"),
                                        ("Smooth vector node", "vector-node-smooth-tool"),
                                        ("Duplicate selected layer", "duplicate-layer"),
                                        ("Delete selected layer", "delete-layer"),
                                    ] {
                                        if ui.button(label).clicked() {
                                            layer_action = Some(command);
                                            ui.close();
                                        }
                                    }
                                });
                        });
                    } else {
                        top = ui.small_button("Top raster").clicked();
                    }
                    if top {
                        self.panels.targets.insert(
                            doc.id,
                            Target::Raster(*drawer_core::layers::order(doc).last().unwrap()),
                        );
                    }
                    if ui.small_button("Center selection").clicked() {
                        let target = self
                            .panels
                            .targets
                            .get(&doc.id)
                            .copied()
                            .unwrap_or(Target::Raster(0));
                        let bounds = drawer_core::thumbnail::bounds(doc, target, true);
                        doc.view.center_on(
                            [
                                (bounds.min[0] + bounds.max[0]) / 2.0,
                                (bounds.min[1] + bounds.max[1]) / 2.0,
                            ],
                            [doc.width as f64, doc.height as f64],
                            [0.0; 2],
                            ui.ctx().pixels_per_point() as f64,
                        );
                    }
                }
            }
        });
        if let Some(command) = layer_action {
            self.execute_command(ui.ctx(), command);
        }
    }
    pub(super) fn panel_settings(&mut self, ctx: &egui::Context) {
        if !self.panels.open {
            return;
        }
        let mut open = true;
        let mut history = None;
        egui::Window::new("Panels and shortcuts")
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ComboBox::from_id_salt("panel-kind")
                    .selected_text(self.panels.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in Kind::ALL {
                            ui.selectable_value(&mut self.panels.kind, kind, kind.label());
                        }
                    });
                let cfg = &mut self.panels.settings.panels[self.panels.kind.index()];
                crate::help::checkbox(ui, &mut cfg.visible, "Show panel", "tools");
                if self.panels.kind == Kind::Brush {
                    crate::help::checkbox(ui, &mut cfg.icons, "Tool icons", "tools");
                    crate::help::checkbox(ui, &mut cfg.names, "Tool names", "tools");
                    crate::help::checkbox(ui, &mut cfg.strokes, "Stroke previews", "tools");
                }
                if matches!(self.panels.kind, Kind::Layers | Kind::Timeline) {
                    egui::ComboBox::from_id_salt("panel-thumb")
                        .selected_text(cfg.thumbnail.label())
                        .show_ui(ui, |ui| {
                            for size in Thumbnail::ALL {
                                ui.selectable_value(&mut cfg.thumbnail, size, size.label());
                            }
                        });
                    crate::help::checkbox(
                        ui,
                        &mut cfg.content,
                        "Show content bounds instead of whole canvas",
                        "tools",
                    );
                    crate::help::checkbox(ui, &mut cfg.checker, "Transparency checker", "tools");
                    if self.panels.kind == Kind::Timeline {
                        crate::help::checkbox(
                            ui,
                            &mut cfg.animation_only,
                            "Animation tracks only",
                            "tools",
                        );
                    }
                }
                for (label, bar) in [
                    ("Properties bar", &mut cfg.properties),
                    ("Commands bar", &mut cfg.commands),
                ] {
                    egui::ComboBox::from_id_salt(label)
                        .selected_text(format!("{label}: {}", bar.label()))
                        .show_ui(ui, |ui| {
                            for value in Bar::ALL {
                                ui.selectable_value(bar, value, value.label());
                            }
                        });
                }
                ui.separator();
                ui.label(
                    self.workspaces
                        .store
                        .current
                        .bindings
                        .iter()
                        .map(|b| format!("{} {}", b.command, b.label()))
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
                if crate::help::button(ui, "Restore all panels", "tools").clicked() {
                    self.panels.settings = Settings::default();
                }
                if ui
                    .add_enabled(
                        !self.panels.undo.is_empty(),
                        egui::Button::new("Undo panel setting"),
                    )
                    .clicked()
                {
                    history = Some(false);
                }
                if ui
                    .add_enabled(
                        !self.panels.redo.is_empty(),
                        egui::Button::new("Redo panel setting"),
                    )
                    .clicked()
                {
                    history = Some(true);
                }
                if ui
                    .add_enabled(
                        self.storage.is_some(),
                        egui::Button::new("Restore previous panels"),
                    )
                    .clicked()
                {
                    match Settings::read(
                        &self
                            .storage
                            .as_ref()
                            .unwrap()
                            .root()
                            .join("settings/panels.previous.json"),
                    ) {
                        Ok(s) => self.panels.settings = s,
                        Err(e) => self.status = Some(e.to_string()),
                    }
                }
            });
        self.panels.open = open;
        if let Some(redo) = history
            && let Err(e) = self.panels.history(redo, self.storage.as_ref())
        {
            self.status = Some(e);
        }
    }
}
#[cfg(test)]
pub(super) fn shortcut_action(input: &mut egui::InputState) -> Option<u8> {
    super::workspaces::shortcut(input, &drawer_core::workspaces::default_bindings()).and_then(
        |(c, _, _)| {
            drawer_core::workspaces::COMMANDS
                .iter()
                .position(|a| *a == c)
                .map(|n| n as u8)
        },
    )
}
