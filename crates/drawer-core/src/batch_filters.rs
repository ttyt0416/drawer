//! Preflight every target, compute outside the document, then return one atomic state.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Task {
    Detail(crate::detail_filter::Settings),
    Blur(crate::spatial_blur::Settings),
    Gaussian {
        radius: u8,
        selection: bool,
    },
    Surface {
        operation: crate::surface_effects::Operation,
        selection: bool,
    },
}
impl Default for Task {
    fn default() -> Self {
        Self::Detail(crate::detail_filter::Settings {
            operation: crate::detail_filter::Operation::Sharpen { strength: 0.5 },
            selection: false,
        })
    }
}
impl Task {
    pub fn valid(self) -> bool {
        match self {
            Self::Detail(s) => s.valid(),
            Self::Blur(s) => s.valid(),
            Self::Gaussian { radius, .. } => radius <= 64,
            Self::Surface { operation, .. } => operation.valid(),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Detail(_) => "Detail / pixel / distortion",
            Self::Blur(_) => "Motion / radial / spin / lens blur",
            Self::Gaussian { .. } => "Gaussian blur",
            Self::Surface { .. } => "Editable surface effect",
        }
    }
    pub fn selection(&mut self) -> &mut bool {
        match self {
            Self::Detail(s) => &mut s.selection,
            Self::Blur(s) => &mut s.selection,
            Self::Gaussian { selection, .. } | Self::Surface { selection, .. } => selection,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub task: Task,
    pub targets: Vec<u64>,
}
impl Record {
    pub fn valid(&self) -> bool {
        self.task.valid()
            && !self.targets.is_empty()
            && self.targets.len() <= 256
            && self.targets.iter().all(|id| *id > 0)
            && self
                .targets
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.targets.len()
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.targets.len() * 8
    }
}
#[derive(Clone, Debug)]
pub struct Status {
    pub target: Target,
    pub reason: Option<String>,
}
pub fn inspect(d: &Document, targets: &[Target]) -> Vec<Status> {
    targets
        .iter()
        .map(|&target| {
            let reason = match target {
                Target::Raster(id) => crate::layers::index(d, id)
                    .ok_or("Missing raster source".to_string())
                    .and_then(|n| p::paintable(d, n))
                    .err(),
                _ => Some("Unsupported content type; select a raster layer".into()),
            };
            Status { target, reason }
        })
        .collect()
}
pub fn inspect_for(d: &Document, targets: &[Target], task: Task) -> Vec<Status> {
    let mut statuses = inspect(d, targets);
    for status in &mut statuses {
        if status.reason.is_some() {
            continue;
        }
        let Target::Raster(id) = status.target else {
            continue;
        };
        let n = crate::layers::index(d, id).unwrap();
        match task {
            Task::Surface { .. }
                if d.scene
                    .surface_effects
                    .rasters
                    .get(&id)
                    .is_some_and(|s| s.effects.len() >= 16) =>
            {
                status.reason = Some("Effect stack is full (16); remove an effect first".into())
            }
            Task::Detail(crate::detail_filter::Settings {
                operation: crate::detail_filter::Operation::Pixels(op),
                ..
            }) if !op.identity()
                && crate::clipping::alpha_locked(d, n)
                && matches!(
                    op,
                    crate::pixel_effects::Operation::LineWidth { .. }
                        | crate::pixel_effects::Operation::Dust {
                            target: crate::pixel_effects::DustTarget::Ink,
                            ..
                        }
                ) =>
            {
                status.reason = Some("Unlock transparency before changing ink coverage".into())
            }
            _ => {}
        }
    }
    statuses
}

pub fn prepare(
    d: &Document,
    targets: &[Target],
    task: Task,
    budget: usize,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    if !task.valid()
        || targets.is_empty()
        || targets.len() > 256
        || targets
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != targets.len()
    {
        return Err("Choose1–256 unique layers and valid filter settings".into());
    }
    let statuses = inspect_for(d, targets, task);
    let errors = statuses
        .iter()
        .filter_map(|s| {
            s.reason
                .as_ref()
                .map(|reason| format!("{:?}: {reason}", s.target))
        })
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(format!(
            "No layers processed. Resolve all selected targets first:\n{}",
            errors.join("\n")
        ));
    }
    let ids = targets
        .iter()
        .map(|t| {
            if let Target::Raster(id) = *t {
                id
            } else {
                unreachable!()
            }
        })
        .collect::<Vec<_>>();
    let pixels: usize = ids
        .iter()
        .map(|id| d.layers[crate::layers::index(d, *id).unwrap()].pixels.len())
        .sum();
    if pixels > 4 * 1024 * 1024
        || d.memory_bytes()
            .saturating_mul(5)
            .saturating_add(160 * 1024 * 1024)
            > budget.min(DEFAULT_DOCUMENT_BUDGET)
    {
        return Err("Batch source4M pixels or aggregate snapshot/workspace memory limit".into());
    }
    tick(0, targets.len() * 100)?;
    let mut state = d.snapshot();
    for (i, id) in ids.iter().enumerate() {
        tick(i * 100, ids.len() * 100)?;
        let working = crate::layer_merge::preview(&state, d.current_frame)?;
        let n = crate::layers::index(&working, *id).ok_or("Batch target disappeared")?;
        let mut progress = |done: usize, total: usize| {
            tick(
                i * 100
                    + done
                        .saturating_mul(100)
                        .checked_div(total.max(1))
                        .unwrap_or(0)
                        .min(100),
                ids.len() * 100,
            )
        };
        state = match task {
            Task::Detail(s) => {
                crate::detail_filter::prepare_with_budget(&working, n, s, budget, &mut progress)?
            }
            Task::Blur(s) => crate::spatial_blur::prepare(&working, n, s, &mut progress)?,
            Task::Gaussian { radius, selection } => crate::gaussian::prepare_with_progress(
                &working,
                n,
                radius,
                selection,
                &mut progress,
            )?,
            Task::Surface {
                operation,
                selection,
            } => {
                let mut stack = working
                    .scene
                    .surface_effects
                    .rasters
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                stack.effects.push(crate::surface_effects::Entry {
                    operation,
                    enabled: true,
                    scope: selection.then(|| crate::surface_effects::Scope::capture(&working)),
                });
                crate::surface_effects::prepare(&working, n, stack, &mut progress)?
            }
        };
        if state
            .memory_bytes()
            .saturating_mul(5)
            .saturating_add(160 * 1024 * 1024)
            > budget.min(DEFAULT_DOCUMENT_BUDGET)
        {
            return Err(
                "Batch output exceeds aggregate workspace budget; no layers applied".into(),
            );
        }
    }
    if state != d.snapshot() {
        state.scene.last_batch_filter = Some(Box::new(Record { task, targets: ids }));
        crate::layer_groups::finish(d, &state)?;
    }
    tick(targets.len() * 100, targets.len() * 100)?;
    Ok(state)
}
