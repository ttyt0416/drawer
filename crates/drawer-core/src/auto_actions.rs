//! Local, bounded command sequences. Never dispatches arbitrary editor, shell or network commands.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Edit,
    Buttons,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Canvas {
    FlipHorizontal,
    FlipVertical,
    Clockwise,
    Counterclockwise,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRef {
    pub document: u64,
    pub id: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", deny_unknown_fields)]
pub enum Operation {
    Canvas(Canvas),
    Transform {
        layer: usize,
        vector: bool,
        settings: crate::vector_scope::Transform,
        filter: crate::interpolation::Filter,
    },
    Scroll {
        document: [f64; 2],
    },
    Dpi(f64),
    NewRaster {
        name: String,
    },
    RenameRaster {
        layer: usize,
        name: String,
    },
    Opacity {
        layer: usize,
        percent: u8,
    },
    Fill {
        layer: usize,
        rgba: [u8; 4],
        erase: bool,
        alpha_lock: bool,
    },
    /// Preserved for review only; even a known editor command cannot bypass the whitelist.
    Unsupported {
        command: String,
        tool: Option<ToolRef>,
    },
}
impl Operation {
    pub fn valid(&self) -> bool {
        match self {
            Self::Dpi(n) => n.is_finite() && (1.0..=9600.0).contains(n),
            Self::NewRaster { name } => name_valid(name),
            Self::RenameRaster { layer, name } => *layer < 256 && name_valid(name),
            Self::Opacity { layer, percent } => *layer < 256 && *percent <= 100,
            Self::Fill { layer, .. } => *layer < 256,
            Self::Unsupported { command, tool } => {
                name_valid(command) && tool.as_ref().is_none_or(|t| t.id > 0 && t.document > 0)
            }
            Self::Transform {
                layer, settings, ..
            } => *layer < 256 && !settings.duplicate && settings.matrix().is_ok(),
            Self::Scroll { document } => document.iter().all(|v| v.is_finite() && v.abs() <= 1e6),
            Self::Canvas(_) => true,
        }
    }
    pub fn issue(&self, document: Option<u64>, tools: &[u64]) -> Option<String> {
        match self {
            Self::Unsupported { command, tool } => Some(if let Some(t) = tool {
                if document != Some(t.document) {
                    format!(
                        "{command}: tool {} belongs to another document {}",
                        t.id, t.document
                    )
                } else if !tools.contains(&t.id) {
                    format!("{command}: missing tool {}", t.id)
                } else {
                    format!("{command}: tool commands cannot be recorded or replayed")
                }
            } else {
                format!("{command}: unsupported / non-recordable command")
            }),
            _ => None,
        }
    }
}
fn name_valid(n: &str) -> bool {
    !n.trim().is_empty() && n.chars().count() <= 64 && !n.chars().any(char::is_control)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: u64,
    pub enabled: bool,
    pub operation: Operation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: u64,
    pub name: String,
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    pub id: u64,
    pub name: String,
    pub actions: Vec<Action>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub next_id: u64,
    pub mode: Mode,
    pub sets: Vec<Set>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            mode: Mode::Edit,
            sets: vec![],
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        let mut count = 0usize;
        self.version == 1
            && self.next_id > 0
            && self.next_id < u64::MAX
            && self.sets.len() <= 32
            && self.sets.iter().all(|s| {
                ids.insert(s.id)
                    && s.id > 0
                    && s.id < self.next_id
                    && name_valid(&s.name)
                    && s.actions.len() <= 64
                    && s.actions.iter().all(|a| {
                        ids.insert(a.id)
                            && a.id > 0
                            && a.id < self.next_id
                            && name_valid(&a.name)
                            && a.steps.len() <= 256
                            && a.steps.iter().all(|p| {
                                count += 1;
                                count <= 2048
                                    && ids.insert(p.id)
                                    && p.id > 0
                                    && p.id < self.next_id
                                    && p.operation.valid()
                            })
                    })
            })
    }
    fn allocate(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|n| *n < u64::MAX)
            .ok_or("Auto action identity limit")?;
        Ok(id)
    }
    fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if !self.valid() {
            return Err("Invalid auto action settings".into());
        }
        let mut next = self.clone();
        let out = f(&mut next)?;
        if !next.valid() {
            return Err("Auto action name, ID or capacity limit".into());
        }
        *self = next;
        Ok(out)
    }
    pub fn action(&self, id: u64) -> Option<&Action> {
        self.sets
            .iter()
            .flat_map(|s| &s.actions)
            .find(|a| a.id == id)
    }
    pub fn action_mut(&mut self, id: u64) -> Result<&mut Action, String> {
        self.sets
            .iter_mut()
            .flat_map(|s| &mut s.actions)
            .find(|a| a.id == id)
            .ok_or("Select an action".into())
    }
    pub fn add_set(&mut self, name: String) -> Result<u64, String> {
        self.transaction(|s| {
            let id = s.allocate()?;
            s.sets.push(Set {
                id,
                name,
                actions: vec![],
            });
            Ok(id)
        })
    }
    pub fn add_action(&mut self, set: u64, name: String) -> Result<u64, String> {
        self.transaction(|s| {
            let id = s.allocate()?;
            s.sets
                .iter_mut()
                .find(|v| v.id == set)
                .ok_or("Select a set")?
                .actions
                .push(Action {
                    id,
                    name,
                    steps: vec![],
                });
            Ok(id)
        })
    }
    pub fn append(&mut self, action: u64, operation: Operation) -> Result<u64, String> {
        self.transaction(|s| {
            let id = s.allocate()?;
            s.action_mut(action)?.steps.push(Step {
                id,
                enabled: true,
                operation,
            });
            Ok(id)
        })
    }
    pub fn duplicate_set(&mut self, id: u64) -> Result<u64, String> {
        let set = self
            .sets
            .iter()
            .find(|s| s.id == id)
            .ok_or("Select a set")?
            .clone();
        self.transaction(|s| {
            let mut set = set;
            set.id = s.allocate()?;
            for a in &mut set.actions {
                a.id = s.allocate()?;
                for p in &mut a.steps {
                    p.id = s.allocate()?;
                }
            }
            let id = set.id;
            s.sets.push(set);
            Ok(id)
        })
    }
    pub fn duplicate_action(&mut self, id: u64) -> Result<u64, String> {
        self.transaction(|s| {
            let index = s
                .sets
                .iter()
                .position(|v| v.actions.iter().any(|a| a.id == id))
                .ok_or("Select an action")?;
            let mut a = s.action(id).unwrap().clone();
            a.id = s.allocate()?;
            for p in &mut a.steps {
                p.id = s.allocate()?;
            }
            let id = a.id;
            s.sets[index].actions.push(a);
            Ok(id)
        })
    }
    pub fn duplicate_step(&mut self, action: u64, index: usize) -> Result<(), String> {
        self.transaction(|s| {
            let mut p = s
                .action(action)
                .and_then(|a| a.steps.get(index))
                .ok_or("Select a step")?
                .clone();
            p.id = s.allocate()?;
            s.action_mut(action)?.steps.insert(index + 1, p);
            Ok(())
        })
    }
    pub fn import(&self, source: &Self) -> Result<Self, String> {
        if !source.valid() || source.sets.is_empty() {
            return Err("Invalid / empty imported action sets".into());
        }
        let mut next = self.clone();
        next.transaction(|s| {
            for set in &source.sets {
                let mut set = set.clone();
                set.id = s.allocate()?;
                for a in &mut set.actions {
                    a.id = s.allocate()?;
                    for p in &mut a.steps {
                        p.id = s.allocate()?;
                    }
                }
                s.sets.push(set);
            }
            Ok(())
        })?;
        Ok(next)
    }
    pub fn issues(&self, doc: Option<u64>, tools: &[u64]) -> Vec<String> {
        self.sets
            .iter()
            .flat_map(|s| {
                s.actions.iter().flat_map(move |a| {
                    a.steps.iter().filter_map(move |p| {
                        p.operation
                            .issue(doc, tools)
                            .map(|i| format!("{} / {} / step {}: {i}", s.name, a.name, p.id))
                    })
                })
            })
            .collect()
    }
}
pub struct Prepared {
    source: u64,
    revision: u64,
    frame: u32,
    pub state: DocumentState,
    pub label: String,
    pub scroll: [f64; 2],
}
impl Prepared {
    pub fn commit(self, doc: &mut Document) -> Result<bool, String> {
        self.commit_with_scale(doc, 1.)
    }
    pub fn commit_with_scale(self, doc: &mut Document, ppp: f64) -> Result<bool, String> {
        if doc.id != self.source || doc.revision != self.revision || doc.current_frame != self.frame
        {
            return Err("Document / revision / frame changed; replay again".into());
        }
        let mut view = doc.view;
        scroll(
            &mut view,
            [self.state.width as f64, self.state.height as f64],
            self.scroll,
            ppp,
        )?;
        let changed = doc.commit_state_named(self.state, &self.label)?;
        let view_changed = view != doc.view;
        doc.view = view;
        Ok(changed || view_changed)
    }
}
pub fn prepare(
    doc: &Document,
    action: &Action,
    available: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    if !name_valid(&action.name)
        || action.steps.is_empty()
        || action.steps.len() > 256
        || !action.steps.iter().any(|p| p.enabled)
    {
        return Err("Action has no enabled steps or exceeds limits".into());
    }
    if doc
        .memory_bytes()
        .saturating_mul(8)
        .saturating_add(doc.resident_bytes())
        > available
    {
        return Err("Auto action snapshot exceeds workspace memory".into());
    }
    let mut work = crate::layer_merge::preview(&doc.snapshot(), doc.current_frame)?;
    let mut scroll = [0.; 2];
    for (n, step) in action.steps.iter().enumerate().filter(|(_, p)| p.enabled) {
        if cancel.load(Ordering::Relaxed) {
            return Err("Auto action canceled; document unchanged".into());
        }
        let state = prepare_operation(&work, &step.operation, cancel)
            .map_err(|e| format!("Step {}: {e}", n + 1))?;
        if state
            .memory_bytes()
            .saturating_mul(8)
            .saturating_add(doc.resident_bytes())
            > available
        {
            return Err("Auto action result exceeds workspace memory".into());
        }
        state.restore(&mut work);
        if let Operation::Scroll { document } = step.operation {
            for k in 0..2 {
                scroll[k] += document[k];
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Auto action canceled; document unchanged".into());
    }
    let mut label = format!("Auto action: {}", action.name);
    while label.len() > 256 {
        label.pop();
    }
    Ok(Prepared {
        source: doc.id,
        revision: doc.revision,
        frame: doc.current_frame,
        state: work.snapshot(),
        scroll,
        label,
    })
}
pub fn prepare_operation(
    doc: &Document,
    op: &Operation,
    cancel: &AtomicBool,
) -> Result<DocumentState, String> {
    use crate::{
        layer_properties as p, layers,
        sizing::{self, Change},
    };
    if !op.valid() {
        return Err("Invalid action parameters".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Auto action canceled".into());
    }
    let guard = |n: usize| -> Result<(), String> {
        p::paintable(doc, n)?;
        if doc.layers[n].locked
            || doc
                .scene
                .file_objects
                .entries
                .iter()
                .any(|a| a.target == crate::layer_search::Key::Layer(p::raster(doc, n)))
        {
            return Err("Locked or linked raster cannot be edited by auto action".into());
        }
        Ok(())
    };
    match op {
        Operation::Canvas(c) => sizing::prepare(
            &doc.snapshot(),
            match c {
                Canvas::FlipHorizontal => Change::FlipHorizontal,
                Canvas::FlipVertical => Change::FlipVertical,
                Canvas::Clockwise => Change::Rotate90 { clockwise: true },
                Canvas::Counterclockwise => Change::Rotate90 { clockwise: false },
            },
            DEFAULT_DOCUMENT_BUDGET,
            cancel,
        ),
        Operation::Dpi(dpi) => sizing::prepare(
            &doc.snapshot(),
            Change::Dpi(*dpi),
            DEFAULT_DOCUMENT_BUDGET,
            cancel,
        ),
        Operation::NewRaster { name } => layers::prepare(
            doc,
            layers::Command::Create {
                kind: layers::Kind::Raster,
                name: name.clone(),
                parent: None,
            },
        )
        .map(|p| p.state),
        Operation::RenameRaster { layer, name } => {
            guard(*layer)?;
            layers::prepare(
                doc,
                layers::Command::Rename {
                    target: layers::Target::Raster(doc.scene.layer_ids.id(*layer)),
                    name: name.clone(),
                },
            )
            .map(|p| p.state)
        }
        Operation::Opacity { layer, percent } => {
            guard(*layer)?;
            p::prepare_patch(
                doc,
                &[p::raster(doc, *layer)],
                p::Patch {
                    opacity: Some(*percent),
                    ..Default::default()
                },
            )
        }
        Operation::Fill {
            layer,
            rgba,
            erase,
            alpha_lock,
        } => {
            guard(*layer)?;
            crate::bucket::prepare(
                doc,
                *layer,
                None,
                *rgba,
                *erase,
                *alpha_lock,
                Default::default(),
            )
        }
        Operation::Transform {
            layer,
            vector,
            settings,
            filter,
        } => {
            let key = if *vector {
                crate::layer_search::Key::Layer(p::Target::Vector(
                    doc.scene
                        .lines
                        .layers
                        .get(*layer)
                        .ok_or("Missing vector slot")?
                        .id,
                ))
            } else {
                if *layer >= doc.layers.len() {
                    return Err("Missing raster slot".into());
                }
                crate::layer_search::Key::Layer(p::raster(doc, *layer))
            };
            crate::affine_transform::prepare_with_progress(
                doc,
                key,
                false,
                *settings,
                false,
                *filter,
                &mut || {
                    if cancel.load(Ordering::Relaxed) {
                        Err("Transform round canceled".into())
                    } else {
                        Ok(())
                    }
                },
            )
            .map(|p| p.state)
        }
        Operation::Scroll { .. } => Ok(doc.snapshot()),
        Operation::Unsupported { command, .. } => {
            Err(format!("Unsupported / non-recordable command: {command}"))
        }
    }
}
/// A view command moves the viewport center in document pixels, independently
/// of zoom/rotation/flip and without adding a drawing history entry.
pub fn scroll(
    view: &mut crate::view::View,
    size: [f64; 2],
    delta: [f64; 2],
    ppp: f64,
) -> Result<(), String> {
    if delta == [0.; 2] {
        return Ok(());
    }
    if !ppp.is_finite() || ppp <= 0. || delta.iter().any(|v| !v.is_finite() || v.abs() > 256e6) {
        return Err("Invalid document scroll/display scale".into());
    }
    let center = view.screen_to_document([0.; 2], size, [0.; 2], ppp);
    let target = [center[0] + delta[0], center[1] + delta[1]];
    let mut next = *view;
    next.center_on(target, size, [0.; 2], ppp);
    let actual = next.screen_to_document([0.; 2], size, [0.; 2], ppp);
    if actual
        .iter()
        .zip(target)
        .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-5)
    {
        return Err("Scroll exceeds the supported view range".into());
    }
    *view = next;
    Ok(())
}
#[path = "auto_actions_io.rs"]
mod io;
#[cfg(test)]
#[path = "auto_actions_tests.rs"]
mod tests;
