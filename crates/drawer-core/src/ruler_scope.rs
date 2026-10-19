//! Page-owned layer/frame scopes and explicit linked-layer movement.
use crate::{
    document::Document,
    line_tools::Target,
    scene::{Affine, Point, Scene},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    All,
    Layer(Target),
    Folder(u16),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    #[default]
    All,
    AppliedLayers,
    Hidden,
}
pub fn target_valid(t: Target) -> bool {
    match t {
        Target::Raster(i) => i < 256,
        Target::Vector(id) => id > 0,
        _ => false,
    }
}
impl Scope {
    pub fn valid(self) -> bool {
        match self {
            Self::All => true,
            Self::Layer(t) => target_valid(t),
            Self::Folder(i) => i < 256,
        }
    }
    pub fn matches(self, doc: &Document, target: Target) -> bool {
        match self {
            Self::All => true,
            Self::Layer(t) => t == target,
            Self::Folder(i) => match target {
                Target::Raster(n) => {
                    doc.scene.paper.folders.get(i as usize).is_some()
                        && doc
                            .scene
                            .paper
                            .raster_indices(crate::paper::Node::Folder(i as usize))
                            .contains(&n)
                }
                _ => false,
            },
        }
    }
}
pub fn linked_visible(doc: &Document, target: Target) -> bool {
    match target {
        Target::Raster(n) => {
            n < doc.layers.len() && doc.scene.paper.raster_visible(n, doc.layers.len())
        }
        Target::Vector(id) => doc
            .scene
            .lines
            .layers
            .iter()
            .any(|l| l.id == id && !l.hidden),
        _ => false,
    }
}
pub fn transform(doc: &Document, g: &crate::rulers::Guide) -> Option<Affine> {
    let t = Affine([1.0, 0.0, 0.0, 1.0, g.offset[0], g.offset[1]]).then(doc.scene.transform);
    t.valid().then_some(t)
}
pub fn applies(doc: &Document, g: &crate::rulers::Guide, target: Target) -> bool {
    g.enabled
        && g.scope.matches(doc, target)
        && g.frames
            .is_none_or(|r| doc.current_frame >= r[0] && doc.current_frame <= r[1])
        && g.link.is_none_or(|t| linked_visible(doc, t))
}
pub fn shift(scene: &mut Scene, target: Target, delta: Point) -> Result<(), String> {
    let inverse = scene.transform.inverse();
    let p = inverse.map(delta);
    let zero = inverse.map([0.0; 2]);
    let delta = [p[0] - zero[0], p[1] - zero[1]];
    for g in &mut scene.rulers.guides {
        if g.link == Some(target) {
            g.offset = [g.offset[0] + delta[0], g.offset[1] + delta[1]];
            if g.offset.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
                return Err("Linked ruler movement exceeds coordinate limit".into());
            }
        }
    }
    Ok(())
}
pub fn remap_rasters(
    scene: &mut Scene,
    map: &[usize; 256],
    consumed: &[usize],
    replacement: usize,
    folders: &[usize],
) {
    for g in &mut scene.rulers.guides {
        let raster = |n: usize| {
            if consumed.contains(&n) {
                replacement
            } else if n > 0 && map[n] == 0 {
                n
            } else {
                map[n]
            }
        };
        g.scope = match g.scope {
            Scope::Layer(Target::Raster(n)) => Scope::Layer(Target::Raster(raster(n))),
            Scope::Folder(n) if folders.contains(&(n as usize)) => {
                Scope::Layer(Target::Raster(replacement))
            }
            s => s,
        };
        if let Some(Target::Raster(n)) = g.link {
            g.link = Some(Target::Raster(raster(n)));
        }
    }
}
pub fn translate_layer(
    doc: &Document,
    target: Target,
    delta: Point,
) -> Result<DocumentState, String> {
    if !linked_visible(doc, target) {
        return Err("Layer is hidden or missing".into());
    }
    if delta.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
        || doc.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid linked layer movement or snapshot limit".into());
    }
    match target {
        Target::Raster(layer) => {
            let nudge =
                crate::precision::Nudge::begin(doc, layer, crate::precision::Target::Layer)?;
            let rounded = delta.map(|v| v.round() as i64);
            let state = nudge.preview_total(doc, rounded)?;
            Ok(state)
        }
        Target::Vector(id) => {
            let layer = doc
                .scene
                .lines
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Vector layer is missing")?;
            if layer.hidden || layer.locked {
                return Err("Vector layer is hidden/locked".into());
            }
            let mut state = doc.snapshot();
            let inverse = state.scene.transform.inverse();
            let zero = inverse.map([0.0; 2]);
            let d = inverse.map(delta);
            let d = [d[0] - zero[0], d[1] - zero[1]];
            let original_move = Affine([1.0, 0.0, 0.0, 1.0, -d[0], -d[1]]);
            for path in &mut state
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap()
                .paths
            {
                for p in &mut path.points {
                    p[0] += d[0];
                    p[1] += d[1];
                }
                if let Some(def) = &mut path.definition {
                    def.creation = original_move.then(def.creation);
                }
            }
            shift(&mut state.scene, target, delta)?;
            if !state.scene.valid()
                || serde_json::to_vec(&state.scene)
                    .map_err(|e| e.to_string())?
                    .len()
                    > 4 * 1024 * 1024
            {
                return Err("Moved layer/ruler exceeds scene budget".into());
            }
            Ok(state)
        }
        _ => Err("Choose a raster or vector layer to move".into()),
    }
}
pub fn follow(
    doc: &Document,
    id: u64,
    target: crate::shapes::Target,
    width: f64,
    color: [u8; 4],
) -> Result<DocumentState, String> {
    follow_part(doc, id, target, width, color, 0)
}
pub fn follow_part(
    doc: &Document,
    id: u64,
    target: crate::shapes::Target,
    width: f64,
    color: [u8; 4],
    part: usize,
) -> Result<DocumentState, String> {
    let guide = doc
        .scene
        .rulers
        .guides
        .iter()
        .find(|g| g.id == id)
        .ok_or("Ruler is missing")?;
    let layer = match target {
        crate::shapes::Target::Raster(i) => Target::Raster(i),
        crate::shapes::Target::Vector(Some(id)) => Target::Vector(id),
        _ => return Err("Choose a raster or vector layer first".into()),
    };
    if !applies(doc, guide, layer) {
        return Err("This ruler does not apply to the chosen layer/frame".into());
    }
    let (points, closed) = path(doc, id, part)?;
    let settings = crate::shapes::Settings {
        kind: crate::shapes::Kind::Polyline,
        width: width / crate::line_tools::maximum_scale(doc.scene.transform),
        line: color,
        closed,
        ..Default::default()
    };
    crate::shapes::prepare(doc, target, &points, settings)
}

pub fn path(doc: &Document, id: u64, part: usize) -> Result<(Vec<Point>, bool), String> {
    let guide = doc
        .scene
        .rulers
        .guides
        .iter()
        .find(|g| g.id == id)
        .ok_or("Ruler is missing")?;
    let transform = transform(doc, guide).ok_or("Linked ruler layer is hidden/missing")?;
    let (points, closed) = match &guide.kind {
        crate::rulers::Kind::Special(s) => s.follow_part(part)?,
        crate::rulers::Kind::Straight { a, b } | crate::rulers::Kind::Mirror { a, b } => {
            (vec![*a, *b], false)
        }
        _ => {
            return Err(
                "Choose a finite straight/curve/circle or perspective guide to follow".into(),
            );
        }
    };
    let points = points
        .into_iter()
        .map(|p| transform.map(p))
        .collect::<Vec<_>>();
    Ok((points, closed))
}
