//! Atomic folder structure editing and grayscale layer masks.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_effects::{Effect, Mask},
    layer_properties::{self as p, Target},
    paper::{Folder, Node},
    sizing::DocumentState,
};
#[derive(Clone, Debug)]
pub enum Command {
    Create {
        name: String,
        parent: Option<usize>,
    },
    Wrap {
        name: String,
        targets: Vec<Target>,
    },
    Move {
        folder: usize,
        parent: Option<usize>,
        position: usize,
    },
    Unwrap(usize),
    Mode {
        folder: usize,
        through: bool,
    },
    Mask {
        target: Target,
        mask: Option<Mask>,
    },
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Target,
}
fn parent(d: &Document, n: Node) -> Option<usize> {
    d.scene
        .paper
        .folders
        .iter()
        .position(|f| f.children.contains(&n))
}
fn list(state: &mut DocumentState, parent: Option<usize>) -> Result<&mut Vec<Node>, String> {
    if let Some(n) = parent {
        Ok(&mut state
            .scene
            .paper
            .folders
            .get_mut(n)
            .ok_or("Missing parent folder")?
            .children)
    } else {
        Ok(&mut state.scene.paper.roots)
    }
}
fn writable(d: &Document, t: Target) -> Result<(), String> {
    let p = p::inherited(d, t);
    if p.locked || p.pixels_locked || p.position_locked {
        Err("Unlock this layer/folder before editing its structure or mask".into())
    } else {
        Ok(())
    }
}
fn name(n: &str) -> Result<String, String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 64 || n.chars().any(char::is_control) {
        Err("Folder name must contain 1–64 printable characters".into())
    } else {
        Ok(n.into())
    }
}
fn new_folder(state: &mut DocumentState, n: String, children: Vec<Node>) -> Result<usize, String> {
    let i = state.scene.paper.folders.len();
    if i >= 256 {
        return Err("Folder identity slot limit (256)".into());
    }
    state.scene.paper.folders.push(Folder {
        name: n,
        children,
        through: true,
        paper: Default::default(),
    });
    Ok(i)
}
fn member(d: &Document, root: Node, n: Node, depth: usize) -> bool {
    root == n
        || depth < 16
            && matches!(root,Node::Folder(i) if d.scene.paper.folders[i].children.iter().any(|c|member(d,*c,n,depth+1)))
}
pub fn prepare(d: &Document, command: Command) -> Result<Prepared, String> {
    if !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || !d.scene.layer_ids.matches(d.layers.len())
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || d.scene.clipping.solo.is_some()
    {
        return Err("Invalid folder document, snapshot limit, or active folder solo".into());
    }
    let folder_exists =
        |n: usize| n < 256 && n < d.scene.paper.folders.len() && d.scene.paper.folder_active(n);
    let missing = match &command {
        Command::Create { parent, .. } => parent.is_some_and(|n| !folder_exists(n)),
        Command::Move { folder, parent, .. } => {
            !folder_exists(*folder) || parent.is_some_and(|n| !folder_exists(n))
        }
        Command::Unwrap(n) | Command::Mode { folder: n, .. } => !folder_exists(*n),
        _ => false,
    };
    if missing {
        return Err("Folder index is missing or out of range".into());
    }
    let targets = p::targets(d);
    let valid = |t: Target| -> Result<(), String> {
        if targets.contains(&t) {
            writable(d, t)
        } else {
            Err("Layer/folder target is missing".into())
        }
    };
    if let Command::Mask { target, mask } = &command {
        valid(*target)?;
        if d.scene
            .layer_effects
            .get(*target)
            .and_then(|e| e.mask.as_ref())
            == mask.as_ref()
        {
            return Ok(Prepared {
                state: d.snapshot(),
                selected: *target,
            });
        }
    }
    if let Command::Mode { folder, through } = &command {
        valid(Target::Folder(*folder as u16))?;
        if d.scene.paper.folders[*folder].through == *through {
            return Ok(Prepared {
                state: d.snapshot(),
                selected: Target::Folder(*folder as u16),
            });
        }
    }
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    state.scene.paper.roots = state.scene.paper.root_nodes(d.layers.len());
    let selected = match command {
        Command::Create { name: n, parent } => {
            if let Some(n) = parent {
                valid(Target::Folder(n as u16))?;
            }
            let i = new_folder(&mut state, name(&n)?, vec![])?;
            list(&mut state, parent)?.push(Node::Folder(i));
            Target::Folder(i as u16)
        }
        Command::Wrap {
            name: n,
            targets: chosen,
        } => {
            if chosen.is_empty()
                || chosen.len() > 512
                || chosen
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != chosen.len()
            {
                return Err("Choose unique raster layers/folders to wrap".into());
            }
            let mut nodes = vec![];
            for t in &chosen {
                valid(*t)?;
                nodes.push(crate::clipping::node(d,*t).ok_or("Only raster layers/folders share this tree; animation tracks keep their own folder")?);
            }
            let par = parent(d, nodes[0]);
            if let Some(n) = par {
                valid(Target::Folder(n as u16))?;
            }
            if nodes.iter().any(|n| parent(d, *n) != par) {
                return Err("Wrap consecutive nodes with the same parent".into());
            }
            let siblings = list(&mut state, par)?.clone();
            nodes.sort_by_key(|n| siblings.iter().position(|v| v == n).unwrap());
            let start = siblings.iter().position(|v| *v == nodes[0]).unwrap();
            if siblings[start..start + nodes.len()] != nodes {
                return Err("Wrapped nodes must be consecutive to preserve their order".into());
            }
            let owned = |t: Target| {
                crate::clipping::node(d, t)
                    .is_some_and(|n| nodes.iter().any(|root| member(d, *root, n, 0)))
            };
            for t in &targets {
                if crate::clipping::active(d, *t)
                    && let Some(base) = d.scene.clipping.binding(*t)
                    && owned(*t) != owned(base)
                {
                    return Err("Wrap the complete clipping run with its base".into());
                }
            }
            let i = new_folder(&mut state, name(&n)?, nodes.clone())?;
            list(&mut state, par)?.splice(start..start + nodes.len(), [Node::Folder(i)]);
            Target::Folder(i as u16)
        }
        Command::Move {
            folder,
            parent: destination,
            position,
        } => {
            valid(Target::Folder(folder as u16))?;
            if let Some(n) = destination {
                valid(Target::Folder(n as u16))?;
                if member(d, Node::Folder(folder), Node::Folder(n), 0) {
                    return Err("A folder cannot move inside itself or its descendants".into());
                }
            }
            let par = parent(d, Node::Folder(folder));
            if let Some(n) = par {
                valid(Target::Folder(n as u16))?;
            }
            list(&mut state, par)?.retain(|n| *n != Node::Folder(folder));
            let dest = list(&mut state, destination)?;
            if position > dest.len() {
                return Err("Folder destination slot is out of range".into());
            }
            dest.insert(position, Node::Folder(folder));
            Target::Folder(folder as u16)
        }
        Command::Unwrap(folder) => {
            valid(Target::Folder(folder as u16))?;
            let f = d.scene.paper.folders.get(folder).ok_or("Missing folder")?;
            let par = parent(d, Node::Folder(folder));
            if let Some(n) = par {
                valid(Target::Folder(n as u16))?;
            }
            let effect = d.scene.layer_effects.get(Target::Folder(folder as u16));
            let prop = p::properties(d, Target::Folder(folder as u16));
            if effect.is_some_and(|e| e.mask.is_some())
                || prop.opacity != 100
                || !prop.blend.is_normal()
                || f.paper.pattern.effective()
                || f.paper.blend != crate::brush_composite::Mode::Normal
                || d.scene
                    .clipping
                    .binding(Target::Folder(folder as u16))
                    .is_some()
            {
                return Err("Folder mask, opacity, blend, paper or clipping needs raster preservation. Use Merge layers with the lower backdrop or Flatten page".into());
            }
            for child in &f.children {
                writable(d, crate::clipping::node_key(d, *child))?;
            }
            let dest = list(&mut state, par)?;
            let at = dest
                .iter()
                .position(|n| *n == Node::Folder(folder))
                .ok_or("Missing folder in parent")?;
            dest.splice(at..=at, f.children.clone());
            if f.paper.hidden {
                for child in &f.children {
                    let t = crate::clipping::node_key(d, *child);
                    let mut v = p::properties(d, t);
                    v.hidden = true;
                    p::set_state(d, &mut state, t, v)?;
                }
            }
            state.scene.paper.folders[folder].children.clear();
            state.scene.paper.folders[folder].paper = Default::default();
            state.scene.paper.folders[folder].through = false;
            state
                .scene
                .layer_properties
                .folders
                .remove(&(folder as u16));
            state.scene.layer_effects.folders.remove(&(folder as u16));
            state.scene.catalog.folders.remove(&(folder as u16));
            state
                .scene
                .clipping
                .remove_owner(Target::Folder(folder as u16));
            for g in &mut state.scene.rulers.guides {
                if g.scope == crate::ruler_scope::Scope::Folder(folder as u16) {
                    g.scope = par.map_or(crate::ruler_scope::Scope::All, |n| {
                        crate::ruler_scope::Scope::Folder(n as u16)
                    });
                }
            }
            compare(d, &state)?;
            f.children
                .first()
                .map_or(Target::Raster(state.scene.layer_ids.id(0)), |n| {
                    crate::clipping::node_key(d, *n)
                })
        }
        Command::Mode { folder, through } => {
            valid(Target::Folder(folder as u16))?;
            if through
                && (d
                    .scene
                    .clipping
                    .binding(Target::Folder(folder as u16))
                    .is_some()
                    || targets.iter().any(|t| {
                        crate::clipping::active(d, *t)
                            && d.scene.clipping.binding(*t) == Some(Target::Folder(folder as u16))
                    }))
            {
                return Err("Unclip the folder and its clipping run before through mode".into());
            }
            let f = state
                .scene
                .paper
                .folders
                .get_mut(folder)
                .ok_or("Missing folder")?;
            if through
                && (crate::paper::Paper {
                    hidden: false,
                    ..f.paper
                }) != Default::default()
            {
                return Err("Remove folder paper effects before selecting through mode".into());
            }
            f.through = through;
            if through {
                let mut prop = p::properties(d, Target::Folder(folder as u16));
                prop.blend = Default::default();
                state
                    .scene
                    .layer_properties
                    .set(Target::Folder(folder as u16), prop);
            }
            Target::Folder(folder as u16)
        }
        Command::Mask { target, mask } => {
            valid(target)?;
            let map = match target {
                Target::Raster(n) => {
                    let e = state.scene.layer_effects.rasters.entry(n).or_default();
                    e.mask = mask;
                    e
                }
                Target::Folder(n) => {
                    let e = state.scene.layer_effects.folders.entry(n).or_default();
                    e.mask = mask;
                    e
                }
                Target::Vector(n) => {
                    let e = state.scene.layer_effects.vectors.entry(n).or_default();
                    e.mask = mask;
                    e
                }
                _ => return Err("Masks attach to raster, vector and layer folders".into()),
            };
            if *map == Effect::default() {
                state.scene.layer_effects.remove(target);
            }
            target
        }
    };
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Folder depth, cycle, mask or scene/Undo limit".into());
    }
    p::validate_edit(d, &state)?;
    Ok(Prepared { state, selected })
}
fn compare(d: &Document, state: &DocumentState) -> Result<(), String> {
    let mut min = [0i64; 2];
    let mut max = [d.width as i64, d.height as i64];
    for l in &d.layers {
        let e = l.extent(d.width, d.height);
        for a in 0..2 {
            min[a] = min[a].min(e.offset[a]);
            max[a] = max[a].max(e.offset[a] + [e.width, e.height][a] as i64);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1]];
    if size.iter().any(|n| !(1..=32768).contains(n)) {
        return Err("Unwrap comparison extent limit; use raster preservation".into());
    }
    let count = size[0] as usize * size[1] as usize;
    if count > 4 * 1024 * 1024
        || count.saturating_mul(d.layers.len()).saturating_mul(2) > 64 * 1024 * 1024
    {
        return Err("Unwrap comparison work limit; use raster preservation".into());
    }
    let after = crate::layer_merge::preview(state, d.current_frame)?;
    let a = d.scene.paper.root_nodes(d.layers.len());
    let b = after.scene.paper.root_nodes(after.layers.len());
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let p = [x, y];
            if d.scene
                .paper
                .composite_nodes(d, &a, p)
                .iter()
                .zip(after.scene.paper.composite_nodes(&after, &b, p))
                .any(|(a, b)| a.abs_diff(b) > 1)
            {
                return Err("Unwrapping changes the outside backdrop or effects. Use Merge layers with its lower backdrop or Flatten page".into());
            }
        }
    }
    Ok(())
}
pub fn rectangle_mask(d: &Document, bounds: crate::scene::Rect, gray: u8) -> Result<Mask, String> {
    if !bounds.valid()
        || bounds
            .min
            .iter()
            .chain(bounds.max.iter())
            .any(|n| !n.is_finite() || n.abs() > 1e6)
        || bounds.min == bounds.max
    {
        return Err("Choose a finite nonempty mask rectangle".into());
    }
    let inv = d.scene.transform.inverse();
    let corners = [
        bounds.min,
        [bounds.max[0], bounds.min[1]],
        bounds.max,
        [bounds.min[0], bounds.max[1]],
    ]
    .map(|p| inv.map(p));
    let min: [i32; 2] = std::array::from_fn(|a| {
        corners
            .iter()
            .map(|p| p[a])
            .fold(f64::INFINITY, f64::min)
            .floor() as i32
    });
    let max: [i32; 2] = std::array::from_fn(|a| {
        corners
            .iter()
            .map(|p| p[a])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i32
    });
    let size = [max[0] as i64 - min[0] as i64, max[1] as i64 - min[1] as i64];
    if size.iter().any(|n| !(1..=32768).contains(n)) || size[0] * size[1] > 4 * 1024 * 1024 {
        return Err("Mask creation work limit".into());
    }
    let mut runs = vec![];
    if gray > 0 {
        for y in min[1]..max[1] {
            let mut start = None;
            let mut end = 0;
            for x in min[0]..max[0] {
                if bounds.contains(d.scene.transform.map([x as f64 + 0.5, y as f64 + 0.5])) {
                    start.get_or_insert(x);
                    end = x + 1;
                }
            }
            if let Some(x) = start {
                runs.push(crate::selection::Run {
                    x,
                    y,
                    length: (end - x) as u32,
                    alpha: gray,
                });
            }
        }
    }
    let coverage = crate::selection::Mask { runs };
    if !coverage.valid() {
        return Err("Mask coordinate/run limit".into());
    }
    Ok(Mask {
        coverage,
        offset: [0.; 2],
        enabled: true,
        linked: true,
        inverted: false,
    })
}
