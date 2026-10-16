//! Local workspace preferences, independent of document data and undo history.
use crate::{assets::BrushTip, display_view::Monitor, panels, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::Path,
};

const LIMIT: u64 = 16 * 1024 * 1024;
pub const COMMANDS: [&str; 11] = [
    "save", "save-as", "undo", "redo", "Pen", "Pencil", "Eraser", "Wash", "Marker", "Smudge",
    "PixelPen",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dock {
    Left,
    Right,
    Floating,
}
impl Dock {
    pub const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Floating];
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub dock: Dock,
    pub monitor: Option<String>,
    /// Desktop physical pixels; floating windows use their receiving DPI.
    pub position: [f64; 2],
    pub size: [f64; 2],
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            dock: Dock::Left,
            monitor: None,
            position: [40.0, 80.0],
            size: [260.0, 320.0],
        }
    }
}
impl Placement {
    fn valid(&self) -> bool {
        self.monitor
            .as_ref()
            .is_none_or(|s| !s.is_empty() && s.len() <= 512)
            && self
                .position
                .iter()
                .all(|n| n.is_finite() && n.abs() <= 1e6)
            && self.size.iter().all(|n| (80.0..=4096.0).contains(n))
    }
    pub fn recover(&mut self, monitors: &[Monitor]) -> Option<String> {
        if self.dock != Dock::Floating || monitors.is_empty() {
            return None;
        }
        let monitor = crate::display_view::selected(monitors, self.monitor.as_deref())?;
        let missing = self.monitor.as_ref().is_some_and(|key| key != &monitor.key);
        let old = self.position;
        for (axis, old_position) in old.iter().enumerate() {
            // Reserve a title bar margin so a moved window can always be recovered.
            self.size[axis] = self.size[axis].min((monitor.size[axis] as f64 - 48.0).max(80.0));
            self.position[axis] = old_position.clamp(
                monitor.position[axis] as f64,
                (monitor.position[axis] as f64 + monitor.size[axis] as f64
                    - self.size[axis]
                    - 32.0)
                    .max(monitor.position[axis] as f64),
            );
        }
        if missing || old != self.position {
            let reason = format!(
                "Panel screen {:?} unavailable or outside the screen; moved to {}",
                self.monitor, monitor.key
            );
            self.monitor = Some(monitor.key.clone());
            Some(reason)
        } else {
            None
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: String,
}
impl Binding {
    pub fn label(&self) -> String {
        format!(
            "{}{}{}{}",
            if self.ctrl { "Ctrl+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            self.key
        )
    }
    fn valid(&self) -> bool {
        !self.key.is_empty()
            && self.key.len() <= 32
            && !self.command.is_empty()
            && self.command.len() <= 128
    }
}
pub fn default_bindings() -> Vec<Binding> {
    [
        ("S", true, false, "save"),
        ("S", true, true, "save-as"),
        ("Z", true, false, "undo"),
        ("Y", true, false, "redo"),
        ("Z", true, true, "redo"),
        ("B", false, false, "Pen"),
        ("N", false, false, "Pencil"),
        ("E", false, false, "Eraser"),
        ("W", false, false, "Wash"),
        ("Y", false, false, "Marker"),
        ("U", false, false, "Smudge"),
        ("P", false, false, "PixelPen"),
        ("T", false, false, "cycle-next:1"),
        ("T", false, true, "cycle-prev:1"),
        ("Q", false, false, "quick:2"),
        ("R", false, false, "radial:4"),
        ("F1", false, false, "help"),
        ("F12", false, false, "screen-mode"),
        ("[", false, false, "brush-shrink"),
        ("]", false, false, "brush-grow"),
        ("[", false, true, "opacity-down"),
        ("]", false, true, "opacity-up"),
        ("1", false, false, "opacity-10"),
        ("2", false, false, "opacity-20"),
        ("3", false, false, "opacity-30"),
        ("4", false, false, "opacity-40"),
        ("5", false, false, "opacity-50"),
        ("6", false, false, "opacity-60"),
        ("7", false, false, "opacity-70"),
        ("8", false, false, "opacity-80"),
        ("9", false, false, "opacity-90"),
        ("0", false, false, "opacity-100"),
        ("L", false, false, "connect-line"),
        ("I", false, false, "Eyedropper"),
        ("M", false, false, "Move"),
    ]
    .into_iter()
    .map(|(key, ctrl, shift, command)| Binding {
        key: key.into(),
        ctrl,
        shift,
        alt: false,
        command: command.into(),
    })
    .collect()
}
pub fn validate_bindings(bindings: &[Binding]) -> io::Result<()> {
    let mut used = HashSet::new();
    for b in bindings {
        if !b.valid() || !supported_key(&b.key) || !used.insert((&b.key, b.ctrl, b.shift, b.alt)) {
            return Err(io::Error::other(format!(
                "Conflicting or invalid shortcut: {}",
                b.label()
            )));
        }
        if (b.alt && b.key == "F4")
            || (b.ctrl && ["PageUp", "PageDown"].contains(&b.key.as_str()))
            || (b.alt && ["1", "2", "3", "4", "5", "6", "7", "8", "9"].contains(&b.key.as_str()))
        {
            return Err(io::Error::other(format!(
                "Reserved shortcut: {}",
                b.label()
            )));
        }
    }
    Ok(())
}
pub const PAD_SIZE: [usize; 2] = [128, 96];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scratchpad {
    #[serde(default)]
    pub background: [u8; 4],
    #[serde(default = "pad_zoom")]
    pub zoom: f64,
    #[serde(default)]
    pub slots: Vec<crate::brush_presets::Preset>,
    #[serde(default)]
    pub tool_slot: Option<usize>,
    pub visible: bool,
    pub pixels: Vec<[u8; 4]>,
}
fn pad_zoom() -> f64 {
    1.0
}
impl Default for Scratchpad {
    fn default() -> Self {
        Self {
            background: [0; 4],
            zoom: 1.0,
            slots: vec![],
            tool_slot: None,
            visible: false,
            pixels: vec![[0; 4]; PAD_SIZE[0] * PAD_SIZE[1]],
        }
    }
}
impl Scratchpad {
    pub fn valid(&self) -> bool {
        self.pixels.len() == PAD_SIZE[0] * PAD_SIZE[1]
            && (0.25..=2.0).contains(&self.zoom)
            && self.slots.len() <= 8
            && self.slots.iter().all(|p| p.validate().is_ok())
            && self.tool_slot.is_none_or(|n| n < self.slots.len())
    }
    pub fn stroke(
        &mut self,
        from: [f64; 2],
        to: [f64; 2],
        size: f64,
        tip: BrushTip,
        color: [u8; 4],
    ) {
        self.stroke_tip(from, to, size, (tip, Default::default()), color);
    }
    pub fn stroke_tip(
        &mut self,
        from: [f64; 2],
        to: [f64; 2],
        size: f64,
        tip: (BrushTip, crate::tip_profile::Settings),
        color: [u8; 4],
    ) {
        let (tip, profile) = tip;
        if !from
            .iter()
            .chain(to.iter())
            .all(|n| n.is_finite() && n.abs() <= 1e6)
            || !size.is_finite()
        {
            return;
        }
        if self.pixels.len() != PAD_SIZE[0] * PAD_SIZE[1] {
            return;
        }
        let Ok(mut d) = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: PAD_SIZE[0] as u32,
                height: PAD_SIZE[1] as u32,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            4 * PAD_SIZE[0] * PAD_SIZE[1],
        ) else {
            return;
        };
        d.layers[0].pixels.clone_from(&self.pixels);
        let v = crate::basic_brush::values(tip);
        let style = crate::painting::Style {
            clone_stamp: Default::default(),
            variation: Default::default(),
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile,
            texture: Default::default(),
            material: crate::basic_brush::material(tip),
            shape: crate::brush_cursor::Shape {
                size: size.clamp(1.0, 64.0),
                aspect: 1.0,
                angle: 0.0,
            },
            tip,
            color,
            smoothing: 0.0,
            opacity: v[4],
            hardness: v[5],
            spacing: v[6],
            pressure: crate::pressure::Mapping {
                size: false,
                opacity: false,
                ..Default::default()
            },
        };
        if let Ok(mut stroke) = crate::painting::Stroke::begin(&d, 0, style)
            && stroke.sample(&d, from).is_ok()
            && stroke.sample(&d, to).is_ok()
            && stroke.commit(&mut d).is_ok()
        {
            self.pixels = d.layers.remove(0).pixels;
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(default)]
    pub related_colors: crate::related_colors::Settings,
    #[serde(default)]
    pub color_tables: crate::color_tables::Settings,
    #[serde(default)]
    pub mixer: Scratchpad,
    #[serde(default)]
    pub color_picker: crate::color_selector::Settings,
    #[serde(default)]
    pub locks: crate::tool_locks::Settings,
    #[serde(default)]
    pub user_tools: crate::user_tools::Library,
    #[serde(default)]
    pub eyedropper: crate::eyedropper::State,
    pub version: u32,
    pub placements: [Placement; 4],
    pub order: [panels::Kind; 4],
    pub panels: panels::Settings,
    pub navigator: crate::navigator_preferences::Preferences,
    pub display: crate::display_view::Preferences,
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub actions: crate::actions::Settings,
    #[serde(default)]
    pub input: Box<crate::tool_input::Settings>,
    #[serde(default)]
    pub pointer: crate::pointer_input::Settings,
    #[serde(default)]
    pub pressure: crate::pressure::Settings,
    #[serde(default)]
    pub help: crate::help::Preferences,
    #[serde(default)]
    pub behavior: crate::ui_behavior::Settings,
    #[serde(default)]
    pub cursors: crate::brush_cursor::Settings,
    #[serde(default)]
    pub information: crate::information::Settings,
    #[serde(default)]
    pub layout: crate::layout::Settings,
    #[serde(default)]
    pub properties: Option<crate::tool_properties::Settings>,
    #[serde(default)]
    pub brushes: crate::brush_presets::Book,
    #[serde(default)]
    pub preferences: crate::preferences::Settings,
    pub interface: crate::ui_scale::Settings,
    pub interpolation: crate::interpolation::Settings,
    pub aids: crate::aids::AidPreferences,
    pub tool: String,
    pub color: [u8; 4],
    pub scratchpad: Scratchpad,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            placements: std::array::from_fn(|_| Placement::default()),
            order: panels::Kind::ALL,
            panels: Default::default(),
            navigator: Default::default(),
            display: Default::default(),
            bindings: default_bindings(),
            actions: Default::default(),
            input: Default::default(),
            pointer: Default::default(),
            pressure: Default::default(),
            help: Default::default(),
            behavior: Default::default(),
            cursors: Default::default(),
            information: Default::default(),
            layout: Default::default(),
            properties: None,
            brushes: Default::default(),
            preferences: Default::default(),
            interface: Default::default(),
            interpolation: Default::default(),
            aids: Default::default(),
            tool: "Pen".into(),
            related_colors: Default::default(),
            color_tables: Default::default(),
            mixer: Default::default(),
            color_picker: Default::default(),
            locks: Default::default(),
            user_tools: Default::default(),
            eyedropper: Default::default(),
            color: [0, 0, 0, 255],
            scratchpad: Default::default(),
        }
    }
}
#[derive(Clone, Copy, Default)]
pub struct Scope {
    pub layout: bool,
    pub keys: bool,
    pub bars: bool,
    pub other: bool,
}
impl Scope {
    pub const ALL: Self = Self {
        layout: true,
        keys: true,
        bars: true,
        other: true,
    };
    pub fn any(self) -> bool {
        self.layout || self.keys || self.bars || self.other
    }
}
impl Snapshot {
    pub fn commands(&self) -> Vec<String> {
        let mut list = self.actions.commands();
        list.extend(self.user_tools.commands());
        list
    }
    pub fn command_label(&self, command: &str) -> String {
        self.user_tools
            .command_label(command)
            .unwrap_or_else(|| self.actions.label(command))
    }
    pub fn missing_commands(&self) -> Vec<String> {
        let mut reports = self
            .actions
            .missing()
            .into_iter()
            .filter(|line| {
                !line
                    .split_whitespace()
                    .any(|part| self.user_tools.has_command(part))
            })
            .collect::<Vec<_>>();
        reports.extend(self.user_tools.missing_refs(self));
        reports
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.placements.iter().all(Placement::valid)
            && panels::Kind::ALL
                .iter()
                .all(|k| self.order.iter().filter(|v| *v == k).count() == 1)
            && self.panels.valid()
            && self.navigator.valid()
            && self.display.valid()
            && self.interface.valid()
            && self.input.valid()
            && self.related_colors.valid()
            && self.color_tables.valid()
            && self.mixer.valid()
            && self.color_picker.valid()
            && self.eyedropper.valid()
            && self.user_tools.valid()
            && self.locks.valid()
            && (self.locks.empty() || self.properties.as_ref().is_some_and(|p| p.version == 2))
            && self.pointer.valid()
            && self.pressure.valid()
            && self.help.valid()
            && self.behavior.valid()
            && self.cursors.valid()
            && self.information.valid()
            && self.layout.valid()
            && self.preferences.valid()
            && self.properties.as_ref().is_none_or(|s| s.valid())
            && self.brushes.valid()
            && self.aids.valid()
            && self.interpolation.version == 1
            && self.actions.valid()
            && self.bindings.len() <= 256
            && self.bindings.iter().all(Binding::valid)
            && !self.tool.is_empty()
            && self.tool.len() <= 128
            && self.scratchpad.valid()
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let s: Self = read_json(path)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid workspace"));
        }
        Ok(s)
    }
    pub fn export(&self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid workspace"));
        }
        write_json(
            path,
            &crate::tool_locks::for_storage(self).map_err(io::Error::other)?,
        )
    }
    pub fn merge(
        &self,
        incoming: &Self,
        scope: Scope,
        monitors: &[Monitor],
    ) -> io::Result<(Self, Vec<String>)> {
        if !incoming.valid() || !scope.any() {
            return Err(io::Error::other("Invalid workspace or empty import scope"));
        }
        let mut next = self.clone();
        if scope.layout {
            next.layout = incoming.layout.clone();
            next.placements = incoming.placements.clone();
            next.order = incoming.order;
            let bars = next.panels.panels.map(|p| p.commands);
            // Brush shape is a tool setting, not layout.
            for (i, bar) in bars.into_iter().enumerate() {
                next.panels.panels[i] = incoming.panels.panels[i];
                next.panels.panels[i].commands = bar;
            }
            next.navigator = incoming.navigator;
            next.display = incoming.display.clone();
        }
        if scope.bars {
            next.actions.access = incoming.actions.access.clone();
            next.actions.radials = incoming.actions.radials.clone();
            next.actions.next_id = next.actions.next_id.max(incoming.actions.next_id);
            for i in 0..4 {
                next.panels.panels[i].commands = incoming.panels.panels[i].commands;
            }
        }
        if scope.keys {
            validate_bindings(&incoming.bindings)?;
            next.bindings = incoming.bindings.clone();
            next.actions.cycles = incoming.actions.cycles.clone();
            next.actions.devices = incoming.actions.devices.clone();
            next.actions.controller = incoming.actions.controller;
            next.actions.next_id = next.actions.next_id.max(incoming.actions.next_id);
        }
        if scope.other {
            next.related_colors = incoming.related_colors;
            next.color_tables = incoming.color_tables.clone();
            next.mixer = incoming.mixer.clone();
            next.color_picker = incoming.color_picker;
            next.locks = incoming.locks.clone();
            next.user_tools = incoming.user_tools.clone();
            next.eyedropper = incoming.eyedropper.clone();
            next.preferences = incoming.preferences.clone();
            next.properties = incoming.properties.clone();
            next.brushes = incoming.brushes.clone();
            next.input = incoming.input.clone();
            next.pointer = incoming.pointer;
            next.pressure = incoming.pressure;
            next.help = incoming.help.clone();
            next.behavior = incoming.behavior;
            next.cursors = incoming.cursors;
            next.information = incoming.information.clone();
            next.interface = incoming.interface;
            next.interpolation = incoming.interpolation;
            next.aids = incoming.aids;
            next.tool = incoming.tool.clone();
            next.color = incoming.color;
            next.scratchpad = incoming.scratchpad.clone();
            next.panels.brush_size = incoming.panels.brush_size;
            next.panels.brush_opacity = incoming.panels.brush_opacity;
            next.panels.brush_hardness = incoming.panels.brush_hardness;
            next.panels.brush_spacing = incoming.panels.brush_spacing;
            next.panels.brush_aspect = incoming.panels.brush_aspect;
            next.panels.brush_angle = incoming.panels.brush_angle;
        }
        if !next.valid() {
            return Err(io::Error::other(
                "Selected workspace groups have invalid or colliding collection IDs/names",
            ));
        }
        let mut reports = next.recover(monitors);
        reports.extend(next.missing_commands());
        if scope.keys {
            next.bindings.retain(|b| {
                let found = (next.actions.has_command(&b.command)
                    || crate::user_tools::command_id(&b.command).is_some())
                    && supported_key(&b.key);
                if !found {
                    reports.push(format!(
                        "Unavailable command/key: {} ({}) — skipped",
                        b.command,
                        b.label()
                    ));
                }
                found
            });
        }
        Ok((next, reports))
    }
    pub fn recover(&mut self, monitors: &[Monitor]) -> Vec<String> {
        let mut reports = Vec::new();
        for (i, p) in self.placements.iter_mut().enumerate() {
            if let Some(reason) = p.recover(monitors) {
                reports.push(format!("{}: {reason}", panels::Kind::ALL[i].label()));
            }
        }
        if !BrushTip::ALL.iter().any(|t| t.name() == self.tool) {
            reports.push(format!("Unavailable tool {} — using Pen", self.tool));
            self.tool = "Pen".into();
        }
        reports
    }
}
pub fn supported_key(key: &str) -> bool {
    (key.len() == 1
        && (key.as_bytes()[0].is_ascii_uppercase() || key.as_bytes()[0].is_ascii_digit()))
        || [
            "ArrowUp",
            "[",
            "]",
            "ArrowRight",
            "ArrowDown",
            "ArrowLeft",
            "Space",
            "Escape",
            "Enter",
            "Tab",
            "Delete",
            "F1",
            "F2",
            "F3",
            "F4",
            "F5",
            "F6",
            "F7",
            "F8",
            "F9",
            "F10",
            "F11",
            "F12",
        ]
        .contains(&key)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    pub name: String,
    pub snapshot: Snapshot,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub version: u32,
    pub current: Snapshot,
    pub active: Option<String>,
    pub named: Vec<Named>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            current: Default::default(),
            active: None,
            named: Vec::new(),
        }
    }
}
impl Store {
    fn valid(&self) -> bool {
        let mut names = HashSet::new();
        self.version == 1
            && self.current.valid()
            && self.named.len() <= 32
            && self.named.iter().all(|n| {
                valid_name(&n.name) && names.insert(n.name.to_lowercase()) && n.snapshot.valid()
            })
            && self
                .active
                .as_ref()
                .is_none_or(|a| self.named.iter().any(|n| &n.name == a))
    }
    pub fn load(storage: &UserStorage) -> io::Result<Option<Self>> {
        let path = storage.root().join("settings/workspaces.json");
        if !path.exists() {
            return Ok(None);
        }
        let s: Self = read_json(&path)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid workspace collection"));
        }
        Ok(Some(s))
    }
    pub fn save(&self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid workspace collection"));
        }
        let path = storage.root().join("settings/workspaces.json");
        if path.exists() && Self::load(storage).is_err() {
            write_bytes(
                &storage.root().join("settings/workspaces.damaged.json"),
                &read_bytes(&path)?,
            )?;
        }
        let mut disk = self.clone();
        disk.current = crate::tool_locks::for_storage(&disk.current).map_err(io::Error::other)?;
        for n in &mut disk.named {
            n.snapshot = crate::tool_locks::for_storage(&n.snapshot).map_err(io::Error::other)?;
        }
        write_json(&path, &disk)
    }
    pub fn save_named(&mut self, name: &str, snapshot: Snapshot) -> io::Result<()> {
        if !valid_name(name)
            || !snapshot.valid()
            || self.named.len() >= 32
            || self.named.iter().any(|n| n.name.eq_ignore_ascii_case(name))
        {
            return Err(io::Error::other(
                "Workspace name invalid, already used, or collection full (32)",
            ));
        }
        self.named.push(Named {
            name: name.into(),
            snapshot,
        });
        self.active = Some(name.into());
        Ok(())
    }
    // One settings transaction needs both scope and the optional independent named backup.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        storage: Option<&UserStorage>,
        current: Snapshot,
        incoming: &Snapshot,
        scope: Scope,
        active: Option<String>,
        save_unsaved: Option<&str>,
        monitors: &[Monitor],
    ) -> io::Result<Vec<String>> {
        let (next, reports) = current.merge(incoming, scope, monitors)?;
        let mut staged = self.clone();
        if let Some(name) = save_unsaved {
            staged.save_named(name, current.clone())?;
        }
        staged.current = next;
        staged.active = active;
        if !staged.valid() {
            return Err(io::Error::other("Invalid staged workspace"));
        }
        if let Some(storage) = storage {
            current.export(&storage.root().join("settings/workspace-before-apply.json"))?;
            staged.save(storage)?;
        }
        *self = staged;
        Ok(reports)
    }
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}
fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    if fs::metadata(path)?.len() > LIMIT {
        return Err(io::Error::other("Workspace exceeds local size limit"));
    }
    fs::read(path)
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&read_bytes(path)?).map_err(io::Error::other)
}
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    write_bytes(path, &serde_json::to_vec(value).map_err(io::Error::other)?)
}
fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("Workspace exceeds local size limit"));
    }
    let temp = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = result.and_then(|_| fs::rename(&temp, path)) {
        let _ = fs::remove_file(temp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_import_conflicts_unknown_items_and_monitor_recovery() {
        let before = Snapshot::default();
        let mut imported = before.clone();
        imported.placements[0].dock = Dock::Floating;
        imported.placements[0].monitor = Some("removed".into());
        imported.placements[0].position = [-9000.0, 8000.0];
        imported.panels.panels[0].visible = false;
        imported.panels.panels[0].commands = panels::Bar::Bottom;
        imported.bindings[0].key = "F2".into();
        imported.tool = "removed-tip".into();
        let screen = Monitor {
            key: "main".into(),
            index: 0,
            position: [-1920, 0],
            size: [1920, 1080],
            scale: 1.5,
            primary: true,
        };
        let (layout, reasons) = before
            .merge(
                &imported,
                Scope {
                    layout: true,
                    ..Default::default()
                },
                std::slice::from_ref(&screen),
            )
            .unwrap();
        assert!(!reasons.is_empty());
        assert_eq!(layout.bindings, before.bindings);
        assert_eq!(
            layout.panels.panels[0].commands,
            before.panels.panels[0].commands
        );
        assert_eq!(layout.placements[0].monitor.as_deref(), Some("main"));
        let (keys, _) = before
            .merge(
                &imported,
                Scope {
                    keys: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(keys.placements, before.placements);
        assert_eq!(keys.bindings, imported.bindings);
        imported.bindings.push(imported.bindings[0].clone());
        assert!(before.merge(&imported, Scope::ALL, &[]).is_err());
        imported.bindings.pop();
        imported.bindings[0].command = "missing-command".into();
        let (next, reports) = before.merge(&imported, Scope::ALL, &[screen]).unwrap();
        assert_eq!(next.tool, "Pen");
        assert_eq!(next.bindings.len(), before.bindings.len() - 1);
        assert!(reports.iter().any(|s| s.contains("missing-command")));
    }
    #[test]
    fn atomic_store_backup_and_failed_application_preserve_every_setting() {
        let root = crate::test_support::TempDir::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let mut store = Store::default();
        let mut changed = store.current.clone();
        changed.tool = "Wash".into();
        changed.scratchpad.stroke(
            [8.0, 8.0],
            [80.0, 60.0],
            8.0,
            BrushTip::Pen,
            [30, 90, 100, 255],
        );
        store.save_named("Painting", changed.clone()).unwrap();
        store
            .apply(
                Some(&storage),
                store.current.clone(),
                &changed,
                Scope::ALL,
                Some("Painting".into()),
                Some("Before"),
                &[],
            )
            .unwrap();
        assert_eq!(
            Snapshot::read(&storage.root().join("settings/workspace-before-apply.json")).unwrap(),
            Snapshot::default()
        );
        assert_eq!(Store::load(&storage).unwrap().unwrap(), store);
        let old = store.clone();
        fs::create_dir(storage.root().join("settings/workspaces.json.write.tmp")).unwrap();
        assert!(
            store
                .apply(
                    Some(&storage),
                    store.current.clone(),
                    &Snapshot::default(),
                    Scope::ALL,
                    None,
                    None,
                    &[]
                )
                .is_err()
        );
        assert_eq!(store, old);
        fs::remove_dir(storage.root().join("settings/workspaces.json.write.tmp")).unwrap();
        fs::write(storage.root().join("settings/workspaces.json"), b"damaged").unwrap();
        assert!(Store::load(&storage).is_err());
        store.save(&storage).unwrap();
        assert_eq!(
            fs::read(storage.root().join("settings/workspaces.damaged.json")).unwrap(),
            b"damaged"
        );
        assert!(store.save_named("painting", changed).is_err());
    }
}
