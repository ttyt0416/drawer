//! Frozen, non-destructive visible copies and reviewed layer cleanup.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_search::{Key, Kind, Row},
    sizing::DocumentState,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    VisibleCopy,
    Empty,
    Hidden,
}
impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::VisibleCopy => "Copy visible to new layer",
            Self::Empty => "Delete empty layers",
            Self::Hidden => "Delete hidden layers",
        }
    }
}
pub struct Prepared {
    pub state: DocumentState,
    pub candidates: Vec<(Key, String)>,
    pub retained: Vec<(Key, String)>,
    pub comp_references: Vec<(String, usize)>,
    pub exposures: usize,
    pub notes: Vec<String>,
    pub selected: Option<Key>,
}
fn check(d: &Document) -> Result<(), String> {
    if d.layers.is_empty()
        || !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || !d.color.valid_layers(&d.layers)
        || d.layers.iter().any(|l| !l.valid(d.width, d.height))
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || d.scene.clipping.solo.is_some()
        || d.scene.layer_groups.solo.is_some()
    {
        return Err(
            "Restore solo visibility and use a valid document within the snapshot budget".into(),
        );
    }
    Ok(())
}
fn locked(row: &Row) -> bool {
    row.states[1] || row.states[2] || row.states[3] || row.states[4]
}
fn empty(d: &Document, row: &Row) -> bool {
    if row.states[5]
        || matches!(row.kind, Kind::Correction | Kind::Tone)
        || d.scene.file_objects.get(row.key).is_some()
    {
        return false;
    }
    if let Some(t) = c::target(d, row.key) {
        if !p::properties(d, t).blend.is_normal() {
            return false;
        }
        if d.scene
            .layer_effects
            .get(t)
            .is_some_and(|e| e.mask.is_some() || e.adjustment.is_some())
            || d.scene
                .appearance
                .get(t)
                .is_some_and(|s| *s != Default::default())
        {
            return false;
        }
        match t {
            Target::Raster(id) => {
                let Some(n) = crate::layers::index(d, id) else {
                    return false;
                };
                if d.scene.paper_background == Some(id)
                    || d.scene.paper.layers.get(&n).is_some_and(|v| {
                        v.pattern.effective() || v.blend != crate::brush_composite::Mode::Normal
                    })
                {
                    return false;
                }
                return d.layers[n].precise.as_ref().map_or_else(
                    || d.layers[n].pixels.iter().all(|v| v[3] == 0),
                    |v| v.iter().all(|v| v[3] == 0),
                );
            }
            Target::Vector(id) => {
                return d
                    .scene
                    .lines
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .is_some_and(|l| l.paths.is_empty());
            }
            Target::Cell { track, cell } => {
                return d
                    .animation
                    .as_ref()
                    .and_then(|a| a.tracks.get(track as usize))
                    .and_then(|t| t.cells.get(cell as usize))
                    .is_some_and(|c| c.pixels.iter().all(|p| p[3] == 0));
            }
            _ => {}
        }
    }
    matches!(row.key, Key::Object(_)) && c::source_index(d, row.key).is_some_and(|n| matches!(&d.scene.objects[n], crate::scene::PageObject::Text {text, ..} if text.is_empty()))
}
fn prune_required(d: &Document, rows: &[Row], keys: &mut BTreeSet<Key>) {
    if (0..d.layers.len()).all(|n| keys.contains(&Key::Layer(p::raster(d, n)))) {
        keys.remove(&Key::Layer(p::raster(d, 0)));
    }
    if let Some(a) = &d.animation {
        for (n, t) in a.tracks.iter().enumerate() {
            let id = d.scene.catalog.tracks.id(n);
            let cell = |i| Key::Cell {
                track: id,
                cell: d.scene.catalog.cell_id(id, i),
            };
            let all = (0..t.cells.len()).all(|i| keys.contains(&cell(i)));
            if keys.contains(&Key::Track(id)) && all {
                continue;
            }
            keys.remove(&Key::Track(id));
            if all {
                keys.remove(&cell(0));
            }
            // A surviving track needs an exposure as well as a cell. Removing a
            // referenced cell removes its exposure markers, never retargets one.
            if t.exposures.iter().all(|e| keys.contains(&cell(e.cell)))
                && let Some(e) = t.exposures.first()
            {
                keys.remove(&cell(e.cell));
            }
        }
    }
    // Never remove a parent around a retained/locked descendant.
    for _ in 0..17 {
        let parents = rows
            .iter()
            .filter_map(|r| {
                r.parent
                    .filter(|parent| keys.contains(parent) && !keys.contains(&r.key))
            })
            .collect::<Vec<_>>();
        if parents.is_empty() {
            break;
        }
        for parent in parents {
            keys.remove(&parent);
        }
    }
}
pub fn prepare(d: &Document, operation: Operation) -> Result<Prepared, String> {
    check(d)?;
    if operation == Operation::VisibleCopy {
        return visible_copy(d);
    }
    let rows = c::rows(d)?;
    let matches = |r: &Row| match operation {
        Operation::Empty => empty(d, r),
        Operation::Hidden => !r.states[0] && !matches!(r.key, Key::Selection(_)),
        Operation::VisibleCopy => false,
    };
    let possible = rows.iter().filter(|r| matches(r)).collect::<Vec<_>>();
    let mut keys = possible
        .iter()
        .filter(|r| !locked(r))
        .map(|r| r.key)
        .collect::<BTreeSet<_>>();
    prune_required(d, &rows, &mut keys);
    let retained = possible
        .iter()
        .filter(|r| !keys.contains(&r.key))
        .map(|r| {
            (
                r.key,
                format!(
                    "{} — {}",
                    r.name,
                    if locked(r) {
                        "locked/protected"
                    } else {
                        "required by a retained child, raster or timeline"
                    }
                ),
            )
        })
        .collect();
    if keys.is_empty() {
        return Err("No eligible layers to delete; locked/protected layers and required raster/timeline content are retained".into());
    }
    let candidates = rows
        .iter()
        .filter(|r| keys.contains(&r.key))
        .map(|r| (r.key, r.name.clone()))
        .collect::<Vec<_>>();
    let comp_references = d
        .scene
        .layer_comps
        .comps
        .iter()
        .filter_map(|c| {
            let count = c.entries.iter().filter(|e| keys.contains(&e.key)).count();
            (count > 0).then(|| (c.name.clone(), count))
        })
        .collect();
    let exposures = d
        .animation
        .iter()
        .flat_map(|a| a.tracks.iter().enumerate())
        .map(|(n, t)| {
            let track = d.scene.catalog.tracks.id(n);
            t.exposures
                .iter()
                .filter(|e| {
                    keys.contains(&Key::Track(track))
                        || keys.contains(&Key::Cell {
                            track,
                            cell: d.scene.catalog.cell_id(track, e.cell),
                        })
                })
                .count()
        })
        .sum();
    let chosen = candidates.iter().map(|v| v.0).collect::<Vec<_>>();
    let state = crate::search_edit::delete_batch(d, &chosen, true)?;
    Ok(Prepared {
        state, candidates, retained, comp_references, exposures,
        notes: vec![
            "All layer, pixel, position and alpha protection locks (including inherited locks) are excluded. At least one raster and valid content in each retained timeline track are kept.".into(),
            "Comp records keep stable IDs and report deleted entries as missing. Removed cell exposure markers are deleted; surviving exposures may hold longer. One Undo restores all pixels, structure and references.".into(),
        ], selected:None,
    })
}
fn visible_copy(d: &Document) -> Result<Prepared, String> {
    let count = d.width as usize * d.height as usize;
    if count > 4 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(count.saturating_mul(44))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Visible copy exceeds floating work/Undo memory".into());
    }
    let mut state = crate::layers::prepare(
        d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: format!("Visible copy — frame {}", d.current_frame + 1),
            parent: None,
        },
    )?
    .state;
    let n = state.layers.len() - 1;
    let id = state.scene.layer_ids.id(n);
    let renderer = crate::render::Renderer::new(d);
    let mut samples = Vec::with_capacity(count);
    for y in 0..d.height {
        for x in 0..d.width {
            samples.push(renderer.sample([x as f64 + 0.5, y as f64 + 0.5])?);
        }
    }
    let extent = PlacedRaster {
        width: d.width,
        height: d.height,
        offset: [0; 2],
    };
    state.layers[n].placement = None;
    crate::color_storage::set_samples(&mut state.layers[n], &d.color, extent, samples)?;
    // A page source stack follows the raster/timeline stack. Defer the copy
    // until its end, otherwise vector/text sources would render over it again.
    state.scene.render_order.entries = crate::render_order::normalized(&d.scene);
    state
        .scene
        .render_order
        .entries
        .push(crate::render_order::Entry {
            key: Key::Layer(Target::Raster(id)),
            group: crate::render_order::Group::World,
        });
    p::validate_edit(d, &state)?;
    if !state.scene.valid()
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Visible copy exceeds scene/Undo budget".into());
    }
    Ok(Prepared {
        state, candidates:vec![], retained:vec![], comp_references:vec![], exposures:0,
        notes:vec![format!("Copy the displayed page at frame {} into a new Normal/100% raster, including visible draft layers. Sources, locks, masks, objects and animation remain editable.",d.current_frame+1), "Only page bounds are copied. The new layer is visible above all page sources. Original layers stay visible, so semi-transparent areas can become denser; review the before/after preview. One Undo removes only the copy.".into()],
        selected:Some(Key::Layer(Target::Raster(id))),
    })
}
