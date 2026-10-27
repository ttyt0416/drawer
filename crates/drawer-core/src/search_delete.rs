use super::*;
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn prepare(
    d: &Document,
    selected: &[Key],
    drop_exposures: bool,
) -> Result<DocumentState, String> {
    let chosen = selected.iter().copied().collect::<BTreeSet<_>>();
    let targets = c::target_map(d);
    if selected.iter().any(|k| {
        c::properties(d, *k).locked || targets.get(k).is_some_and(|t| p::inherited(d, *t).locked)
    }) {
        return Err("Unlock selected layers before deleting".into());
    }
    let mut s = d.snapshot();
    let raster = selected
        .iter()
        .filter_map(|k| {
            if let Key::Layer(Target::Raster(id)) = k {
                Some(*id)
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    if !raster.is_empty() {
        if raster.len() >= s.layers.len() {
            return Err("Keep at least one raster layer".into());
        }
        s.scene.layer_ids.initialize(d.layers.len())?;
        let mut map = vec![None; d.layers.len()];
        let mut next = 0;
        for (n, to) in map.iter_mut().enumerate() {
            if !raster.contains(&s.scene.layer_ids.id(n)) {
                *to = Some(next);
                next += 1;
            }
        }
        s.layers = s
            .layers
            .into_iter()
            .enumerate()
            .filter_map(|(n, l)| map[n].map(|_| l))
            .collect();
        for id in raster {
            s.scene.layer_properties.rasters.remove(&id);
            s.scene.layer_effects.rasters.remove(&id);
            s.scene.appearance.rasters.remove(&id);
            s.scene.catalog.rasters.remove(&id);
            s.scene.clipping.remove_owner(Target::Raster(id));
        }
        crate::layers::remap(&mut s.scene, &map);
    }
    for id in selected.iter().filter_map(|k| {
        if let Key::Layer(Target::Vector(id)) = k {
            Some(*id)
        } else {
            None
        }
    }) {
        let working = crate::layer_merge::preview(&s, d.current_frame)?;
        s = crate::layers::prepare(
            &working,
            crate::layers::Command::Delete(crate::layers::Target::Vector(id)),
        )?
        .state;
    }
    let mut folders = selected
        .iter()
        .filter_map(|k| {
            if let Key::Layer(Target::Folder(n)) = k {
                Some(*n as usize)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let depth = |mut n: usize| {
        let mut depth = 0;
        for _ in 0..16 {
            let Some(p) = d
                .scene
                .paper
                .folders
                .iter()
                .position(|f| f.children.contains(&crate::paper::Node::Folder(n)))
            else {
                break;
            };
            n = p;
            depth += 1;
        }
        depth
    };
    folders.sort_by_key(|n| std::cmp::Reverse(depth(*n)));
    for n in folders {
        if !s.scene.paper.folders[n].children.is_empty() {
            return Err("Folder contents are outside the selected results; select them explicitly or unwrap first".into());
        }
        s.scene
            .paper
            .roots
            .retain(|v| *v != crate::paper::Node::Folder(n));
        for f in &mut s.scene.paper.folders {
            f.children.retain(|v| *v != crate::paper::Node::Folder(n));
        }
        s.scene.paper.folders[n].paper = Default::default();
        s.scene.paper.folders[n].through = false;
        s.scene.layer_properties.folders.remove(&(n as u16));
        s.scene.layer_effects.folders.remove(&(n as u16));
        s.scene.catalog.folders.remove(&(n as u16));
        s.scene.clipping.remove_owner(Target::Folder(n as u16));
        s.scene
            .rulers
            .guides
            .retain(|g| g.scope != crate::ruler_scope::Scope::Folder(n as u16));
    }
    let mut object_indices = selected
        .iter()
        .filter_map(|k| {
            if matches!(k, Key::Object(_)) {
                c::source_index(d, *k)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    object_indices.sort_unstable_by(|a, b| b.cmp(a));
    if !object_indices.is_empty() {
        s.scene.catalog.objects.initialize(s.scene.objects.len())?;
        let old = s.scene.catalog.objects.clone();
        for n in object_indices {
            let count = s.scene.objects.len();
            let id = s.scene.catalog.objects.remove(n, count)?;
            s.scene.objects.remove(n);
            s.scene.catalog.object_info.remove(&id);
        }
        let map = (0..d.scene.objects.len())
            .map(|n| {
                s.scene
                    .catalog
                    .objects
                    .index(old.id(n), s.scene.objects.len())
            })
            .collect::<Vec<_>>();
        let item = |i: crate::alignment::Item| {
            if let crate::alignment::Item::Object(n) = i {
                map.get(n)
                    .copied()
                    .flatten()
                    .map(crate::alignment::Item::Object)
            } else {
                Some(i)
            }
        };
        s.scene.alignment.selected = s
            .scene
            .alignment
            .selected
            .iter()
            .copied()
            .filter_map(item)
            .collect();
        if let crate::alignment::Reference::Key(i) = s.scene.alignment.reference {
            s.scene.alignment.reference = item(i).map_or(
                crate::alignment::Reference::Selection,
                crate::alignment::Reference::Key,
            );
        }
    }
    let mut worlds = selected
        .iter()
        .filter_map(|k| {
            if matches!(k, Key::World(_)) {
                c::source_index(d, *k)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    worlds.sort_unstable_by(|a, b| b.cmp(a));
    for n in worlds {
        let count = s.scene.worlds.len();
        let id = s.scene.catalog.worlds.remove(n, count)?;
        s.scene.worlds.remove(n);
        s.scene.catalog.world_info.remove(&id);
    }
    for key in selected {
        match *key {
            Key::Gradient(id) => {
                s.scene.gradients.layers.retain(|g| g.id != id);
                s.scene.catalog.gradients.remove(&id);
                s.scene
                    .alignment
                    .selected
                    .retain(|i| *i != crate::alignment::Item::Gradient(id));
                if s.scene.alignment.reference
                    == crate::alignment::Reference::Key(crate::alignment::Item::Gradient(id))
                {
                    s.scene.alignment.reference = Default::default();
                }
            }
            Key::Selection(id) => {
                s.scene.selections.saved.retain(|v| v.id != id);
                s.scene.catalog.selections.remove(&id);
            }
            _ => {}
        }
    }
    animation(&mut s, &chosen, drop_exposures)?;
    if s.scene
        .rulers
        .active
        .is_some_and(|id| !s.scene.rulers.guides.iter().any(|g| g.id == id))
    {
        s.scene.rulers.active = None;
    }
    Ok(s)
}
fn animation(
    s: &mut DocumentState,
    selected: &BTreeSet<Key>,
    drop_exposures: bool,
) -> Result<(), String> {
    if !selected
        .iter()
        .any(|k| matches!(k, Key::Track(_) | Key::Cell { .. }))
    {
        return Ok(());
    }
    let Some(a) = s.animation.as_mut() else {
        return Err("Missing timeline".into());
    };
    s.scene.catalog.tracks.initialize(a.tracks.len())?;
    let old_tracks = s.scene.catalog.tracks.clone();
    let mut old_cells = BTreeMap::new();
    let mut kept = vec![];
    let mut kept_ids = vec![];
    for (n, t) in a.tracks.iter().enumerate() {
        let track = old_tracks.id(n);
        let mut ids = s
            .scene
            .catalog
            .cells
            .get(&track)
            .cloned()
            .unwrap_or_default();
        ids.initialize(t.cells.len())?;
        old_cells.insert(track, ids.clone());
        let remove = selected.contains(&Key::Track(track));
        if remove
            && !t.cells.iter().enumerate().all(|(i, _)| {
                selected.contains(&Key::Cell {
                    track,
                    cell: ids.id(i),
                })
            })
        {
            return Err("Animation folder has cells outside the selected results".into());
        }
        if remove {
            s.scene.catalog.track_info.remove(&track);
            s.scene.catalog.cell_info.remove(&track);
            s.scene.catalog.cells.remove(&track);
            continue;
        }
        let mut track_value = t.clone();
        let mut map = vec![None; t.cells.len()];
        let mut cell_values = vec![];
        let mut cell_ids = vec![];
        for (i, cell) in t.cells.iter().enumerate() {
            let id = ids.id(i);
            if selected.contains(&Key::Cell { track, cell: id }) {
                if !drop_exposures && t.exposures.iter().any(|e| e.cell == i) {
                    return Err("A selected cell has exposures; select its complete animation folder or edit the timeline first".into());
                }
                if let Some(info) = s.scene.catalog.cell_info.get_mut(&track) {
                    info.remove(&id);
                }
            } else {
                map[i] = Some(cell_values.len());
                cell_values.push(cell.clone());
                cell_ids.push(id);
            }
        }
        if cell_values.is_empty() && s.scene.cels.is_default() {
            return Err("Keep at least one cell in an animation folder".into());
        }
        if drop_exposures {
            track_value.exposures.retain(|e| map[e.cell].is_some());
            if track_value.exposures.is_empty() && s.scene.cels.is_default() {
                return Err("Keep at least one exposure in a retained animation folder".into());
            }
        }
        for e in &mut track_value.exposures {
            e.cell = map[e.cell].ok_or("Selected cell exposure remains")?;
        }
        track_value.cells = cell_values;
        ids.values = cell_ids;
        s.scene.catalog.cells.insert(track, ids);
        kept.push(track_value);
        kept_ids.push(track);
    }
    a.tracks = kept;
    s.scene.catalog.tracks.values = kept_ids;
    let map_track = |old: usize| {
        s.scene
            .catalog
            .tracks
            .index(old_tracks.id(old), a.tracks.len())
    };
    let map_cell = |track: usize, cell: usize| {
        let id = old_tracks.id(track);
        let next = map_track(track)?;
        let old_id = old_cells.get(&id)?.id(cell);
        let current = s.scene.catalog.cells.get(&id)?;
        current
            .index(old_id, a.tracks[next].cells.len())
            .map(|c| (next, c))
    };
    let old_properties = s.scene.layer_properties.clone();
    s.scene.layer_properties.tracks = old_properties
        .tracks
        .into_iter()
        .filter_map(|(n, p)| map_track(n as usize).map(|n| (n as u16, p)))
        .collect();
    s.scene.layer_properties.cells = old_properties
        .cells
        .into_iter()
        .filter_map(|(n, p)| {
            map_cell((n >> 32) as usize, (n & 0xffffffff) as usize)
                .map(|(t, c)| (((t as u64) << 32 | c as u64), p))
        })
        .collect();
    s.scene.clipping.alpha = s
        .scene
        .clipping
        .alpha
        .iter()
        .filter_map(|t| {
            if let Target::Cell { track, cell } = *t {
                map_cell(track as usize, cell as usize).map(|(track, cell)| Target::Cell {
                    track: track as u16,
                    cell: cell as u16,
                })
            } else {
                Some(*t)
            }
        })
        .collect();
    Ok(())
}
