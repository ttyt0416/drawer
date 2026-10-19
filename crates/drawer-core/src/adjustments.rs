//! Nondestructive correction layers occupy the raster tree and affect their preceding backdrop.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_effects::Adjustment,
    layer_properties::{self as p, Target},
    sizing::DocumentState,
};
pub enum Destination {
    New { name: String, parent: Option<usize> },
    Existing(u64),
}
pub struct Prepared {
    pub state: DocumentState,
    pub layer: u64,
}
pub fn prepare(
    d: &Document,
    destination: Destination,
    value: Adjustment,
) -> Result<Prepared, String> {
    if !value.valid()
        || !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid correction parameters/document or snapshot limit".into());
    }
    let (mut state, id) = match destination {
        Destination::New { name, parent } => {
            if let Some(n) = parent
                && (n >= 256 || !d.scene.paper.folder_active(n))
            {
                return Err("Choose an existing parent folder".into());
            }
            let result = crate::layers::prepare(
                d,
                crate::layers::Command::Create {
                    kind: crate::layers::Kind::Raster,
                    name,
                    parent,
                },
            )?;
            let crate::layers::Target::Raster(id) = result.selected else {
                return Err("Correction layer kind mismatch".into());
            };
            (result.state, id)
        }
        Destination::Existing(id) => {
            if crate::layers::index(d, id).is_none()
                || crate::layer_effects::adjustment(d, Target::Raster(id)).is_none()
            {
                return Err("Select an existing correction layer".into());
            }
            if p::inherited(d, Target::Raster(id)).locked {
                return Err("The correction layer is locked".into());
            }
            (d.snapshot(), id)
        }
    };
    state
        .scene
        .layer_effects
        .rasters
        .entry(id)
        .or_default()
        .adjustment = Some(value);
    if !state.scene.valid()
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Correction scene/Undo budget".into());
    }
    p::validate_edit(d, &state)?;
    Ok(Prepared { state, layer: id })
}
