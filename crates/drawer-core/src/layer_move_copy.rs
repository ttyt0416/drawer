use crate::{
    document::Document,
    layer_catalog as c, layer_groups as groups,
    layer_properties::{self as p, Target},
    layer_search::Key,
    paper::Node,
    sizing::DocumentState,
};
use std::collections::BTreeMap;
pub(super) struct Copied {
    pub state: DocumentState,
    pub keys: Vec<Key>,
}
fn name(n: &str) -> String {
    format!("{} copy", n.chars().take(59).collect::<String>())
}
fn one(
    state: &mut DocumentState,
    key: Key,
    map: &mut BTreeMap<Key, Key>,
    depth: usize,
) -> Result<Key, String> {
    if depth > 16 {
        return Err("Copy folder depth limit".into());
    }
    let d = crate::layer_merge::preview(state, 0)?;
    let t = c::target(&d, key).ok_or("Copy source missing")?;
    let new = match t {
        Target::Raster(id) | Target::Vector(id) => {
            let kind = if matches!(t, Target::Raster(_)) {
                crate::layers::Target::Raster(id)
            } else {
                crate::layers::Target::Vector(id)
            };
            let copy = crate::layers::prepare(&d, crate::layers::Command::Duplicate(kind))?;
            *state = copy.state;
            match copy.selected {
                crate::layers::Target::Raster(id) => Key::Layer(Target::Raster(id)),
                crate::layers::Target::Vector(id) => Key::Layer(Target::Vector(id)),
            }
        }
        Target::Folder(n) => {
            if d.scene.file_objects.get(key).is_some() {
                return Err(
                    "Restore the original linked folder before copying its editable subtree".into(),
                );
            }
            let index = n as usize;
            let old = d.scene.paper.folders[index].clone();
            let parent = d
                .scene
                .paper
                .folders
                .iter()
                .position(|f| f.children.contains(&Node::Folder(index)));
            let created = crate::layer_folders::prepare(
                &d,
                crate::layer_folders::Command::Create {
                    name: name(&old.name),
                    parent,
                },
            )?;
            let Target::Folder(to) = created.selected else {
                unreachable!()
            };
            *state = created.state;
            map.insert(key, Key::Layer(Target::Folder(to)));
            for node in &old.children {
                let child = crate::clipping::node_key(&d, *node);
                let copied = one(state, Key::Layer(child), map, depth + 1)?;
                let current = crate::layer_merge::preview(state, 0)?;
                let t = c::target(&current, copied).ok_or("Copied child missing")?;
                let copied_node =
                    crate::clipping::node(&current, t).ok_or("Copied child has no folder node")?;
                state.scene.paper.roots.retain(|n| *n != copied_node);
                for f in &mut state.scene.paper.folders {
                    f.children.retain(|n| *n != copied_node);
                }
                state.scene.paper.folders[to as usize]
                    .children
                    .push(copied_node);
            }
            state.scene.paper.folders[to as usize].through = old.through;
            state.scene.paper.folders[to as usize].paper = old.paper;
            if let Some(p) = d.scene.layer_properties.folders.get(&n) {
                state.scene.layer_properties.folders.insert(to, *p);
            }
            if let Some(e) = d.scene.layer_effects.folders.get(&n) {
                state.scene.layer_effects.folders.insert(to, e.clone());
            }
            if let Some(i) = d.scene.catalog.folders.get(&n) {
                state.scene.catalog.folders.insert(to, i.clone());
            }
            if let Some(base) = d.scene.clipping.binding(t) {
                state.scene.clipping.bind(Target::Folder(to), Some(base));
            }
            Key::Layer(Target::Folder(to))
        }
        Target::Object(id) => {
            let n = c::source_index(&d, Key::Object(id)).ok_or("Copy object missing")?;
            state.scene.objects.push(d.scene.objects[n].clone());
            c::reconcile(&d, state)?;
            let new = Key::Object(
                state
                    .scene
                    .catalog
                    .objects
                    .id(state.scene.objects.len() - 1),
            );
            if let Some(info) = d.scene.catalog.info(key) {
                state.scene.catalog.set(new, info.clone());
            }
            crate::file_objects::duplicate(state, key, new);
            new
        }
        Target::Gradient(id) => {
            let mut layer = d
                .scene
                .gradients
                .layers
                .iter()
                .find(|g| g.id == id)
                .ok_or("Copy gradient missing")?
                .clone();
            let to = state.scene.gradients.next_id;
            state.scene.gradients.next_id = to.checked_add(1).ok_or("Gradient ID limit")?;
            layer.id = to;
            layer.name = name(&layer.name);
            state.scene.gradients.layers.push(layer);
            let new = Key::Gradient(to);
            if let Some(info) = d.scene.catalog.info(key) {
                state.scene.catalog.set(new, info.clone());
            }
            crate::file_objects::duplicate(state, key, new);
            new
        }
        Target::World(id) => {
            let n = c::source_index(&d, Key::World(id)).ok_or("Copy 3D missing")?;
            state.scene.worlds.push(d.scene.worlds[n].clone());
            c::reconcile(&d, state)?;
            let new = Key::World(state.scene.catalog.worlds.id(state.scene.worlds.len() - 1));
            if let Some(info) = d.scene.catalog.info(key) {
                state.scene.catalog.set(new, info.clone());
            }
            crate::file_objects::duplicate(state, key, new);
            new
        }
        Target::Track(n) => {
            let a = state.animation.as_mut().ok_or("Copy timeline missing")?;
            if a.tracks.len() >= 32 {
                return Err("Animation track limit".into());
            }
            let to = a.tracks.len() as u16;
            let mut track = a.tracks[n as usize].clone();
            track.name = name(&track.name);
            let cells = track.cells.len();
            a.tracks.push(track);
            c::reconcile(&d, state)?;
            if let Some(props) = d.scene.layer_properties.tracks.get(&n) {
                state.scene.layer_properties.tracks.insert(to, *props);
            }
            let old_id = d.scene.catalog.tracks.id(n as usize);
            let to_id = state.scene.catalog.tracks.id(to as usize);
            if let Some(i) = d.scene.catalog.track_info.get(&old_id) {
                state.scene.catalog.track_info.insert(to_id, i.clone());
            }
            for cell in 0..cells {
                let old = ((n as u64) << 32) | cell as u64;
                let new = ((to as u64) << 32) | cell as u64;
                if let Some(props) = d.scene.layer_properties.cells.get(&old) {
                    state.scene.layer_properties.cells.insert(new, *props);
                }
                let a = Target::Cell {
                    track: n,
                    cell: cell as u16,
                };
                let b = Target::Cell {
                    track: to,
                    cell: cell as u16,
                };
                if d.scene.clipping.alpha.contains(&a) {
                    state.scene.clipping.alpha.insert(b);
                }
                let old_cell = d.scene.catalog.cell_id(old_id, cell);
                let new_cell = state.scene.catalog.cell_id(to_id, cell);
                if let Some(info) = d
                    .scene
                    .catalog
                    .cell_info
                    .get(&old_id)
                    .and_then(|m| m.get(&old_cell))
                {
                    state
                        .scene
                        .catalog
                        .cell_info
                        .entry(to_id)
                        .or_default()
                        .insert(new_cell, info.clone());
                }
                let old_key = Key::Cell {
                    track: old_id,
                    cell: old_cell,
                };
                let new_key = Key::Cell {
                    track: to_id,
                    cell: new_cell,
                };
                crate::file_objects::duplicate(state, old_key, new_key);
                map.insert(old_key, new_key);
            }
            let new = Key::Track(to_id);
            crate::file_objects::duplicate(state, key, new);
            new
        }
        _ => return Err(
            "Copy-move an animation track or an image/vector source rather than an individual cell"
                .into(),
        ),
    };
    map.insert(key, new);
    groups::finish(&d, state)?;
    Ok(new)
}
pub(super) fn prepare(d: &Document, keys: &[Key]) -> Result<Copied, String> {
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    c::reconcile(d, &mut state)?;
    let original = crate::layer_merge::preview(&state, d.current_frame)?;
    let mut map = BTreeMap::new();
    let mut copies = vec![];
    for key in keys {
        copies.push(one(&mut state, *key, &mut map, 0)?);
    }
    let current = crate::layer_merge::preview(&state, d.current_frame)?;
    for (old, new) in &map {
        let (Some(a), Some(b)) = (c::target(&original, *old), c::target(&current, *new)) else {
            continue;
        };
        if let Some(base) = original.scene.clipping.binding(a) {
            let base_key = c::key(&original, base);
            let copied = map
                .get(&base_key)
                .and_then(|k| c::target(&current, *k))
                .unwrap_or(base);
            state.scene.clipping.bind(b, Some(copied));
        }
    }
    for old in &original.scene.layer_groups.groups {
        if old.members.iter().all(|k| map.contains_key(k)) {
            let data = &mut state.scene.layer_groups;
            let id = data.next.checked_add(1).ok_or("Group ID limit")?;
            data.next = id;
            data.groups.push(groups::Group {
                id,
                name: name(&old.name),
                members: old.members.iter().map(|k| map[k]).collect(),
                linked: old.linked,
            });
        }
    }
    let convert = |t| match t {
        crate::line_tools::Target::Raster(n) => {
            map.get(&Key::Layer(p::raster(&original, n))).and_then(|k| {
                if let Some(Target::Raster(id)) = c::target(&current, *k) {
                    crate::layers::index(&current, id).map(crate::line_tools::Target::Raster)
                } else {
                    None
                }
            })
        }
        crate::line_tools::Target::Vector(id) => {
            map.get(&Key::Layer(Target::Vector(id))).and_then(|k| {
                if let Key::Layer(Target::Vector(id)) = k {
                    Some(crate::line_tools::Target::Vector(*id))
                } else {
                    None
                }
            })
        }
        _ => None,
    };
    for guide in &original.scene.rulers.guides {
        let linked = guide.link.and_then(convert);
        let scope = match guide.scope {
            crate::ruler_scope::Scope::Folder(n) => {
                map.get(&Key::Layer(Target::Folder(n))).and_then(|k| {
                    if let Key::Layer(Target::Folder(n)) = k {
                        Some(crate::ruler_scope::Scope::Folder(*n))
                    } else {
                        None
                    }
                })
            }
            crate::ruler_scope::Scope::Layer(t) => convert(t).map(crate::ruler_scope::Scope::Layer),
            _ => None,
        };
        if linked.is_some() || scope.is_some() {
            let mut copied = guide.clone();
            let data = &mut state.scene.rulers;
            copied.id = data.next_id;
            data.next_id = data.next_id.checked_add(1).ok_or("Ruler ID limit")?;
            copied.name = name(&copied.name);
            copied.link = linked;
            copied.scope = scope.unwrap_or(copied.scope);
            data.guides.push(copied);
        }
    }
    crate::render_order::reconcile(&mut state);
    groups::finish(d, &state)?;
    Ok(Copied {
        state,
        keys: copies,
    })
}
