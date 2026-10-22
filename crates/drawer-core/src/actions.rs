//! User-assigned local commands. References remain stable when lists are renamed.
use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    io,
};

pub const EXTRA_COMMANDS: [&str; 149] = [
    "gaussian-blur",
    "tone-curves",
    "gradient-map",
    "scale-rotate",
    "raster-trace",
    "vector-remove-short",
    "vector-scope-tools",
    "vector-copy-strokes",
    "vector-cut-strokes",
    "vector-copy-spans",
    "vector-cut-spans",
    "vector-paste",
    "vector-select-overlap",
    "vector-select-contained",
    "vector-selection-undo",
    "vector-selection-redo",
    "vector-pressure-reset-selected",
    "vector-pressure-reset-layer",
    "svg-import",
    "svg-export",
    "svg-copy",
    "vector-tip-tools",
    "vector-path-overlay",
    "vector-refine-tools",
    "vector-push-tool",
    "vector-redraw-tool",
    "opacity-up",
    "opacity-down",
    "opacity-0",
    "opacity-10",
    "opacity-20",
    "opacity-30",
    "opacity-40",
    "opacity-50",
    "opacity-60",
    "opacity-70",
    "opacity-80",
    "opacity-90",
    "opacity-100",
    "connect-line",
    "Eyedropper",
    "Move",
    "brush-grow",
    "brush-shrink",
    "zoom-in",
    "zoom-out",
    "screen-mode",
    "view-center",
    "view-reset",
    "new-canvas",
    "test-painting",
    "help",
    "primary-color",
    "secondary-color",
    "swap-colors",
    "transparent-color",
    "screen-eyedropper",
    "select-rectangle",
    "select-lasso",
    "select-ellipse",
    "select-polygon",
    "selection-clear",
    "selection-invert",
    "selection-reselect",
    "selection-tools",
    "selection-save",
    "selection-expand",
    "selection-shrink",
    "selection-feather",
    "duplicate-page",
    "move-selection-boundary",
    "move-selected-pixels",
    "copy-selection",
    "paste-selection",
    "pick-next-layer",
    "bucket-tool",
    "fill-selection",
    "lasso-fill-tool",
    "enclose-fill-tool",
    "fill-small-gaps",
    "ruler-tools",
    "special-ruler-tools",
    "ruler-follow",
    "ruler-move-layer",
    "ruler-snap",
    "quick-mask",
    "color-range",
    "selection-border",
    "gradient-tool",
    "shape-tool",
    "vector-line-tool",
    "vector-curve-tool",
    "vector-point-tool",
    "vector-pressure-tool",
    "vector-color-tool",
    "vector-edit-tools",
    "vector-stroke-tool",
    "vector-delete-strokes",
    "vector-eraser-tool",
    "vector-node-add-tool",
    "vector-node-delete-tool",
    "vector-node-corner-tool",
    "vector-node-smooth-tool",
    "align-objects",
    "manage-layers",
    "layer-properties",
    "layer-clipping",
    "layer-blend",
    "merge-layers",
    "merge-visible-copy",
    "delete-empty-layers",
    "delete-hidden-layers",
    "layer-folders",
    "adjustment-layer",
    "layer-search",
    "layer-roles",
    "layer-appearance",
    "material-layer-new",
    "linked-file-layer",
    "refresh-file-layers",
    "raster-conversion",
    "layer-comps",
    "layer-masks",
    "transfer-down",
    "layer-groups",
    "paper-background",
    "document-color",
    "material-layer-edit",
    "material-transform",
    "layer-defaults",
    "import-image-layer",
    "new-raster-layer",
    "new-vector-layer",
    "duplicate-layer",
    "delete-layer",
    "layer-selection-tools",
    "contour-selection",
    "selection-shortcuts",
    "delete-outside-selection",
    "animation-output-frame",
    "vector-selection-clear",
    "selection-outline",
    "alpha-selection-replace",
    "alpha-selection-add",
    "alpha-selection-subtract",
    "alpha-selection-intersect",
    "wand-tool",
    "selection-brush-tool",
    "selection-erase-tool",
];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    Command(String),
    Tool(String),
    Color([u8; 4]),
    Separator,
}
impl Item {
    pub fn label(&self) -> String {
        match self {
            Self::Command(s) | Self::Tool(s) => BrushTip::ALL
                .into_iter()
                .find(|t| t.name() == s)
                .map_or_else(|| s.clone(), |t| t.label().into()),
            Self::Color(c) => format!("#{:02X}{:02X}{:02X} · alpha {}", c[0], c[1], c[2], c[3]),
            Self::Separator => "Separator".into(),
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Command(s) | Self::Tool(s) => reference(s),
            _ => true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    Tiles,
    List,
    Popup,
}
impl Display {
    pub const ALL: [Self; 3] = [Self::Tiles, Self::List, Self::Popup];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetKind {
    QuickAccess,
    CommandBar,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessSet {
    pub id: u64,
    pub name: String,
    pub kind: SetKind,
    pub display: Display,
    pub ctrl_drag: bool,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cycle {
    pub id: u64,
    pub name: String,
    pub tools: Vec<String>,
}
impl Cycle {
    pub fn next(&self, current: &str, reverse: bool) -> io::Result<BrushTip> {
        let tools: Vec<_> = self
            .tools
            .iter()
            .filter_map(|s| BrushTip::ALL.into_iter().find(|t| t.name() == s))
            .collect();
        if tools.is_empty() {
            return Err(io::Error::other(
                "Tool cycle is empty or every referenced tool is unavailable",
            ));
        }
        let n = match tools.iter().position(|t| t.name() == current) {
            Some(n) if reverse => (n + tools.len() - 1) % tools.len(),
            Some(n) => (n + 1) % tools.len(),
            None if reverse => tools.len() - 1,
            None => 0,
        };
        Ok(tools[n])
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Radial {
    pub id: u64,
    pub name: String,
    pub directions: u8,
    pub slots: Vec<Option<String>>,
}
impl Radial {
    pub fn resize(&mut self, directions: u8) {
        if [4, 8].contains(&directions) {
            self.directions = directions;
            self.slots.resize(directions as usize, None);
        }
    }
    pub fn slot_at(&self, delta: [f32; 2]) -> Option<usize> {
        let radius = delta[0].hypot(delta[1]);
        if !(30.0..=120.0).contains(&radius) {
            return None;
        }
        let angle = (delta[1].atan2(delta[0]) + std::f32::consts::FRAC_PI_2)
            .rem_euclid(std::f32::consts::TAU);
        Some(
            (angle / std::f32::consts::TAU * self.directions as f32).round() as usize
                % self.directions as usize,
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceBinding {
    pub device: String,
    pub action: String,
    pub command: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub next_id: u64,
    pub cycles: Vec<Cycle>,
    pub access: Vec<AccessSet>,
    pub radials: Vec<Radial>,
    pub devices: Vec<DeviceBinding>,
    #[serde(default)]
    pub controller: crate::controller::Settings,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 5,
            cycles: vec![Cycle {
                id: 1,
                name: "Drawing tools".into(),
                tools: BrushTip::ALL.iter().map(|t| t.name().into()).collect(),
            }],
            access: vec![
                AccessSet {
                    id: 2,
                    name: "Drawing".into(),
                    kind: SetKind::QuickAccess,
                    display: Display::Tiles,
                    ctrl_drag: false,
                    items: vec![
                        Item::Command("undo".into()),
                        Item::Command("redo".into()),
                        Item::Tool("Pen".into()),
                        Item::Tool("Eraser".into()),
                        Item::Separator,
                        Item::Color([0, 0, 0, 255]),
                    ],
                },
                AccessSet {
                    id: 3,
                    name: "Document".into(),
                    kind: SetKind::CommandBar,
                    display: Display::Tiles,
                    ctrl_drag: false,
                    items: vec![
                        Item::Command("save".into()),
                        Item::Command("undo".into()),
                        Item::Command("redo".into()),
                    ],
                },
            ],
            radials: vec![Radial {
                id: 4,
                name: "Drawing tools".into(),
                directions: 4,
                slots: BrushTip::ALL
                    .iter()
                    .take(4)
                    .map(|t| Some(t.name().into()))
                    .collect(),
            }],
            devices: Vec::new(),
            controller: Default::default(),
        }
    }
}
fn reference(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}
fn name(s: &str) -> bool {
    reference(s) && !s.trim().is_empty()
}
impl Settings {
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut devices = HashSet::new();
        self.version == 1
            && self.controller.valid()
            && self.next_id > 0
            && self.cycles.len() <= 32
            && self.access.len() <= 32
            && self.radials.len() <= 32
            && self.devices.len() <= 128
            && self.cycles.iter().all(|c| {
                c.id > 0
                    && c.id < self.next_id
                    && ids.insert(c.id)
                    && name(&c.name)
                    && names.insert((0, c.name.to_lowercase()))
                    && c.tools.len() <= 128
                    && c.tools.iter().all(|t| reference(t))
                    && c.tools.iter().collect::<HashSet<_>>().len() == c.tools.len()
            })
            && self.access.iter().all(|s| {
                s.id > 0
                    && s.id < self.next_id
                    && ids.insert(s.id)
                    && name(&s.name)
                    && names.insert((
                        if s.kind == SetKind::QuickAccess { 1 } else { 2 },
                        s.name.to_lowercase(),
                    ))
                    && s.items.len() <= 128
                    && s.items.iter().all(Item::valid)
            })
            && self.radials.iter().all(|r| {
                r.id > 0
                    && r.id < self.next_id
                    && ids.insert(r.id)
                    && name(&r.name)
                    && names.insert((3, r.name.to_lowercase()))
                    && [4, 8].contains(&r.directions)
                    && r.slots.len() == r.directions as usize
                    && r.slots.iter().flatten().all(|s| reference(s))
            })
            && self.devices.iter().all(|d| {
                reference(&d.device)
                    && reference(&d.action)
                    && reference(&d.command)
                    && devices.insert((&d.device, &d.action))
            })
    }
    pub fn allocate_id(&mut self) -> io::Result<u64> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Command collection ID limit"))?;
        Ok(id)
    }
    pub fn commands(&self) -> Vec<String> {
        let mut c = crate::workspaces::COMMANDS
            .iter()
            .chain(EXTRA_COMMANDS.iter())
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>();
        for cycle in &self.cycles {
            c.push(format!("cycle-next:{}", cycle.id));
            c.push(format!("cycle-prev:{}", cycle.id));
        }
        for set in &self.access {
            c.push(format!("quick:{}", set.id));
        }
        for radial in &self.radials {
            c.push(format!("radial:{}", radial.id));
        }
        c
    }
    pub fn has_command(&self, command: &str) -> bool {
        self.commands().iter().any(|c| c == command)
    }
    pub fn label(&self, command: &str) -> String {
        if let Some((prefix, id)) = command.split_once(':') {
            let id = id.parse::<u64>().ok();
            if ["cycle-next", "cycle-prev"].contains(&prefix)
                && let Some(c) = self.cycles.iter().find(|c| Some(c.id) == id)
            {
                return format!(
                    "{} tool — {}",
                    if prefix == "cycle-prev" {
                        "Previous"
                    } else {
                        "Next"
                    },
                    c.name
                );
            }
            if prefix == "quick"
                && let Some(s) = self.access.iter().find(|s| Some(s.id) == id)
            {
                return format!("Open quick access — {}", s.name);
            }
            if prefix == "radial"
                && let Some(r) = self.radials.iter().find(|r| Some(r.id) == id)
            {
                return format!("Open radial menu — {}", r.name);
            }
        }
        match command {
            "Wash" => "Airbrush",
            "PixelPen" => "Pixel pen",
            "save" => "Save",
            "gradient-tool" => "Gradient and freeform gradient",
            "vector-line-tool" => "Draw an editable vector line with clicked control points",
            "vector-point-tool" => "Move selected vector control points on the canvas",
            "vector-refine-tools" => "Join, simplify, split, convert and restyle vector strokes",
            "raster-trace" => {
                "Trace raster pixels into a new local vector layer while preserving the source"
            }
            "vector-remove-short" => {
                "Preview removal of short selected spans or strokes across eligible vector layers"
            }
            "vector-tip-tools" => "Change vector brush tips while preserving paths and pressure",
            "vector-scope-tools" => {
                "Transform selected vector points or strokes around a pivot and duplicate them"
            }
            "vector-copy-strokes" => {
                "Copy whole vector strokes containing selected points with a raster fallback"
            }
            "vector-cut-strokes" => "Cut whole selected vector strokes after clipboard success",
            "vector-copy-spans" => "Copy spans between adjacent selected vector controls",
            "vector-cut-spans" => "Cut selected vector spans after clipboard success",
            "vector-paste" => "Paste editable vectors with fresh object identifiers",
            "vector-select-overlap" => {
                "Select centerlines overlapping the area, including boundary contact"
            }
            "vector-select-contained" => {
                "Select centerlines entirely inside the area, including boundaries"
            }
            "vector-selection-undo" => "Undo session vector selection without changing artwork",
            "vector-selection-redo" => "Redo session vector selection without changing artwork",
            "vector-pressure-reset-selected" => {
                "Preview resetting selected vector points or strokes to pressure 1.00"
            }
            "vector-pressure-reset-layer" => {
                "Preview resetting all editable pressure in the current vector layer"
            }
            "svg-import" => "Import local SVG paths, paints and image snapshots",
            "svg-export" => "Export SVG paths with local conversion notices and embedded images",
            "svg-copy" => "Copy SVG text using the same conversion policy as SVG export",
            "vector-path-overlay" => "Show or hide vector path overlays without changing the page",
            "vector-push-tool" => "Locally push vector outlines with endpoint constraints",
            "vector-redraw-tool" => "Redraw a selected vector span while preserving the rest",
            "vector-stroke-tool" => "Select whole vector strokes on the canvas",
            "vector-node-add-tool" => "Add a cubic node inside a vector segment",
            "vector-node-delete-tool" => {
                "Delete a vector node while retaining minimum path controls"
            }
            "vector-node-corner-tool" => "Switch a vector node to independent corner tangents",
            "vector-node-smooth-tool" => {
                "Switch a vector node to a shared smooth tangent direction"
            }
            "vector-delete-strokes" => "Delete selected whole vector strokes as one Undo",
            "vector-eraser-tool" => "Erase touched spans, intersections or whole vector strokes",
            "vector-pressure-tool" => "Edit vector point pressure with a horizontal drag",
            "vector-color-tool" => "Recolor vector strokes with the current drawing color",
            "vector-edit-tools" => {
                "Edit vector point position, pressure, width and color numerically"
            }
            "vector-curve-tool" => "Draw an editable vector curve with clicked control points",
            "shape-tool" => "Draw shape",
            "align-objects" => "Align and distribute objects",
            "manage-layers" => "Manage local layers",
            "adjustment-layer" => "Nondestructive brightness, contrast, hue and saturation layers",
            "layer-search" => "Search, filter and select local layers",
            "document-color" => "Document channel precision and RGB ICC conversion preview",
            "paper-background" => "Output paper layer and display-only transparency colors",
            "layer-groups" => "Linked layer movement, transforms, ink filters and solo visibility",
            "transfer-down" => "Transfer selected content down and retain the upper layer",
            "layer-masks" => "Invert, copy, create, link and apply layer masks",
            "layer-comps" => "Save and switch layer visibility, opacity and blend comps",
            "raster-conversion" => {
                "Rasterize sources, convert brightness to opacity, or recolor selected content"
            }
            "refresh-file-layers" => "Review updates for all local linked sources",
            "linked-file-layer" => "Local file layers: create, refresh, relink and convert content",
            "material-layer-new" => "Create an editable solid, gradient or local image material",
            "material-transform" => "Transform an image material, linked source or editable vector",
            "material-layer-edit" => "Edit the selected material and preserve its source",
            "layer-roles" => "Reference and draft layer roles; rendered output policy",
            "layer-appearance" => "Layer display color, border and watercolor edge",
            "layer-defaults" => "Choose and save defaults for new layers of the same kind",
            "layer-folders" => "Layer folders, wrapping and grayscale masks",
            "merge-visible-copy" => "Copy the visible page to a new editable raster",
            "delete-empty-layers" => "Preview and delete empty layers",
            "delete-hidden-layers" => "Preview and delete hidden layers and review references",
            "merge-layers" => "Merge selected, merge down or flatten page",
            "layer-blend" => "Layer blend modes: Normal, Multiply, Screen, Overlay",
            "layer-clipping" => "Layer alpha protection, clipping and folder solo",
            "layer-properties" => "Layer properties and identification colors",
            "import-image-layer" => "Import local image as layer",
            "new-raster-layer" => "Create raster layer",
            "new-vector-layer" => "Create vector layer",
            "duplicate-layer" => "Duplicate selected layer",
            "delete-layer" => "Delete selected layer",
            "layer-selection-tools" => "Layer alpha and vector editing selections",
            "vector-selection-clear" => "Clear vector editing selection",
            "selection-outline" => "Show/hide area selection outline",
            "alpha-selection-replace" => "Select from layer alpha",
            "alpha-selection-add" => "Add layer alpha to selection",
            "alpha-selection-subtract" => "Subtract layer alpha from selection",
            "alpha-selection-intersect" => "Intersect selection with layer alpha",
            "save-as" => "Save as",
            "undo" => "Undo",
            "redo" => "Redo",
            "view-center" => "Center view",
            "view-reset" => "Reset view",
            "screen-mode" => "Simple/full screen",
            "new-canvas" => "New canvas",
            "test-painting" => "Popup test painting",
            "help" => "Local help",
            "primary-color" => "Use primary drawing color",
            "secondary-color" => "Use secondary drawing color",
            "swap-colors" => "Swap primary and secondary colors",
            "select-rectangle" => "Rectangle selection",
            "select-lasso" => "Lasso selection",
            "select-ellipse" => "Ellipse selection",
            "select-polygon" => "Polygon selection",
            "selection-clear" => "Deselect",
            "selection-invert" => "Invert page selection",
            "selection-reselect" => "Reselect previous page selection",
            "selection-tools" => "Page selections",
            "selection-save" => "Save named page selection",
            "duplicate-page" => "Duplicate page as document",
            "screen-eyedropper" => "Sample entire screen (display RGB)",
            "transparent-color" => "Toggle transparent shape eraser",
            "brush-grow" => "Increase brush size",
            "brush-shrink" => "Decrease brush size",
            "zoom-in" => "Zoom in",
            "zoom-out" => "Zoom out",
            "connect-line" => "Connect last stroke to next click",
            "opacity-up" => "Increase brush opacity",
            "opacity-down" => "Decrease brush opacity",
            c if c.starts_with("opacity-") => return format!("Brush opacity — {}%", &c[8..]),
            other => other,
        }
        .into()
    }
    pub fn missing(&self) -> Vec<String> {
        let mut missing = Vec::new();
        for cycle in &self.cycles {
            for tool in &cycle.tools {
                if !BrushTip::ALL.iter().any(|t| t.name() == tool) {
                    missing.push(format!(
                        "{}: missing tool {tool} — skipped during cycling",
                        cycle.name
                    ));
                }
            }
        }
        for set in &self.access {
            for item in &set.items {
                match item {
                    Item::Command(c) if !self.has_command(c) => {
                        missing.push(format!("{}: missing command {c}", set.name))
                    }
                    Item::Tool(t) if !BrushTip::ALL.iter().any(|v| v.name() == t) => {
                        missing.push(format!("{}: missing tool {t}", set.name))
                    }
                    _ => {}
                }
            }
        }
        for radial in &self.radials {
            for c in radial.slots.iter().flatten() {
                if !self.has_command(c) {
                    missing.push(format!("{}: missing command {c}", radial.name));
                }
            }
        }
        for d in &self.devices {
            if !self.has_command(&d.command) {
                missing.push(format!(
                    "{} / {}: missing command {}",
                    d.device, d.action, d.command
                ));
            }
            if !((d.device == "pointer" && ["extra-1", "extra-2"].contains(&d.action.as_str()))
                || (d.device == crate::controller::DEVICE
                    && crate::controller::ACTIONS.contains(&d.action.as_str())))
            {
                missing.push(format!("{} / {}: adapter action unavailable here; test the command with a keyboard assignment",d.device,d.action));
            }
        }
        missing
    }
}
/// Idempotent named device events; disconnect starts a fresh sequence.
#[derive(Default)]
pub struct DeviceEvents {
    seen: VecDeque<(String, u64)>,
}
impl DeviceEvents {
    pub fn dispatch<'a>(
        &mut self,
        settings: &'a Settings,
        device: &str,
        action: &str,
        token: u64,
    ) -> Option<&'a str> {
        if self.seen.iter().any(|(d, t)| d == device && *t == token) {
            return None;
        }
        self.seen.push_back((device.into(), token));
        if self.seen.len() > 256 {
            self.seen.pop_front();
        }
        settings
            .devices
            .iter()
            .find(|d| d.device == device && d.action == action)
            .map(|d| d.command.as_str())
    }
    pub fn disconnect(&mut self, device: &str) {
        self.seen.retain(|(d, _)| d != device);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycles_stable_ids_duplicates_deleted_tools_endpoints_and_empty() {
        let mut settings = Settings::default();
        let c = &mut settings.cycles[0];
        assert_eq!(c.next("Wash", false).unwrap(), BrushTip::Marker);
        assert_eq!(c.next("Pen", true).unwrap(), BrushTip::PixelPen);
        c.tools = vec!["missing-preset".into(), "Pen".into()];
        assert_eq!(c.next("Pen", false).unwrap(), BrushTip::Pen);
        assert!(settings.valid());
        assert!(!settings.missing().is_empty());
        settings.cycles[0].tools.push("Pen".into());
        assert!(!settings.valid());
        settings.cycles[0].tools.clear();
        assert!(settings.cycles[0].next("Pen", false).is_err());
        assert!(settings.valid());
        let mut copy = settings.cycles[0].clone();
        copy.id = settings.allocate_id().unwrap();
        settings.cycles.push(copy);
        assert!(!settings.valid());
        settings.cycles[1].name = "Another".into();
        assert!(settings.valid());
        assert!(settings.has_command("cycle-next:5"));
        settings.cycles.remove(0);
        assert!(!settings.has_command("cycle-next:1"));
        assert!(settings.has_command("cycle-next:5"));
    }
    #[test]
    fn radial_mouse_keyboard_slots_boundaries_empty_and_missing_sets() {
        let mut settings = Settings::default();
        let r = &mut settings.radials[0];
        r.resize(8);
        for n in 0..8 {
            let angle = n as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
            assert_eq!(r.slot_at([angle.cos() * 80.0, angle.sin() * 80.0]), Some(n));
        }
        assert_eq!(r.slot_at([0.0, 0.0]), None);
        assert_eq!(r.slot_at([200.0, 0.0]), None);
        assert!(r.slots[7].is_none());
        r.resize(4);
        assert_eq!(r.slots.len(), 4);
        assert!(settings.valid());
        settings.access.clear();
        settings.radials.clear();
        assert!(settings.valid());
        assert!(!settings.has_command("quick:2"));
    }
    #[test]
    fn device_assignments_deduplicate_events_disconnect_remove_and_unknown_actions() {
        let mut settings = Settings::default();
        settings.devices.push(DeviceBinding {
            device: "pointer".into(),
            action: "extra-1".into(),
            command: "cycle-next:1".into(),
        });
        let mut events = DeviceEvents::default();
        assert_eq!(
            events.dispatch(&settings, "pointer", "extra-1", 1),
            Some("cycle-next:1")
        );
        assert_eq!(events.dispatch(&settings, "pointer", "extra-1", 1), None);
        assert_eq!(events.dispatch(&settings, "pointer", "unknown", 2), None);
        events.disconnect("pointer");
        assert_eq!(
            events.dispatch(&settings, "pointer", "extra-1", 1),
            Some("cycle-next:1")
        );
        settings.devices.push(settings.devices[0].clone());
        assert!(!settings.valid());
        settings.devices.clear();
        assert_eq!(events.dispatch(&settings, "pointer", "extra-1", 3), None);
    }
}
