//! An ordinary output raster below existing ink; checker colors never enter this state.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    layers::{self, Command, Kind},
    paper::Node,
    sizing::DocumentState,
};
pub fn checker_default() -> [[u8; 3]; 2] {
    [[196; 3], [224; 3]]
}
pub fn valid_live(d: &Document) -> bool {
    d.scene
        .paper_background
        .is_none_or(|id| layers::index(d, id).is_some())
}
pub(crate) fn prune(s: &mut DocumentState) {
    if s.scene
        .paper_background
        .is_some_and(|id| !(0..s.layers.len()).any(|n| s.scene.layer_ids.id(n) == id))
    {
        s.scene.paper_background = None;
    }
}
pub fn prepare(d: &Document, color: [u8; 3], visible: bool) -> Result<DocumentState, String> {
    let count = (d.width as usize).saturating_mul(d.height as usize);
    if count > 4 * 1024 * 1024 || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Paper background exceeds 4Mi pixels or snapshot budget".into());
    }
    let mut state;
    let id;
    if let Some(current) = d.scene.paper_background {
        id = current;
        let target = Target::Raster(id);
        let flags = p::inherited(d, target);
        if flags.locked || flags.pixels_locked || flags.position_locked {
            return Err("Unlock the paper background before replacing its pixels".into());
        }
        state = d.snapshot();
    } else {
        let prepared = layers::prepare(
            d,
            Command::Create {
                kind: Kind::Raster,
                name: "Paper background".into(),
                parent: None,
            },
        )?;
        let layers::Target::Raster(created) = prepared.selected else {
            unreachable!()
        };
        id = created;
        state = prepared.state;
        let n = (0..state.layers.len())
            .find(|n| state.scene.layer_ids.id(*n) == id)
            .ok_or("Missing new paper layer")?;
        state
            .scene
            .paper
            .roots
            .retain(|node| *node != Node::Raster(n));
        state.scene.paper.roots.insert(0, Node::Raster(n));
    }
    let n = (0..state.layers.len())
        .find(|n| state.scene.layer_ids.id(*n) == id)
        .ok_or("Missing paper layer")?;
    state.layers[n].pixels = vec![[color[0], color[1], color[2], 255]; count];
    state.layers[n].placement = None;
    state.layers[n].precise =
        (d.color.depth == crate::color_precision::Depth::Sixteen).then(|| {
            vec![
                [
                    color[0] as u16 * 257,
                    color[1] as u16 * 257,
                    color[2] as u16 * 257,
                    65535
                ];
                count
            ]
        });
    state.scene.paper_background = Some(id);
    // Paper color is a replacement of the complete stored rectangle. Visibility remains a layer property.
    let mut paper = state
        .scene
        .paper
        .layers
        .get(&n)
        .copied()
        .unwrap_or_default();
    paper.hidden = !visible;
    if paper == Default::default() {
        state.scene.paper.layers.remove(&n);
    } else {
        state.scene.paper.layers.insert(n, paper);
    }
    if let Some(properties) = state.scene.layer_properties.rasters.get_mut(&id) {
        properties.hidden = !visible;
    }
    if !state.scene.valid()
        || !state.scene.paper.valid_for(state.layers.len())
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Paper background exceeds document/Undo limits".into());
    }
    crate::clipping::validate_state(d, &state)?;
    p::validate_edit(d, &state)?;
    Ok(state)
}
