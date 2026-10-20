use super::*;
use drawer_core::{
    actions::{self, AccessSet, Cycle, Display, Item, Radial, SetKind},
    workspaces::{Binding, Scope, Snapshot},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
enum Target {
    Binding(usize),
    Item(u64, usize),
    Slot(u64, usize),
    Device(usize),
}
struct Picker {
    target: Target,
    selected: String,
}
pub(super) fn smoke_controller() -> Result<(), String> {
    use drawer_core::controller::{DEVICE, Event, Kind};
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let frame = |app: &mut Drawer, events: Vec<Event>| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.controller_actions(ui.ctx(), &events);
                app.show(ui);
            },
        );
        output.textures_delta.clear();
    };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let before = app.documents[0].snapshot();
    let dirty = app.documents[0].dirty;
    app.workspaces.store.current.actions.devices = vec![
        actions::DeviceBinding {
            device: DEVICE.into(),
            action: "clockwise".into(),
            command: "brush-grow".into(),
        },
        actions::DeviceBinding {
            device: DEVICE.into(),
            action: "counterclockwise".into(),
            command: "zoom-out".into(),
        },
        actions::DeviceBinding {
            device: DEVICE.into(),
            action: "click".into(),
            command: "cycle-next:1".into(),
        },
    ];
    let initial_size = app.panels.settings.brush_size;
    let event = |serial, kind| Event { serial, kind };
    frame(
        &mut app,
        vec![event(1, Kind::Acquired), event(2, Kind::Rotation(15.0))],
    );
    if (app.panels.settings.brush_size - initial_size * 1.1).abs() > 1e-9 {
        return Err("Dial did not adjust the live tool property".into());
    }
    frame(&mut app, vec![event(3, Kind::Rotation(-15.0))]);
    if (app.documents[0].view.zoom - 1.0 / 1.1).abs() > 1e-9 {
        return Err("Dial zoom command did not reach the view".into());
    }
    frame(&mut app, vec![event(4, Kind::Click), event(4, Kind::Click)]);
    if app.selected != BrushTip::Pencil {
        return Err("Dial click duplicated or tool switch failed".into());
    }
    frame(&mut app, vec![event(5, Kind::Lost), event(6, Kind::Click)]);
    if app.selected != BrushTip::Pencil {
        return Err("Lost controller executed an action".into());
    }
    frame(
        &mut app,
        vec![event(7, Kind::Acquired), event(8, Kind::Rotation(14.0))],
    );
    app.controls.open = true;
    frame(
        &mut app,
        vec![event(9, Kind::Rotation(15.0)), event(10, Kind::Click)],
    );
    app.controls.open = false;
    app.controls.editor = None;
    let size = app.panels.settings.brush_size;
    frame(&mut app, vec![event(11, Kind::Rotation(1.0))]);
    if size != app.panels.settings.brush_size || app.selected != BrushTip::Pencil {
        return Err("Modal input leaked or retained a dial fraction".into());
    }
    if app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
        || app.documents[0].dirty != dirty
    {
        return Err("Dial view/tool controls changed drawing pixels or history".into());
    }
    Ok(())
}
#[derive(Default)]
struct Editor {
    draft: Snapshot,
    page: u8,
    selected: usize,
    picker: Option<Picker>,
    #[cfg(test)]
    picker_confirm: Option<egui::Rect>,
}
#[derive(Default)]
pub(super) struct Controls {
    pub open: bool,
    editor: Option<Editor>,
    pub quick: Option<u64>,
    pub radial: Option<(u64, egui::Pos2)>,
    just_opened: bool,
    query: HashMap<u64, String>,
    drag: Option<(u64, usize)>,
    pointer_down: HashSet<usize>,
    devices: actions::DeviceEvents,
    token: u64,
    controller: drawer_core::controller::Controller,
    controller_assignments: Vec<actions::DeviceBinding>,
    #[cfg(test)]
    pub radial_center: Option<egui::Pos2>,
    #[cfg(test)]
    pub item_rects: HashMap<(u64, usize), egui::Rect>,
    #[cfg(test)]
    pub item_ids: HashMap<(u64, usize), egui::Id>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn command_reason(&self, command: &str) -> Option<String> {
        if (command.starts_with("opacity-")
            || ["brush-grow", "brush-shrink", "connect-line"].contains(&command))
            && self.drawing.mode != super::precision::Mode::Paint
        {
            return Some(
                "This tool does not support brush size, opacity or connecting strokes".into(),
            );
        }
        if let Some(id) = drawer_core::user_tools::command_id(command) {
            return if self.workspaces.store.current.user_tools.entry(id).is_some() {
                None
            } else {
                Some(format!("User tool unavailable: {command}"))
            };
        }
        if (["brush-grow", "brush-shrink"].contains(&command)
            && !self.user_tool_supports(drawer_core::tool_properties::Property::Size))
            || (command.starts_with("opacity-")
                && !self.user_tool_supports(drawer_core::tool_properties::Property::Opacity))
        {
            return Some("This user tool has a fixed value for that property".into());
        }
        if !self.workspaces.store.current.actions.has_command(command) {
            return Some(format!("Command unavailable: {command}"));
        }
        match command {
            "delete-outside-selection" => {
                let Some(n) = self.active else {
                    return Some("Open a page first".into());
                };
                let d = &self.documents[n];
                let layer = match self
                    .panels
                    .targets
                    .get(&d.id)
                    .copied()
                    .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
                {
                    drawer_core::thumbnail::Target::Raster(i) => i,
                    _ => return Some("Choose a raster target".into()),
                };
                if drawer_core::magnetic_lasso::can_delete_outside(d, layer) {
                    None
                } else {
                    Some("A nonempty selection and unlocked raster target are required".into())
                }
            }
            "save" | "save-as" | "view-center" | "view-reset" | "zoom-in" | "zoom-out"
                if self.active.is_none() =>
            {
                Some("Open a document first".into())
            }
            "undo"
                if self
                    .active
                    .is_none_or(|n| self.documents[n].history_counts().0 == 0) =>
            {
                Some("No drawing command to undo".into())
            }
            "redo"
                if self
                    .active
                    .is_none_or(|n| self.documents[n].history_counts().1 == 0) =>
            {
                Some("No drawing command to redo".into())
            }
            _ => None,
        }
    }
    pub(super) fn execute_command(&mut self, ctx: &egui::Context, command: &str) {
        if command == "selection-outline" {
            self.toggle_selection_outline();
            return;
        }
        let preserve_prepared_contact = matches!(
            command,
            "primary-color"
                | "secondary-color"
                | "swap-colors"
                | "transparent-color"
                | "select-rectangle"
                | "select-lasso"
                | "select-ellipse"
                | "select-polygon"
                | "Move"
                | "move-selection-boundary"
                | "move-selected-pixels"
                | "bucket-tool"
                | "lasso-fill-tool"
                | "enclose-fill-tool"
                | "ruler-snap"
                | "quick-mask"
                | "wand-tool"
                | "selection-brush-tool"
                | "selection-erase-tool"
        );
        if !BrushTip::ALL.iter().any(|t| t.name() == command)
            && !command.starts_with("cycle-")
            && (!preserve_prepared_contact || self.drawing.pending())
        {
            self.finish_stroke();
        }
        if let Some(reason) = self.command_reason(command) {
            self.status = Some(reason);
            return;
        }
        let result = (|| -> Result<(), String> {
            match command {
                "primary-color" | "secondary-color" | "swap-colors" | "transparent-color" => {
                    self.color_mode_command(command)?
                }
                "wand-tool" | "selection-brush-tool" | "selection-erase-tool" => {
                    self.restore_locked_tool();
                    self.drawing.mode = if command == "wand-tool" {
                        super::precision::Mode::Wand
                    } else {
                        self.workspaces.store.current.selection_edit.erase =
                            command == "selection-erase-tool";
                        super::precision::Mode::SelectionBrush
                    };
                    self.drawing.selection_settings = self.workspaces.store.current.selection_edit;
                    self.drawing.connect_line = false;
                }
                "ruler-tools" => self.begin_rulers()?,
                "ruler-snap" => {
                    self.drawing.ruler_disabled = !self.drawing.ruler_disabled;
                    self.workspaces.store.current.snapping.rulers = !self.drawing.ruler_disabled;
                    self.drawing.snap_settings.rulers = !self.drawing.ruler_disabled;
                    self.status = Some(format!(
                        "Ruler snapping {} for following strokes",
                        if self.drawing.ruler_disabled {
                            "off"
                        } else {
                            "on"
                        }
                    ));
                }
                "quick-mask" => self.toggle_quick_mask(),
                "color-range" => self.begin_color_range()?,
                "selection-border" => self.begin_selection_border()?,
                "gradient-tool" => self.begin_gradient()?,
                "align-objects" => self.begin_alignment()?,
                "manage-layers" => self.begin_layer_manager()?,
                "merge-layers" => self.begin_layer_merge()?,
                "merge-visible-copy" => self.begin_layer_maintenance(
                    drawer_core::layer_maintenance::Operation::VisibleCopy,
                )?,
                "delete-empty-layers" => {
                    self.begin_layer_maintenance(drawer_core::layer_maintenance::Operation::Empty)?
                }
                "delete-hidden-layers" => {
                    self.begin_layer_maintenance(drawer_core::layer_maintenance::Operation::Hidden)?
                }
                "layer-folders" => self.begin_layer_folders()?,
                "adjustment-layer" => self.begin_adjustment()?,
                "layer-search" => self.layer_search.open = true,
                "layer-roles" => self.begin_layer_roles()?,
                "layer-groups" => self.begin_layer_groups()?,
                "paper-background" => self.begin_paper_background()?,
                "document-color" => self.begin_document_color()?,
                "transfer-down" => self.begin_transfer_down()?,
                "layer-masks" => self.begin_layer_masks()?,
                "layer-comps" => self.begin_layer_comps()?,
                "raster-conversion" => self.begin_raster_conversion()?,
                "linked-file-layer" => self.begin_file_objects()?,
                "material-layer-new" => self.begin_material_layer(false)?,
                "material-layer-edit" => self.begin_material_layer(true)?,
                "layer-appearance" => self.begin_appearance()?,
                "layer-defaults" => self.begin_layer_defaults()?,
                "layer-properties" | "layer-clipping" | "layer-blend" => {
                    self.begin_layer_properties()?
                }
                "import-image-layer" => self.choose_image_layer(ctx)?,
                "new-raster-layer" | "new-vector-layer" | "duplicate-layer" | "delete-layer" => {
                    self.layer_quick_command(command)?
                }
                "layer-selection-tools" => self.begin_layer_selection()?,
                "vector-selection-clear" => self.clear_vector_selection()?,
                "alpha-selection-replace" => {
                    self.alpha_selection_command(drawer_core::selection_edit::Combine::Replace)?
                }
                "alpha-selection-add" => {
                    self.alpha_selection_command(drawer_core::selection_edit::Combine::Add)?
                }
                "alpha-selection-subtract" => {
                    self.alpha_selection_command(drawer_core::selection_edit::Combine::Subtract)?
                }
                "alpha-selection-intersect" => {
                    self.alpha_selection_command(drawer_core::selection_edit::Combine::Intersect)?
                }
                "shape-tool" => {
                    self.restore_locked_tool();
                    self.drawing.mode = if self.workspaces.store.current.shapes.input
                        == drawer_core::shapes::Input::ByClick
                    {
                        super::precision::Mode::ShapeClick
                    } else {
                        super::precision::Mode::ShapeDrag
                    };
                    self.drawing.connect_line = false;
                }
                "bucket-tool" => {
                    self.restore_locked_tool();
                    self.drawing.mode = super::precision::Mode::Fill;
                    self.drawing.connect_line = false;
                }
                "fill-selection" => self.fill_selection()?,
                "lasso-fill-tool" | "enclose-fill-tool" => {
                    self.restore_locked_tool();
                    self.drawing.mode = if command == "lasso-fill-tool" {
                        super::precision::Mode::LassoFill
                    } else {
                        super::precision::Mode::EncloseFill
                    };
                    self.drawing.connect_line = false;
                }
                "fill-small-gaps" => self.fill_small_gaps()?,
                "pick-next-layer" => {
                    let n = self.active.ok_or("Open a page first")?;
                    let doc = &self.documents[n];
                    let (id, p) = self
                        .drawing
                        .pick_point
                        .ok_or("Use the layer-pick modifier and click the canvas first")?;
                    if id != doc.id {
                        return Err("Pick a point on this page first".into());
                    }
                    let candidates = drawer_core::layer_pick::candidates(
                        doc,
                        p,
                        self.workspaces.store.current.layer_pick,
                    );
                    if !candidates.is_empty() {
                        let selected = self.panels.targets.get(&id);
                        let next = candidates
                            .iter()
                            .position(|t| Some(t) == selected)
                            .map_or(0, |i| (i + 1) % candidates.len());
                        self.panels.targets.insert(id, candidates[next]);
                    }
                }
                "copy-selection" => {
                    self.copy_selection_from(super::paste_dialog::write_clipboard)?
                }
                "paste-selection" => self.begin_paste(ctx, super::paste_dialog::read_clipboard)?,
                "move-selection-boundary" | "move-selected-pixels" => {
                    self.restore_locked_tool();
                    self.precision.target = if command == "move-selection-boundary" {
                        drawer_core::precision::Target::Boundary
                    } else {
                        drawer_core::precision::Target::Pixels
                    };
                    self.drawing.move_target = self.precision.target;
                    self.drawing.mode = super::precision::Mode::Move;
                    self.drawing.connect_line = false;
                }
                "contour-selection" => self.begin_contour_selection()?,
                "animation-output-frame" => self.begin_animation_output()?,
                "special-ruler-tools" => self.begin_special_rulers()?,
                "ruler-follow" => self.begin_follow_ruler()?,
                "ruler-move-layer" => self.begin_ruler_layer_move()?,
                "selection-shortcuts" => self.edit_selection_shortcuts(),
                "selection-expand" | "selection-shrink" | "selection-feather" => self
                    .begin_selection_filter(match command {
                        "selection-expand" => drawer_core::selection_filter::Operation::Expand,
                        "selection-shrink" => drawer_core::selection_filter::Operation::Shrink,
                        _ => drawer_core::selection_filter::Operation::Feather,
                    })?,
                "screen-eyedropper" => self.begin_screen_sample(ctx)?,
                "select-rectangle"
                | "select-lasso"
                | "select-ellipse"
                | "select-polygon"
                | "selection-clear"
                | "selection-invert"
                | "selection-reselect"
                | "selection-tools"
                | "selection-save"
                | "duplicate-page"
                | "delete-outside-selection" => self.selection_command(command)?,
                "brush-grow" | "brush-shrink" => {
                    let step = (self.panels.settings.brush_size * 0.1).max(0.1);
                    self.panels.settings.brush_size = (self.panels.settings.brush_size
                        + if command == "brush-grow" { step } else { -step })
                    .clamp(0.1, 4096.0);
                }
                "Move" | "Eyedropper" => {
                    self.restore_locked_tool();
                    self.drawing.mode = if command == "Move" {
                        super::precision::Mode::Move
                    } else {
                        super::precision::Mode::Eyedropper
                    };
                    self.drawing.connect_line = false;
                }
                "connect-line" => {
                    self.drawing.connect_line = !self.drawing.connect_line;
                    self.status=Some("Click a canvas point to connect from this tool's last stroke; Escape cancels".into());
                }
                c if c.starts_with("opacity-") => {
                    let n = BrushTip::ALL
                        .iter()
                        .position(|tip| *tip == self.selected)
                        .unwrap();
                    let opacity = &mut self.panels.settings.brush_opacity[n];
                    *opacity = match c {
                        "opacity-up" => (*opacity + 0.1).min(1.0),
                        "opacity-down" => (*opacity - 0.1).max(0.0),
                        _ => {
                            c[8..]
                                .parse::<f64>()
                                .map_err(|_| "Invalid opacity command")?
                                / 100.0
                        }
                    };
                }
                "zoom-in" | "zoom-out" => {
                    if let Some(n) = self.active {
                        self.documents[n].view.zoom = (self.documents[n].view.zoom
                            * if command == "zoom-in" { 1.1 } else { 1.0 / 1.1 })
                        .clamp(0.01, 64.0);
                    }
                }
                "save" => {
                    if let Some(n) = self.active {
                        self.save_document(n)?;
                    }
                }
                "save-as" => {
                    if let Some(n) = self.active {
                        self.save_as(n)?;
                    }
                }
                "undo" => {
                    if let Some(n) = self.active {
                        self.documents[n].undo();
                    }
                }
                "redo" => {
                    if let Some(n) = self.active {
                        self.documents[n].redo();
                    }
                }
                "view-center" => {
                    if let Some(n) = self.active {
                        self.documents[n].view.pan = [0.0; 2];
                    }
                }
                "view-reset" => {
                    if let Some(n) = self.active {
                        self.documents[n].view = Default::default();
                    }
                }
                "new-canvas" => {
                    self.new_canvas = Default::default();
                    self.show_new_canvas = true;
                }
                "help" => self.open_help("welcome"),
                "screen-mode" => self.toggle_screen_mode(),
                "test-painting" => self.workspaces.store.current.scratchpad.visible = true,
                _ => {
                    if let Some(tip) = BrushTip::ALL.iter().find(|t| t.name() == command) {
                        self.set_tool(*tip);
                    } else if let Some((prefix, id)) = command.split_once(':') {
                        let id: u64 = id.parse().map_err(|_| "Invalid command reference")?;
                        match prefix {
                            "local-tool" => self.select_user_tool(id)?,
                            "cycle-next" | "cycle-prev" => {
                                let c = self
                                    .workspaces
                                    .store
                                    .current
                                    .actions
                                    .cycles
                                    .iter()
                                    .find(|c| c.id == id)
                                    .ok_or("Tool cycle unavailable")?;
                                let current = self
                                    .workspaces
                                    .store
                                    .current
                                    .user_tools
                                    .active
                                    .map(drawer_core::user_tools::command)
                                    .unwrap_or_else(|| self.selected.name().into());
                                let next = self.workspaces.store.current.user_tools.cycle(
                                    c,
                                    &current,
                                    prefix == "cycle-prev",
                                )?;
                                if let Some(id) = drawer_core::user_tools::command_id(&next) {
                                    self.select_user_tool(id)?;
                                } else {
                                    let tip = BrushTip::ALL
                                        .into_iter()
                                        .find(|t| t.name() == next)
                                        .unwrap();
                                    self.set_tool(tip);
                                }
                            }
                            "quick" => self.controls.quick = Some(id),
                            "radial" => {
                                self.controls.radial = Some((
                                    id,
                                    ctx.pointer_latest_pos()
                                        .unwrap_or(ctx.content_rect().center()),
                                ));
                                self.controls.just_opened = true;
                            }
                            _ => return Err("Command unavailable".into()),
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.status = Some(e);
        }
    }
    fn execute_item(&mut self, ctx: &egui::Context, item: &Item) {
        match item {
            Item::Command(c) | Item::Tool(c) => self.execute_command(ctx, c),
            Item::Color(c) => self.references.session.workspace.color = *c,
            Item::Separator => {}
        }
    }
    pub(super) fn direct_actions(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(dial) = &mut self.native_dial {
            match dial.drain(ctx.input(|i| i.focused)) {
                Ok(events) => self.controller_actions(ctx, &events),
                Err(error) => {
                    self.controls.controller.reset();
                    self.status = Some(error);
                }
            }
        }
        let events = ctx.input(|i| i.events.clone());
        let mut commands = Vec::new();
        for event in events {
            match event {
                egui::Event::PointerButton {
                    button, pressed, ..
                } if matches!(
                    button,
                    egui::PointerButton::Extra1 | egui::PointerButton::Extra2
                ) =>
                {
                    let n = button as usize;
                    if pressed && self.controls.pointer_down.insert(n) {
                        self.controls.token = self.controls.token.wrapping_add(1);
                        let action = if button == egui::PointerButton::Extra1 {
                            "extra-1"
                        } else {
                            "extra-2"
                        };
                        if let Some(c) = self.controls.devices.dispatch(
                            &self.workspaces.store.current.actions,
                            "pointer",
                            action,
                            self.controls.token,
                        ) {
                            commands.push(c.to_string());
                        }
                    } else if !pressed {
                        self.controls.pointer_down.remove(&n);
                    }
                }
                egui::Event::PointerGone | egui::Event::WindowFocused(false) => {
                    self.controls.pointer_down.clear();
                    self.controls.devices.disconnect("pointer");
                    self.controls.drag = None;
                }
                _ => {}
            }
        }
        if self.workspaces.staged.is_none()
            && self.controls.editor.is_none()
            && self.controls.radial.is_none()
            && !ctx.egui_wants_keyboard_input()
        {
            for c in commands {
                self.execute_command(ctx, &c);
            }
        }
    }
    fn controller_actions(
        &mut self,
        ctx: &egui::Context,
        events: &[drawer_core::controller::Event],
    ) {
        let allowed = self.workspaces.is_focused()
            && ctx.input(|i| i.focused)
            && !self.tool_shortcuts_blocked(ctx);
        let settings = &self.workspaces.store.current.actions;
        // Reassignment cannot carry an old partial rotation into a new command.
        let assignments: Vec<_> = settings
            .devices
            .iter()
            .filter(|d| d.device == drawer_core::controller::DEVICE)
            .cloned()
            .collect();
        let changed = assignments != self.controls.controller_assignments;
        self.controls.controller_assignments = assignments;
        if changed {
            self.controls
                .controller
                .actions(settings.controller, &[], false);
        }
        let actions = self
            .controls
            .controller
            .actions(settings.controller, events, allowed);
        let commands: Vec<_> = actions
            .into_iter()
            .filter_map(|action| {
                settings
                    .devices
                    .iter()
                    .find(|d| d.device == drawer_core::controller::DEVICE && d.action == action)
                    .map(|d| d.command.clone())
            })
            .collect();
        for command in commands {
            if self.tool_shortcuts_blocked(ctx) {
                break;
            }
            self.execute_command(ctx, &command);
        }
    }
    fn access_set(&mut self, ui: &mut egui::Ui, set: &AccessSet) {
        ui.push_id(set.id, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&set.name);
                super::edit::text(
                    ui,
                    self.controls.query.entry(set.id).or_default(),
                    "shortcuts",
                )
                .on_hover_text("Search this set");
            });
            let query = self
                .controls
                .query
                .get(&set.id)
                .map(|q| q.to_lowercase())
                .unwrap_or_default();
            let mut execute = None;
            let mut reorder = None;
            let mut items = |ui: &mut egui::Ui| {
                for (index, item) in set.items.iter().enumerate() {
                    let label = match item {
                        Item::Command(c) => self.workspaces.store.current.command_label(c),
                        _ => item.label(),
                    };
                    if !query.is_empty() && !label.to_lowercase().contains(&query) {
                        continue;
                    }

                    let reason = match item {
                        Item::Command(c) | Item::Tool(c) => self.command_reason(c),
                        _ => None,
                    };
                    let text = if *item == Item::Separator {
                        egui::RichText::new("—")
                    } else if reason.is_some() {
                        egui::RichText::new(&label).color(egui::Color32::GRAY)
                    } else {
                        egui::RichText::new(&label)
                    };
                    let r = ui
                        .add(egui::Button::new(text).sense(egui::Sense::click_and_drag()))
                        .on_hover_text(
                            reason.unwrap_or_else(|| "Click to run; drag to reorder".into()),
                        );
                    #[cfg(test)]
                    {
                        self.controls.item_rects.insert((set.id, index), r.rect);
                        self.controls.item_ids.insert((set.id, index), r.id);
                    }
                    if r.drag_started() && (!set.ctrl_drag || ui.input(|i| i.modifiers.ctrl)) {
                        self.controls.drag = Some((set.id, index));
                    }
                    if r.hovered()
                        && ui.input(|i| i.pointer.any_released())
                        && let Some((id, from)) = self.controls.drag
                        && id == set.id
                        && from != index
                    {
                        reorder = Some((from, index));
                    }
                    if r.clicked() && self.controls.drag.is_none() {
                        execute = Some(item.clone());
                    }
                    if set.display == Display::List {
                        ui.end_row();
                    }
                }
            };
            if set.display == Display::List {
                egui::Grid::new("access-list")
                    .num_columns(1)
                    .show(ui, |ui| items(ui));
            } else {
                ui.horizontal_wrapped(|ui| items(ui));
            }
            if ui.input(|i| i.pointer.any_released()) {
                self.controls.drag = None;
            }
            if let Some((from, to)) = reorder
                && let Some(s) = self
                    .workspaces
                    .store
                    .current
                    .actions
                    .access
                    .iter_mut()
                    .find(|s| s.id == set.id)
            {
                let item = s.items.remove(from);
                s.items.insert(to, item);
            }
            if set.items.is_empty() {
                ui.label("This set is empty. Add items in Input and commands.");
            }
            if let Some(item) = execute {
                self.execute_item(ui.ctx(), &item);
            }
        });
    }
    pub(super) fn access_controls(&mut self, ui: &mut egui::Ui) {
        let sets = self.workspaces.store.current.actions.access.clone();
        for set in sets {
            if set.display == Display::Popup {
                if crate::help::button(ui, format!("{}…", set.name), "shortcuts").clicked() {
                    self.controls.quick = Some(set.id);
                }
            } else {
                self.access_set(ui, &set);
            }
        }
    }
    pub(super) fn command_popups(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.controls.quick {
            if let Some(mut set) = self
                .workspaces
                .store
                .current
                .actions
                .access
                .iter()
                .find(|s| s.id == id)
                .cloned()
            {
                set.display = Display::List;
                let mut open = true;
                egui::Window::new(format!("Quick access — {}", set.name))
                    .id(egui::Id::new("quick-access-popup"))
                    .open(&mut open)
                    .constrain(true)
                    .vscroll(true)
                    .scroll_bar_visibility(crate::ui_behavior::scroll(ctx))
                    .show(ctx, |ui| self.access_set(ui, &set));
                if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.controls.quick = None;
                }
            } else {
                self.controls.quick = None;
                self.status = Some("Quick access set unavailable".into());
            }
        }
        if let Some((id, requested)) = self.controls.radial {
            let Some(radial) = self
                .workspaces
                .store
                .current
                .actions
                .radials
                .iter()
                .find(|r| r.id == id)
                .cloned()
            else {
                self.controls.radial = None;
                return;
            };
            let rect = ctx.content_rect();
            let margin = egui::vec2(130.0, 130.0);
            let mut center = requested;
            center.x = center.x.clamp(
                rect.min.x + margin.x,
                (rect.max.x - margin.x).max(rect.min.x + margin.x),
            );
            center.y = center.y.clamp(
                rect.min.y + margin.y,
                (rect.max.y - margin.y).max(rect.min.y + margin.y),
            );
            #[cfg(test)]
            {
                self.controls.radial_center = Some(center);
            }
            let mut selected = None;
            let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            ctx.input_mut(|i| {
                for (n, key) in [
                    egui::Key::Num1,
                    egui::Key::Num2,
                    egui::Key::Num3,
                    egui::Key::Num4,
                    egui::Key::Num5,
                    egui::Key::Num6,
                    egui::Key::Num7,
                    egui::Key::Num8,
                ]
                .iter()
                .enumerate()
                .take(radial.directions as usize)
                {
                    if i.consume_key(egui::Modifiers::NONE, *key) {
                        selected = Some(n);
                    }
                }
                for (n, key) in [
                    egui::Key::ArrowUp,
                    egui::Key::ArrowRight,
                    egui::Key::ArrowDown,
                    egui::Key::ArrowLeft,
                ]
                .iter()
                .enumerate()
                {
                    if i.consume_key(egui::Modifiers::NONE, *key) {
                        selected = Some(n * radial.directions as usize / 4);
                    }
                }
            });
            egui::Area::new(egui::Id::new("radial-popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(center - margin)
                .show(ctx, |ui| {
                    let (_, painter) = ui.allocate_painter(margin * 2.0, egui::Sense::hover());
                    painter.circle_filled(center, 120.0, egui::Color32::from_rgb(30, 45, 42));
                    painter.circle_filled(center, 30.0, egui::Color32::DARK_GRAY);
                    painter.text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        "Cancel",
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    for (n, slot) in radial.slots.iter().enumerate() {
                        let angle = n as f32 * std::f32::consts::TAU / radial.directions as f32
                            - std::f32::consts::FRAC_PI_2;
                        let pos = center + egui::vec2(angle.cos(), angle.sin()) * 80.0;
                        let label = format!(
                            "{}\n{}",
                            n + 1,
                            slot.as_ref()
                                .map(|c| self.workspaces.store.current.command_label(c))
                                .unwrap_or_else(|| "Empty".into())
                        );
                        let disabled = slot.as_ref().and_then(|c| self.command_reason(c));
                        painter.text(
                            pos,
                            egui::Align2::CENTER_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            if disabled.is_some() {
                                egui::Color32::GRAY
                            } else {
                                egui::Color32::WHITE
                            },
                        );
                    }
                });
            if !self.controls.just_opened
                && ctx.input(|i| i.pointer.primary_pressed())
                && let Some(pos) = ctx.pointer_latest_pos()
            {
                selected = radial.slot_at([pos.x - center.x, pos.y - center.y]);
                cancel = selected.is_none();
            }
            self.controls.just_opened = false;
            if cancel {
                self.controls.radial = None;
            } else if let Some(n) = selected {
                self.controls.radial = None;
                if let Some(command) = &radial.slots[n] {
                    self.execute_command(ctx, command);
                }
            }
        }
    }
    pub(super) fn apply_commands(&mut self, draft: Snapshot) -> Result<(), String> {
        if !draft.actions.valid() {
            return Err("Invalid or duplicate list name, tool, item, or device assignment".into());
        }
        drawer_core::workspaces::validate_bindings(&draft.bindings).map_err(|e| e.to_string())?;
        self.stage_workspace(draft, self.workspaces.store.active.clone());
        let result = self.apply_workspace(
            Scope {
                keys: true,
                bars: true,
                ..Default::default()
            },
            None,
        );
        if result.is_err() {
            self.workspaces.staged = None;
        }
        result
    }
    pub(super) fn command_settings(&mut self, ctx: &egui::Context) {
        let mut retry_dial = false;
        if self.controls.open && self.controls.editor.is_none() {
            self.controls.editor = Some(Editor {
                draft: self.capture_workspace(),
                page: 0,
                selected: 0,
                ..Default::default()
            });
        }
        let Some(mut editor) = self.controls.editor.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut import = false;
        let mut export = false;
        egui::Window::new("Input and commands")
            .open(&mut open)
            .vscroll(true)
            .scroll_bar_visibility(crate::ui_behavior::scroll(ctx))
            .default_width(520.0)
            .show(ctx, |ui| {
                #[cfg(windows)]
                if let Some(dial) = &self.native_dial {
                    ui.label(dial.report());
                    retry_dial = ui.button("Retry Windows dial adapter").clicked();
                }
                ui.horizontal_wrapped(|ui| {
                    for (n, label) in [
                        "Shortcuts",
                        "Tool cycles",
                        "Quick access / command bars",
                        "Radial menus",
                        "Device actions",
                    ]
                    .iter()
                    .enumerate()
                    {
                        if ui
                            .selectable_value(&mut editor.page, n as u8, *label)
                            .clicked()
                        {
                            editor.selected = 0;
                        }
                    }
                });
                let keys = drawer_core::workspaces::validate_bindings(&editor.draft.bindings);
                ui.horizontal_wrapped(|ui| {
                    apply = ui
                        .add_enabled(
                            keys.is_ok() && editor.draft.actions.valid(),
                            egui::Button::new("Apply command settings"),
                        )
                        .clicked();
                    let c = crate::help::button(ui, "Cancel command settings", "shortcuts");
                    cancel = c.clicked();
                    #[cfg(test)]
                    {
                        self.controls.cancel_button = Some(c.rect);
                    }
                    if crate::help::button(ui, "Reset draft to defaults", "shortcuts").clicked() {
                        editor.draft.actions = Default::default();
                        editor.draft.bindings = drawer_core::workspaces::default_bindings();
                        editor.selected = 0;
                    }
                    import =
                        crate::help::button(ui, "Import local settings…", "shortcuts").clicked();
                    export = crate::help::button(ui, "Export draft…", "shortcuts").clicked();
                    if ui
                        .add_enabled(
                            self.storage.is_some(),
                            egui::Button::new("Load before-apply backup into draft"),
                        )
                        .clicked()
                    {
                        match Snapshot::read(
                            &self
                                .storage
                                .as_ref()
                                .unwrap()
                                .root()
                                .join("settings/workspace-before-apply.json"),
                        ) {
                            Ok(s) => {
                                editor.draft.actions = s.actions;
                                editor.draft.bindings = s.bindings;
                                editor.selected = 0;
                            }
                            Err(e) => self.status = Some(e.to_string()),
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
                    .id_salt("command-editor-body")
                    .max_height((ctx.content_rect().height() * 0.5).max(100.0))
                    .show(ui, |ui| match editor.page {
                        0 => edit_bindings(ui, &mut editor),
                        1 => edit_cycles(ui, &mut editor),
                        2 => edit_access(ui, &mut editor, self.references.session.workspace.color),
                        3 => edit_radials(ui, &mut editor),
                        _ => edit_devices(ui, &mut editor),
                    });
                if !editor.draft.actions.valid() {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        "Duplicate/empty name or invalid list, item or device assignment",
                    );
                }
                let keys = drawer_core::workspaces::validate_bindings(&editor.draft.bindings);
                if let Err(e) = &keys {
                    ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
                }
                for report in editor.draft.missing_commands() {
                    ui.colored_label(egui::Color32::YELLOW, report);
                }
            });
        if retry_dial {
            #[cfg(windows)]
            if let Some(dial) = &mut self.native_dial {
                dial.retry();
                self.controls.controller.reset();
            }
        }
        if import
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Local commands", &["json"])
                .pick_file()
        {
            match Snapshot::read(&path) {
                Ok(s) => {
                    editor.draft.actions = s.actions;
                    editor.draft.bindings = s.bindings;
                    editor.selected = 0;
                }
                Err(e) => self.status = Some(e.to_string()),
            }
        }
        if export
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("commands.json")
                .save_file()
            && let Err(e) = editor.draft.export(&path)
        {
            self.status = Some(e.to_string());
        }
        picker(ctx, &mut editor);
        if cancel
            || !open
            || (editor.picker.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            self.controls.open = false;
        } else if apply {
            match self.apply_commands(editor.draft.clone()) {
                Ok(()) => self.controls.open = false,
                Err(e) => {
                    self.status = Some(e);
                    self.controls.editor = Some(editor);
                }
            }
        } else {
            self.controls.editor = Some(editor);
        }
    }
}

fn edit_bindings(ui: &mut egui::Ui, e: &mut Editor) {
    let mut remove = None;
    let mut pick = None;
    for (n, b) in e.draft.bindings.iter_mut().enumerate() {
        ui.push_id(n,|ui|ui.horizontal_wrapped(|ui|{
        super::edit::text(ui,&mut b.key,"shortcuts").on_hover_text("Key name: A–Z, 0–9, [, ], F1–F12, ArrowUp/Right/Down/Left, Space, Enter, Tab, Delete");crate::help::checkbox(ui,&mut b.ctrl,"Ctrl","shortcuts");crate::help::checkbox(ui,&mut b.shift,"Shift","shortcuts");crate::help::checkbox(ui,&mut b.alt,"Alt","shortcuts");if crate::help::button(ui,e.draft.user_tools.command_label(&b.command).unwrap_or_else(||e.draft.actions.label(&b.command)),"shortcuts").clicked(){pick=Some(Picker{target:Target::Binding(n),selected:b.command.clone()});}
if ui.small_button("Remove").clicked(){remove=Some(n);}
    }));
    }
    if let Some(n) = remove {
        e.draft.bindings.remove(n);
    }
    if crate::help::button(ui, "Add shortcut", "shortcuts").clicked() {
        e.draft.bindings.push(Binding {
            key: "F2".into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: "Pen".into(),
        });
    }
    if let Some(p) = pick {
        e.picker = Some(p);
    }
}
fn edit_cycles(ui: &mut egui::Ui, e: &mut Editor) {
    let user_choices = e
        .draft
        .user_tools
        .commands()
        .into_iter()
        .map(|c| {
            let label = e.draft.command_label(&c);
            (c, label)
        })
        .collect::<Vec<_>>();
    let s = &mut e.draft.actions;
    for (n, c) in s.cycles.iter().enumerate() {
        ui.selectable_value(&mut e.selected, n, &c.name);
    }
    let mut add = false;
    let mut duplicate = false;
    let mut delete = false;
    if let Some(c) = s.cycles.get_mut(e.selected) {
        super::edit::text(ui, &mut c.name, "shortcuts");
        let mut remove = None;
        let mut move_to = None;
        for (n, t) in c.tools.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(t);
                if ui.add_enabled(n > 0, egui::Button::new("Up")).clicked() {
                    move_to = Some((n, n - 1));
                }
                if ui
                    .add_enabled(n + 1 < c.tools.len(), egui::Button::new("Down"))
                    .clicked()
                {
                    move_to = Some((n, n + 1));
                }
                if crate::help::button(ui, "Remove", "shortcuts").clicked() {
                    remove = Some(n);
                }
            });
        }
        if let Some((a, b)) = move_to {
            c.tools.swap(a, b);
        }
        if let Some(n) = remove {
            c.tools.remove(n);
        }
        ui.horizontal_wrapped(|ui| {
            for t in BrushTip::ALL {
                if ui
                    .add_enabled(
                        !c.tools.iter().any(|v| v == t.name()),
                        egui::Button::new(format!("Add {}", t.name())),
                    )
                    .clicked()
                {
                    c.tools.push(t.name().into());
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (command, label) in &user_choices {
                if ui
                    .add_enabled(
                        !c.tools.contains(command),
                        egui::Button::new(format!("Add {label}")),
                    )
                    .clicked()
                {
                    c.tools.push(command.clone());
                }
            }
        });
        if crate::help::button(ui, "Reverse order", "shortcuts").clicked() {
            c.tools.reverse();
        }
        ui.label("Assign Next tool / Previous tool for this list in Shortcuts or Device actions");
    } else {
        ui.label("No tool cycles. Create a list to assign next/previous.");
    }
    ui.horizontal(|ui| {
        add = crate::help::button(ui, "New cycle", "shortcuts").clicked();
        duplicate = crate::help::button(ui, "Duplicate cycle", "shortcuts").clicked();
        delete = crate::help::button(ui, "Delete cycle", "shortcuts").clicked();
    });
    if delete && e.selected < s.cycles.len() {
        s.cycles.remove(e.selected);
        e.selected = 0;
    }
    if duplicate
        && let Some(mut c) = s.cycles.get(e.selected).cloned()
        && let Ok(id) = s.allocate_id()
    {
        c.id = id;
        c.name = format!("{} copy {id}", c.name);
        s.cycles.push(c);
        e.selected = s.cycles.len() - 1;
    }
    if add && let Ok(id) = s.allocate_id() {
        s.cycles.push(Cycle {
            id,
            name: format!("Cycle {id}"),
            tools: Vec::new(),
        });
        e.selected = s.cycles.len() - 1;
    }
}
fn edit_access(ui: &mut egui::Ui, e: &mut Editor, color: [u8; 4]) {
    let s = &mut e.draft.actions;
    for (n, set) in s.access.iter().enumerate() {
        ui.selectable_value(&mut e.selected, n, format!("{:?}: {}", set.kind, set.name));
    }
    let mut pick = None;
    if let Some(set) = s.access.get_mut(e.selected) {
        super::edit::text(ui, &mut set.name, "shortcuts");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut set.kind, SetKind::QuickAccess, "Quick access");
            ui.selectable_value(&mut set.kind, SetKind::CommandBar, "Command bar");
        });
        ui.horizontal(|ui| {
            for d in Display::ALL {
                ui.selectable_value(&mut set.display, d, format!("{d:?}"));
            }
        });
        crate::help::checkbox(
            ui,
            &mut set.ctrl_drag,
            "Require Ctrl while dragging to reorder",
            "shortcuts",
        );
        let mut remove = None;
        let mut swap = None;
        for (n, item) in set.items.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(item.label());
                match item {
                    Item::Command(c) | Item::Tool(c) => {
                        if crate::help::button(ui, "Choose command / tool", "shortcuts").clicked() {
                            pick = Some(Picker {
                                target: Target::Item(set.id, n),
                                selected: c.clone(),
                            });
                        }
                    }
                    Item::Color(c) => {
                        ui.color_edit_button_srgba_unmultiplied(c);
                    }
                    _ => {}
                }
                if ui.add_enabled(n > 0, egui::Button::new("Up")).clicked() {
                    swap = Some((n, n - 1));
                }
                if crate::help::button(ui, "Remove", "shortcuts").clicked() {
                    remove = Some(n);
                }
            });
        }
        if let Some((a, b)) = swap {
            set.items.swap(a, b);
        }
        if let Some(n) = remove {
            set.items.remove(n);
        }
        ui.horizontal_wrapped(|ui| {
            if crate::help::button(ui, "Add command", "shortcuts").clicked() {
                set.items.push(Item::Command("undo".into()));
            }
            if crate::help::button(ui, "Add tool", "shortcuts").clicked() {
                set.items.push(Item::Tool("Pen".into()));
            }
            if crate::help::button(ui, "Register current fixed color", "shortcuts").clicked() {
                set.items.push(Item::Color(color));
            }
            if crate::help::button(ui, "Add separator", "shortcuts").clicked() {
                set.items.push(Item::Separator);
            }
        });
    } else {
        ui.label("No quick access or command bars.");
    }
    let mut new = false;
    let mut copy = false;
    let mut delete = false;
    ui.horizontal(|ui| {
        new = crate::help::button(ui, "New set", "shortcuts").clicked();
        copy = crate::help::button(ui, "Duplicate set", "shortcuts").clicked();
        delete = crate::help::button(ui, "Delete set", "shortcuts").clicked();
    });
    if delete && e.selected < s.access.len() {
        s.access.remove(e.selected);
        e.selected = 0;
    }
    if copy
        && let Some(mut set) = s.access.get(e.selected).cloned()
        && let Ok(id) = s.allocate_id()
    {
        set.id = id;
        set.name = format!("{} copy {id}", set.name);
        s.access.push(set);
        e.selected = s.access.len() - 1;
    }
    if new && let Ok(id) = s.allocate_id() {
        s.access.push(AccessSet {
            id,
            name: format!("Set {id}"),
            kind: SetKind::QuickAccess,
            display: Display::Tiles,
            ctrl_drag: false,
            items: Vec::new(),
        });
        e.selected = s.access.len() - 1;
    }
    if let Some(p) = pick {
        e.picker = Some(p);
    }
}
fn edit_radials(ui: &mut egui::Ui, e: &mut Editor) {
    let labels = e.draft.clone();
    let s = &mut e.draft.actions;
    for (n, r) in s.radials.iter().enumerate() {
        ui.selectable_value(&mut e.selected, n, &r.name);
    }
    let mut pick = None;
    if let Some(r) = s.radials.get_mut(e.selected) {
        super::edit::text(ui, &mut r.name, "shortcuts");
        ui.horizontal(|ui| {
            for n in [4, 8] {
                if ui
                    .selectable_label(r.directions == n, format!("{n} directions"))
                    .clicked()
                {
                    r.resize(n);
                }
            }
        });
        for (n, slot) in r.slots.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Slot {}", n + 1));
                if ui
                    .button(
                        slot.as_ref()
                            .map(|c| labels.command_label(c))
                            .unwrap_or_else(|| "Empty".into()),
                    )
                    .clicked()
                {
                    pick = Some(Picker {
                        target: Target::Slot(r.id, n),
                        selected: slot.clone().unwrap_or_default(),
                    });
                }
                if crate::help::button(ui, "Clear", "shortcuts").clicked() {
                    *slot = None;
                }
            });
        }
        ui.label("Assign Open radial menu for this list in Shortcuts or Device actions. Popup keys 1–8 select slots; arrows select cardinal slots.");
    }
    let mut new = false;
    let mut delete = false;
    ui.horizontal(|ui| {
        new = crate::help::button(ui, "New radial menu", "shortcuts").clicked();
        delete = crate::help::button(ui, "Delete radial menu", "shortcuts").clicked();
    });
    if delete && e.selected < s.radials.len() {
        s.radials.remove(e.selected);
        e.selected = 0;
    }
    if new && let Ok(id) = s.allocate_id() {
        s.radials.push(Radial {
            id,
            name: format!("Radial {id}"),
            directions: 4,
            slots: vec![None; 4],
        });
        e.selected = s.radials.len() - 1;
    }
    if let Some(p) = pick {
        e.picker = Some(p);
    }
}
fn edit_devices(ui: &mut egui::Ui, e: &mut Editor) {
    let labels = e.draft.clone();
    ui.label("Pointer: extra-1 / extra-2. Windows wheel: windows-radial / clockwise, counterclockwise, click. Select Drawer in the OS wheel menu. Keyboard assignments use the same commands. Physical device validation pending.");
    ui.checkbox(
        &mut e.draft.actions.controller.enabled,
        "Enable assigned Windows dial actions",
    );
    ui.horizontal(|ui| {
        ui.label("Degrees per command step");
        ui.add(
            egui::DragValue::new(&mut e.draft.actions.controller.degrees_per_step)
                .range(1.0..=180.0),
        );
    });
    if ui.button("Add default dial bindings").clicked() {
        for (action, command) in [
            ("clockwise", "brush-grow"),
            ("counterclockwise", "brush-shrink"),
            ("click", "cycle-next:1"),
        ] {
            if !e
                .draft
                .actions
                .devices
                .iter()
                .any(|d| d.device == drawer_core::controller::DEVICE && d.action == action)
            {
                e.draft.actions.devices.push(actions::DeviceBinding {
                    device: drawer_core::controller::DEVICE.into(),
                    action: action.into(),
                    command: command.into(),
                });
            }
        }
    }
    let mut pick = None;
    let mut remove = None;
    for (n, d) in e.draft.actions.devices.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            super::edit::text(ui, &mut d.device, "shortcuts");
            super::edit::text(ui, &mut d.action, "shortcuts");
            if crate::help::button(ui, labels.command_label(&d.command), "shortcuts").clicked() {
                pick = Some(Picker {
                    target: Target::Device(n),
                    selected: d.command.clone(),
                });
            }
            if crate::help::button(ui, "Remove", "shortcuts").clicked() {
                remove = Some(n);
            }
        });
    }
    if let Some(n) = remove {
        e.draft.actions.devices.remove(n);
    }
    if crate::help::button(ui, "Assign pointer action", "shortcuts").clicked() {
        e.draft.actions.devices.push(actions::DeviceBinding {
            device: "pointer".into(),
            action: "extra-1".into(),
            command: "cycle-next:1".into(),
        });
    }
    if let Some(p) = pick {
        e.picker = Some(p);
    }
}
fn picker(ctx: &egui::Context, e: &mut Editor) {
    let Some(mut p) = e.picker.take() else {
        return;
    };
    let mut open = true;
    let mut apply = false;
    let mut cancel = false;
    egui::Window::new("Choose command")
        .open(&mut open)
        .vscroll(true)
        .scroll_bar_visibility(crate::ui_behavior::scroll(ctx))
        .show(ctx, |ui| {
            ui.label("Current assignment is selected. Only Confirm changes it.");
            ui.label(format!("Selected: {}", e.draft.command_label(&p.selected)));
            ui.horizontal(|ui| {
                let confirm = crate::help::button(ui, "Confirm command", "shortcuts");
                apply = confirm.clicked();
                #[cfg(test)]
                {
                    e.picker_confirm = Some(confirm.rect);
                }
                cancel = crate::help::button(ui, "Cancel command choice", "shortcuts").clicked();
            });
            egui::ScrollArea::vertical()
                .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
                .id_salt("command-picker-list")
                .max_height(ctx.content_rect().height() * 0.5)
                .show(ui, |ui| {
                    for command in e.draft.commands() {
                        ui.selectable_value(
                            &mut p.selected,
                            command.clone(),
                            e.draft.command_label(&command),
                        );
                    }
                });
        });
    if apply {
        match p.target {
            Target::Binding(n) => {
                if let Some(b) = e.draft.bindings.get_mut(n) {
                    b.command = p.selected;
                }
            }
            Target::Item(id, n) => {
                if let Some(item) = e
                    .draft
                    .actions
                    .access
                    .iter_mut()
                    .find(|s| s.id == id)
                    .and_then(|s| s.items.get_mut(n))
                {
                    *item = if BrushTip::ALL.iter().any(|t| t.name() == p.selected) {
                        Item::Tool(p.selected)
                    } else {
                        Item::Command(p.selected)
                    };
                }
            }
            Target::Slot(id, n) => {
                if let Some(slot) = e
                    .draft
                    .actions
                    .radials
                    .iter_mut()
                    .find(|r| r.id == id)
                    .and_then(|r| r.slots.get_mut(n))
                {
                    *slot = (!p.selected.is_empty()).then_some(p.selected);
                }
            }
            Target::Device(n) => {
                if let Some(d) = e.draft.actions.devices.get_mut(n) {
                    d.command = p.selected;
                }
            }
        }
    } else if !cancel && open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        e.picker = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn controller_replay_applies_live_brush_zoom_tool_modal_and_duplicate_rules() {
        smoke_controller().unwrap();
    }
    #[test]
    fn keyboard_equivalence_unknown_removed_bindings_and_reassignment_clear_fraction() {
        use drawer_core::controller::{DEVICE, Event, Kind};
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let binding = actions::DeviceBinding {
            device: DEVICE.into(),
            action: "clockwise".into(),
            command: "zoom-in".into(),
        };
        app.workspaces.store.current.actions.devices.push(binding);
        app.workspaces.store.current.bindings.push(Binding {
            key: "F3".into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: "zoom-in".into(),
        });
        let e = |serial, kind| Event { serial, kind };
        app.controller_actions(&ctx, &[e(1, Kind::Acquired), e(2, Kind::Rotation(15.0))]);
        let wheel_zoom = app.documents[0].view.zoom;
        app.documents[0].view.zoom = 1.0;
        frame(&ctx, &mut app, vec![key(egui::Key::F3, true)]);
        assert_eq!(app.documents[0].view.zoom, wheel_zoom);
        frame(&ctx, &mut app, vec![key(egui::Key::F3, false)]);
        app.controller_actions(&ctx, &[e(3, Kind::Rotation(14.0))]);
        app.workspaces.store.current.actions.devices[0].command = "zoom-out".into();
        app.controller_actions(&ctx, &[e(4, Kind::Rotation(1.0))]);
        assert_eq!(app.documents[0].view.zoom, wheel_zoom);
        app.workspaces.store.current.actions.devices.clear();
        app.controller_actions(&ctx, &[e(5, Kind::Rotation(360.0)), e(6, Kind::Click)]);
        assert_eq!(app.documents[0].view.zoom, wheel_zoom);
        app.workspaces
            .store
            .current
            .actions
            .devices
            .push(actions::DeviceBinding {
                device: DEVICE.into(),
                action: "unsupported-air-gesture".into(),
                command: "zoom-in".into(),
            });
        assert!(!app.workspaces.store.current.actions.missing().is_empty());
        app.controller_actions(&ctx, &[e(7, Kind::Rotation(360.0)), e(8, Kind::Click)]);
        assert_eq!(app.documents[0].view.zoom, wheel_zoom);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn command_picker_confirms_only_draft_and_cancel_restores_assignments() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let original = app.capture_workspace();
        app.controls.open = true;
        frame(&ctx, &mut app, vec![]);
        app.controls.editor.as_mut().unwrap().picker = Some(Picker {
            target: Target::Slot(4, 0),
            selected: "Pen".into(),
        });
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.controls
                .editor
                .as_ref()
                .unwrap()
                .picker
                .as_ref()
                .unwrap()
                .selected,
            "Pen"
        );
        app.controls
            .editor
            .as_mut()
            .unwrap()
            .picker
            .as_mut()
            .unwrap()
            .selected = "Wash".into();
        let button = app
            .controls
            .editor
            .as_ref()
            .unwrap()
            .picker_confirm
            .unwrap();
        click(&ctx, &mut app, button);
        assert!(app.controls.editor.as_ref().unwrap().picker.is_none());
        assert_eq!(
            app.controls.editor.as_ref().unwrap().draft.actions.radials[0].slots[0].as_deref(),
            Some("Wash")
        );
        assert_eq!(app.capture_workspace(), original);
        let cancel = app.controls.cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(!app.controls.open);
        assert_eq!(app.capture_workspace(), original);
        let id = app.controls.item_ids.get(&(2, 5)).copied();
        frame(&ctx, &mut app, vec![]);
        let id = id.unwrap_or(app.controls.item_ids[&(2, 5)]);
        app.references.session.workspace.color = [255, 255, 255, 255];
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, &mut app, vec![key(egui::Key::Enter, true)]);
        assert_eq!(app.references.session.workspace.color, [0, 0, 0, 255]);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn edited_shortcuts_cycle_devices_conflict_cancel_and_restart() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        let document = app.documents[0].snapshot();
        let mut draft = app.capture_workspace();
        draft.bindings[5].key = "F2".into();
        draft.actions.controller.degrees_per_step = 7.5;
        draft.actions.devices.push(actions::DeviceBinding {
            device: drawer_core::controller::DEVICE.into(),
            action: "clockwise".into(),
            command: "zoom-in".into(),
        });
        draft.actions.cycles[0].tools = vec!["Pen".into(), "deleted-preset".into(), "Wash".into()];
        draft.actions.devices.push(actions::DeviceBinding {
            device: "pointer".into(),
            action: "extra-1".into(),
            command: "cycle-next:1".into(),
        });
        app.apply_commands(draft.clone()).unwrap();
        app.set_tool(BrushTip::Eraser);
        frame(&ctx, &mut app, vec![key(egui::Key::B, true)]);
        assert_eq!(app.selected, BrushTip::Eraser);
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::B, false), key(egui::Key::F2, true)],
        );
        assert_eq!(app.selected, BrushTip::Pen);
        let event = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(500.0, 500.0),
            button: egui::PointerButton::Extra1,
            pressed,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut app, vec![event(true), event(true)]);
        assert_eq!(app.selected, BrushTip::Wash);
        frame(&ctx, &mut app, vec![event(true)]);
        assert_eq!(app.selected, BrushTip::Wash);
        frame(&ctx, &mut app, vec![event(false), event(true)]);
        assert_eq!(app.selected, BrushTip::Pen);
        frame(&ctx, &mut app, vec![egui::Event::PointerGone, event(true)]);
        assert_eq!(app.selected, BrushTip::Wash);
        frame(
            &ctx,
            &mut app,
            vec![event(false), key(egui::Key::F2, false)],
        );
        let before = app.capture_workspace();
        let mut conflict = before.clone();
        conflict.bindings.push(conflict.bindings[0].clone());
        assert!(app.apply_commands(conflict).is_err());
        assert_eq!(app.capture_workspace(), before);
        assert!(app.workspaces.staged.is_none());
        app.controls.open = true;
        frame(&ctx, &mut app, vec![]);
        app.controls
            .editor
            .as_mut()
            .unwrap()
            .draft
            .actions
            .cycles
            .clear();
        app.controls
            .editor
            .as_mut()
            .unwrap()
            .draft
            .actions
            .controller
            .enabled = false;
        frame(&ctx, &mut app, vec![]);
        let cancel = app.controls.cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(
            !app.controls.open,
            "before={cancel:?} after={:?} staged={}",
            app.controls.cancel_button,
            app.workspaces.staged.is_some(),
        );
        assert_eq!(app.capture_workspace(), before);
        assert_eq!(app.documents[0].snapshot(), document);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.capture_workspace().actions, draft.actions);
        assert_eq!(restarted.capture_workspace().bindings, draft.bindings);
        let mut export = app.capture_workspace();
        export.actions.radials[0].resize(8);
        let path = root.0.join("commands.json");
        export.export(&path).unwrap();
        assert_eq!(Snapshot::read(&path).unwrap(), export);
        frame(&ctx, &mut app, vec![key(egui::Key::T, true)]);
        assert_eq!(app.selected, BrushTip::Pen);
        frame(&ctx, &mut app, vec![key(egui::Key::T, true)]);
        assert_eq!(app.selected, BrushTip::Pen);
    }
    #[test]
    fn radial_mouse_keyboard_empty_escape_outside_and_screen_edge_agree() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        for directions in [4, 8] {
            app.workspaces.store.current.actions.radials[0].resize(directions);
            app.workspaces.store.current.actions.radials[0].slots = (0..directions)
                .map(|n| Some(BrushTip::ALL[n as usize % 4].name().into()))
                .collect();
            for n in 0..directions {
                app.execute_command(&ctx, "radial:4");
                app.controls.radial.as_mut().unwrap().1 = egui::pos2(1099.0, 759.0);
                frame(&ctx, &mut app, vec![]);
                let center = app.controls.radial_center.unwrap();
                assert!(center.x <= 970.0 && center.y <= 630.0);
                let angle = n as f32 * std::f32::consts::TAU / directions as f32
                    - std::f32::consts::FRAC_PI_2;
                let pos = center + egui::vec2(angle.cos(), angle.sin()) * 80.0;
                frame(
                    &ctx,
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                );
                let mouse = app.selected;
                assert!(app.controls.radial.is_none());
                frame(
                    &ctx,
                    &mut app,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    }],
                );
                app.execute_command(&ctx, "radial:4");
                frame(&ctx, &mut app, vec![]);
                let k = [
                    egui::Key::Num1,
                    egui::Key::Num2,
                    egui::Key::Num3,
                    egui::Key::Num4,
                    egui::Key::Num5,
                    egui::Key::Num6,
                    egui::Key::Num7,
                    egui::Key::Num8,
                ][n as usize];
                frame(&ctx, &mut app, vec![key(k, true)]);
                assert_eq!(app.selected, mouse);
                assert!(app.controls.radial.is_none());
                frame(&ctx, &mut app, vec![key(k, false)]);
            }
        }
        app.workspaces.store.current.actions.radials[0].slots[0] = None;
        let tool = app.selected;
        app.execute_command(&ctx, "radial:4");
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![key(egui::Key::Num1, true)]);
        assert_eq!(app.selected, tool);
        for cancel in [
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: egui::pos2(0.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ] {
            app.execute_command(&ctx, "radial:4");
            frame(&ctx, &mut app, vec![]);
            frame(&ctx, &mut app, vec![cancel]);
            assert!(app.controls.radial.is_none());
            assert_eq!(app.selected, tool);
        }
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn quick_access_fixed_color_missing_commands_reorder_and_keyboard_focus() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        app.workspaces.store.current.actions.access[0].items = vec![
            Item::Tool("Pen".into()),
            Item::Tool("Wash".into()),
            Item::Color([40, 50, 60, 128]),
            Item::Command("removed-command".into()),
        ];
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        app.references.session.workspace.color = [200, 0, 0, 255];
        let color = app.controls.item_rects[&(2, 2)];
        click(&ctx, &mut app, color);
        assert_eq!(app.references.session.workspace.color, [40, 50, 60, 128]);
        assert!(app.command_reason("removed-command").is_some());
        let a = app.controls.item_rects[&(2, 0)].center();
        let b = app.controls.item_rects[&(2, 1)].center();
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(a),
                egui::Event::PointerButton {
                    pos: a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(b)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: b,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(
            app.workspaces.store.current.actions.access[0].items[0],
            Item::Tool("Wash".into())
        );
        app.workspaces.store.current.actions.access[0].ctrl_drag = true;
        for ctrl in [false, true] {
            let old = app.workspaces.store.current.actions.access[0].items.clone();
            let a = app.controls.item_rects[&(2, 0)].center();
            let b = app.controls.item_rects[&(2, 1)].center();
            let mods = if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            for mut events in [
                vec![
                    egui::Event::PointerMoved(a),
                    egui::Event::PointerButton {
                        pos: a,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: mods,
                    },
                ],
                vec![egui::Event::PointerMoved(b)],
                vec![egui::Event::PointerButton {
                    pos: b,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: mods,
                }],
            ] {
                events.insert(0, egui::Event::ModifiersChanged(mods));
                let mut o = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 760.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.show(ui),
                );
                o.textures_delta.clear();
            }
            if ctrl {
                assert_eq!(
                    app.workspaces.store.current.actions.access[0].items[0],
                    old[1]
                );
            } else {
                assert_eq!(app.workspaces.store.current.actions.access[0].items, old);
            }
        }
        app.workspaces.store.current.actions.access[0].display = Display::Popup;
        app.execute_command(&ctx, "quick:2");
        for _ in 0..2 {
            let mut o = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 480.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            o.textures_delta.clear();
        }
        assert_eq!(app.controls.quick, Some(2));
        frame(&ctx, &mut app, vec![key(egui::Key::Escape, true)]);
        assert!(app.controls.quick.is_none());
        let mut text = String::new();
        let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        });
        o.textures_delta.clear();
        let tool = app.selected;
        let mut o = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: "한".into(),
                        active_range_chars: None,
                    }),
                    key(egui::Key::T, true),
                ],
                ..Default::default()
            },
            |ui| app.basic_shortcuts(ui.ctx()),
        );
        o.textures_delta.clear();
        assert_eq!(app.selected, tool);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
