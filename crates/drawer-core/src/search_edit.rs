//! Batch edits resolve the live query first; filtered-out selections never enter a command.
#[path = "search_delete.rs"]
mod delete;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_properties::{self as p, Patch, Target},
    layer_search::{self as search, Key, Selection, Settings},
    sizing::DocumentState,
};
pub enum Edit {
    Properties(Patch),
    Alpha(bool),
    Rename(String),
    Tag { value: String, remove: bool },
    Delete,
}
pub struct Prepared {
    pub state: DocumentState,
    pub affected: Vec<Key>,
}
pub fn prepare(
    d: &Document,
    settings: &Settings,
    selection: &Selection,
    edit: Edit,
) -> Result<Prepared, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Search edit snapshot limit".into());
    }
    let rows = c::rows(d)?;
    let results = search::query(
        &rows,
        settings,
        selection.primary,
        d.scene.catalog.expression,
    )?;
    let affected = selection.visible(&results);
    if affected.is_empty() {
        return Err("Select at least one current search result".into());
    }
    let targets = c::target_map(d);
    let canonical = affected
        .iter()
        .filter_map(|k| targets.get(k).copied())
        .collect::<Vec<_>>();
    if canonical.len() != affected.len() {
        return Err("Search source changed; refresh results".into());
    }
    let state = match edit {
        Edit::Properties(patch) => p::prepare_patch(d, &canonical, patch)?,
        Edit::Alpha(enabled) => {
            let eligible = canonical
                .iter()
                .copied()
                .filter(|t| matches!(t, Target::Raster(_) | Target::Cell { .. }))
                .collect::<Vec<_>>();
            if eligible.is_empty() {
                return Err("Alpha protection applies to raster layers and cells".into());
            }
            crate::clipping::prepare(d, &eligible, Some(enabled), None)?
        }
        Edit::Rename(value) => {
            let value = value.trim();
            if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control)
            {
                return Err("Layer names require 1–64 printable characters".into());
            }
            let mut s = d.snapshot();
            for key in &affected {
                if rows.iter().any(|r| r.key == *key && r.name == value) {
                    continue;
                }
                if targets.get(key).is_some_and(|t| p::inherited(d, *t).locked)
                    || c::properties(d, *key).locked
                {
                    return Err("Unlock selected layers before renaming".into());
                }
                match targets.get(key) {
                    Some(Target::Raster(id)) => {
                        s.layers[crate::layers::index(d, *id).ok_or("Missing layer")?].name =
                            value.into()
                    }
                    Some(Target::Vector(id)) => {
                        s.scene
                            .lines
                            .layers
                            .iter_mut()
                            .find(|l| l.id == *id)
                            .ok_or("Missing layer")?
                            .name = value.into()
                    }
                    Some(Target::Folder(n)) => {
                        s.scene.paper.folders[*n as usize].name = value.into()
                    }
                    Some(Target::Track(n)) => {
                        s.animation.as_mut().ok_or("Missing timeline")?.tracks[*n as usize].name =
                            value.into()
                    }
                    Some(Target::Cell { track, cell }) => {
                        s.animation.as_mut().ok_or("Missing timeline")?.tracks[*track as usize]
                            .cells[*cell as usize]
                            .name = value.into()
                    }
                    _ => match *key {
                        Key::Gradient(id) => {
                            s.scene
                                .gradients
                                .layers
                                .iter_mut()
                                .find(|g| g.id == id)
                                .ok_or("Missing gradient")?
                                .name = value.into();
                            let mut info = s.scene.catalog.info(*key).cloned().unwrap_or_default();
                            info.name = None;
                            s.scene.catalog.set(*key, info);
                        }
                        Key::Selection(id) => {
                            s.scene
                                .selections
                                .saved
                                .iter_mut()
                                .find(|v| v.id == id)
                                .ok_or("Missing selection")?
                                .name = value.into()
                        }
                        _ => {
                            let mut info = s.scene.catalog.info(*key).cloned().unwrap_or_default();
                            info.name = Some(value.into());
                            s.scene.catalog.set(*key, info);
                        }
                    },
                }
            }
            s
        }
        Edit::Tag { value, remove } => {
            if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err("Choose a printable tag up to 128 bytes".into());
            }
            let folded = search::fold(&value);
            let mut s = d.snapshot();
            for key in &affected {
                let mut info = s.scene.catalog.info(*key).cloned().unwrap_or_default();
                if remove {
                    info.tags.retain(|v| search::fold(v) != folded);
                } else if !info.tags.iter().any(|v| search::fold(v) == folded) {
                    if info.tags.len() >= 16 {
                        return Err("Layer tag limit (16)".into());
                    }
                    info.tags.push(value.clone());
                }
                s.scene.catalog.set(*key, info);
            }
            s
        }
        Edit::Delete => delete::prepare(d, &affected, false)?,
    };
    let state = finish(d, state)?;
    Ok(Prepared { state, affected })
}
pub(crate) fn delete_batch(
    d: &Document,
    keys: &[Key],
    exposures: bool,
) -> Result<DocumentState, String> {
    finish(d, delete::prepare(d, keys, exposures)?)
}
fn finish(d: &Document, mut state: DocumentState) -> Result<DocumentState, String> {
    c::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    crate::file_objects::prune(&mut state);
    crate::layer_groups::prune(&mut state)?;
    crate::paper_background::prune(&mut state);
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || !state
            .scene
            .catalog
            .valid_animation(state.animation.as_ref())
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Search edit scene/Undo budget".into());
    }
    p::validate_edit(d, &state)?;
    Ok(state)
}
