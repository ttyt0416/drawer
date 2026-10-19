//! Bounded rasterization with explicit hidden/backdrop policy and atomic structure replacement.
use crate::{
    document::{
        Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer,
    },
    layer_properties::{self as p, Target},
    paper::{Graph, Node},
    sizing::DocumentState,
};
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub enum Operation {
    Selected(Vec<Target>),
    Down(Target),
    Flatten,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub include_hidden: bool,
    pub lower_backdrop: bool,
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Target,
    pub affected: Vec<Target>,
    pub notes: Vec<String>,
    pub maximum_delta: u8,
}
fn parent(g: &Graph, node: Node) -> Option<usize> {
    g.folders.iter().position(|f| f.children.contains(&node))
}
fn contains(g: &Graph, root: Node, node: Node, depth: usize) -> bool {
    root == node
        || depth < 16
            && matches!(root,Node::Folder(n) if g.folders[n].children.iter().any(|c|contains(g,*c,node,depth+1)))
}
pub fn preview(state: &DocumentState, frame: u32) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    state.restore(&mut d);
    d.current_frame = frame;
    Ok(d)
}
fn check(doc: &Document) -> Result<(), String> {
    if !doc.scene.valid()
        || !doc.scene.paper.valid_for(doc.layers.len())
        || doc.layers.is_empty()
        || doc.layers.len() > 256
        || doc.layers.iter().any(|l| !l.valid(doc.width, doc.height))
        || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || doc
            .resident_bytes()
            .saturating_add(doc.memory_bytes().saturating_mul(4))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Invalid merge document or 32 MiB snapshot limit".into());
    }
    if doc.scene.clipping.solo.is_some() {
        return Err("Restore folder solo visibility before merging".into());
    }
    Ok(())
}
fn unlock(doc: &Document, targets: &[Target]) -> Result<(), String> {
    crate::file_objects::validate_consumed(doc, targets)?;
    if targets.iter().any(|t| {
        let p = crate::file_objects::stored_flags(doc, *t);
        p.locked || p.pixels_locked || p.position_locked
    }) {
        Err("Unlock the affected layers/folders before rasterizing".into())
    } else {
        Ok(())
    }
}
fn unhide(doc: &Document, state: &mut DocumentState, targets: &[Target]) -> Result<(), String> {
    for t in targets {
        let mut prop = p::state_properties(state, *t);
        prop.hidden = false;
        p::set_state(doc, state, *t, prop)?;
    }
    Ok(())
}
fn finish(
    doc: &Document,
    mut state: DocumentState,
    selected: Target,
    affected: Vec<Target>,
    notes: Vec<String>,
    maximum_delta: u8,
) -> Result<Prepared, String> {
    crate::file_objects::prune(&mut state);
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || !state.scene.layer_ids.matches(state.layers.len())
        || state.memory_bytes().saturating_add(doc.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Merged document exceeds scene/Undo limits".into());
    }
    p::validate_edit(doc, &state)?;
    Ok(Prepared {
        state,
        selected,
        affected,
        notes,
        maximum_delta,
    })
}
pub fn prepare(
    doc: &Document,
    operation: Operation,
    settings: Settings,
) -> Result<Prepared, String> {
    check(doc)?;
    if matches!(operation, Operation::Flatten) {
        return flatten(doc, settings);
    }
    let graph = &doc.scene.paper;
    let (requested, down) = match operation {
        Operation::Selected(v) => (v, false),
        Operation::Down(t) => (vec![t], true),
        Operation::Flatten => unreachable!(),
    };
    if requested.is_empty()
        || requested.len() > 512
        || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()
    {
        return Err("Choose unique raster layers or folders".into());
    }
    let mut nodes=requested.iter().map(|t|crate::clipping::node(doc,*t).ok_or_else(||"Select raster layers/folders; use Flatten page for vectors, objects and animation".to_string())).collect::<Result<Vec<_>,_>>()?;
    let destination = parent(graph, nodes[0]);
    if nodes.iter().any(|n| parent(graph, *n) != destination) {
        return Err("Selected nodes must have the same parent".into());
    }
    let siblings = destination.map_or_else(
        || graph.root_nodes(doc.layers.len()),
        |n| graph.folders[n].children.clone(),
    );
    let position = |n: Node| {
        siblings
            .iter()
            .position(|v| *v == n)
            .ok_or("Missing merge sibling")
    };
    if down {
        let at = position(nodes[0])?;
        if at == 0 {
            return Err("No sibling below this layer".into());
        }
        nodes.insert(0, siblings[at - 1]);
    }
    if settings.lower_backdrop {
        let last = nodes
            .iter()
            .map(|n| position(*n))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .unwrap();
        nodes = siblings[..=last].to_vec();
    }
    if !settings.include_hidden {
        nodes.retain(|n| !p::properties(doc, crate::clipping::node_key(doc, *n)).hidden);
    }
    if nodes.len() < 2 {
        return Err("Choose at least two visible sibling nodes (or include hidden layers)".into());
    }
    nodes.sort_by_key(|n| siblings.iter().position(|v| v == n).unwrap());
    let start = position(nodes[0])?;
    if siblings[start..start + nodes.len()] != nodes {
        return Err("Select consecutive siblings or include their lower backdrop".into());
    }
    let members = p::targets(doc)
        .into_iter()
        .filter(|t| {
            crate::clipping::node(doc, *t)
                .is_some_and(|n| nodes.iter().any(|root| contains(graph, *root, n, 0)))
        })
        .collect::<Vec<_>>();
    if !settings.include_hidden && members.iter().any(|t| p::properties(doc, *t).hidden) {
        return Err("A selected folder contains hidden children; include hidden layers or select visible leaves".into());
    }
    unlock(doc, &members)?;
    for t in p::targets(doc) {
        if crate::clipping::active(doc, t)
            && let Some(base) = doc.scene.clipping.binding(t)
            && members.contains(&t) != members.contains(&base)
        {
            return Err("Include the complete clipping run and its base".into());
        }
    }
    let indices = members
        .iter()
        .filter_map(|t| {
            if let Target::Raster(id) = t {
                crate::layers::index(doc, *id)
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    if indices.is_empty() {
        return Err("The chosen folders contain no raster layers".into());
    }
    let roles = crate::layer_roles::common(doc, indices.iter().copied())?;
    let mut working_state = doc.snapshot();
    if settings.include_hidden {
        unhide(doc, &mut working_state, &members)?;
    }
    let working = preview(&working_state, doc.current_frame)?;
    drop(working_state);
    let appearance = crate::appearance::Caches::new(&working, None)?;
    let mut min = [i64::MAX; 2];
    let mut max = [i64::MIN; 2];
    for e in
        crate::file_objects::render_extents(&working, &working.scene.paper, &nodes, &appearance)?
    {
        for a in 0..2 {
            min[a] = min[a].min(e.offset[a]);
            max[a] = max[a].max(e.offset[a] + [e.width, e.height][a] as i64);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1]];
    if size.iter().any(|n| !(1..=32768).contains(n)) {
        return Err("Merged extent exceeds 32768 pixels".into());
    }
    let count = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .ok_or("Merged extent overflow")?;
    if count > 4 * 1024 * 1024
        || count.saturating_mul(doc.layers.len()).saturating_mul(3) > 64 * 1024 * 1024
    {
        return Err("Merge raster/comparison work limit".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| "Merge allocation failed")?;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            pixels.push(working.scene.paper.composite_nodes_with(
                &working,
                &nodes,
                [x, y],
                &mut |n, _| appearance.raster(&working, n, [x, y]),
            ));
        }
    }
    let raster = RasterLayer {
        locked: false,
        name: "Merged layers".into(),
        placement: Some(PlacedRaster {
            width: size[0] as u32,
            height: size[1] as u32,
            offset: min,
        }),
        pixels,
    };
    let mut after = doc.snapshot();
    after.scene.layer_ids.initialize(doc.layers.len())?;
    let first = *indices.first().unwrap();
    let folders = members
        .iter()
        .filter_map(|t| {
            if let Target::Folder(n) = t {
                Some(*n as usize)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let replace = |list: &mut Vec<Node>| {
        let mut out = vec![];
        for n in list.iter() {
            if *n == nodes[0] {
                out.push(Node::Raster(first));
            } else if !nodes.contains(n) {
                out.push(*n);
            }
        }
        *list = out;
    };
    after.scene.paper.roots = graph.root_nodes(doc.layers.len());
    if let Some(n) = destination {
        replace(&mut after.scene.paper.folders[n].children);
    } else {
        replace(&mut after.scene.paper.roots);
    }
    for n in &folders {
        after.scene.paper.folders[*n].children.clear();
        after.scene.paper.folders[*n].paper = Default::default();
        after.scene.paper.folders[*n].through = false;
    }
    for n in &indices {
        after.scene.paper.layers.remove(n);
    }
    for t in &members {
        after.scene.layer_properties.set(*t, Default::default());
        after.scene.clipping.remove_owner(*t);
        after.scene.layer_effects.remove(*t);
        after.scene.appearance.remove(*t);
        after
            .scene
            .catalog
            .set(crate::layer_search::Key::Layer(*t), Default::default());
    }
    let mut raster = Some(raster);
    let mut map = vec![None; doc.layers.len()];
    let mut layers = vec![];
    for (n, l) in doc.layers.iter().enumerate() {
        if n == first {
            map[n] = Some(layers.len());
            layers.push(raster.take().unwrap());
        } else if !indices.contains(&n) {
            map[n] = Some(layers.len());
            layers.push(l.clone());
        }
    }
    let replacement = map[first].unwrap();
    let rulers = std::mem::take(&mut after.scene.rulers);
    crate::layers::remap(&mut after.scene, &map);
    after.scene.rulers = rulers;
    let ruler_map: [usize; 256] =
        std::array::from_fn(|n| map.get(n).copied().flatten().unwrap_or(replacement));
    crate::ruler_scope::remap_rasters(
        &mut after.scene,
        &ruler_map,
        &indices.iter().copied().collect::<Vec<_>>(),
        replacement,
        &folders,
    );
    let id = after.scene.layer_ids.next;
    after.scene.layer_ids.next = id.checked_add(1).ok_or("Raster identity limit")?;
    after.scene.layer_ids.rasters[replacement] = id;
    after.layers = layers;
    crate::layer_roles::put_raster(&mut after, replacement, roles);
    let rendered = preview(&after, doc.current_frame)?;
    let after_appearance = crate::appearance::Caches::new(&rendered, None)?;
    let before_roots = working.scene.paper.root_nodes(working.layers.len());
    let after_roots = rendered.scene.paper.root_nodes(rendered.layers.len());
    let mut maximum_delta = 0;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let a = working.scene.paper.composite_nodes_with(
                &working,
                &before_roots,
                [x, y],
                &mut |n, _| appearance.raster(&working, n, [x, y]),
            );
            let b = rendered.scene.paper.composite_nodes_with(
                &rendered,
                &after_roots,
                [x, y],
                &mut |n, _| after_appearance.raster(&rendered, n, [x, y]),
            );
            for i in 0..4 {
                maximum_delta = maximum_delta.max(a[i].abs_diff(b[i]));
            }
        }
    }
    if maximum_delta > 1 {
        return Err("These layers depend on an outside backdrop. Include lower backdrop, merge the enclosing isolated folder with its background, or Flatten page".into());
    }
    finish(
        doc,
        after,
        Target::Raster(id),
        members,
        vec![
            format!(
                "{} sibling nodes rasterized; hidden layers {}",
                nodes.len(),
                if settings.include_hidden {
                    "included"
                } else {
                    "excluded and preserved"
                }
            ),
            "Original pixels, layer effects and structure return with one Undo.".into(),
        ],
        maximum_delta,
    )
}
fn flatten(doc: &Document, settings: Settings) -> Result<Prepared, String> {
    let affected = p::targets(doc);
    unlock(doc, &affected)?;
    let count = doc.width as usize * doc.height as usize;
    if count > 4 * 1024 * 1024
        || count.saturating_mul(doc.layers.len()).saturating_mul(2) > 64 * 1024 * 1024
    {
        return Err("Flatten raster/comparison work limit".into());
    }
    let mut source_state = doc.snapshot();
    if settings.include_hidden {
        unhide(doc, &mut source_state, &affected)?;
    }
    let source = preview(&source_state, doc.current_frame)?;
    drop(source_state);
    let (_, _, pixels) =
        crate::render::Renderer::new(&source).output(doc.dpi, DEFAULT_DOCUMENT_BUDGET / 8)?;
    let mut after = doc.snapshot();
    after.scene.layer_ids.initialize(doc.layers.len())?;
    let id = after.scene.layer_ids.next;
    after.scene.layer_ids.next = id.checked_add(1).ok_or("Raster identity limit")?;
    after.scene.layer_ids.rasters = vec![id];
    after.layers = vec![RasterLayer {
        locked: false,
        name: "Flattened page".into(),
        placement: None,
        pixels,
    }];
    crate::layer_catalog::clear_rendered(doc, &mut after)?;
    after.animation = None;
    after.scene.paper = Default::default();
    after.scene.clipping = Default::default();
    after.scene.layer_properties = Default::default();
    after.scene.layer_effects = Default::default();
    after.scene.lines = Default::default();
    after.scene.gradients = Default::default();
    after.scene.objects.clear();
    after.scene.worlds.clear();
    after.scene.camera = None;
    after.scene.clips.clear();
    after.scene.mouse_strokes.clear();
    after.scene.alignment = Default::default();
    after.scene.rulers.guides.retain(|g| {
        g.link.is_none()
            && !matches!(
                g.scope,
                crate::ruler_scope::Scope::Layer(_) | crate::ruler_scope::Scope::Folder(_)
            )
    });
    if after
        .scene
        .rulers
        .active
        .is_some_and(|id| !after.scene.rulers.guides.iter().any(|g| g.id == id))
    {
        after.scene.rulers.active = None;
    }
    finish(doc,after,Target::Raster(id),affected,vec![format!("Current frame {} becomes one raster layer; hidden layers {}.",doc.current_frame+1,if settings.include_hidden{"included"}else{"excluded"}),"Flatten bakes the visible canvas including draft layers, and removes off-canvas pixels, animation/audio, reference/draft layer roles and editable objects/effects. One Undo restores the original document structure.".into()],0)
}
