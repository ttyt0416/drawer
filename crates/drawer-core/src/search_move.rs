//! Recompute a picked source translation from the unchanged gesture source.
use crate::{
    document::Document, layer_catalog as c, layer_properties as p, layer_search::Key,
    sizing::DocumentState,
};
pub fn prepare(d: &Document, key: Key, delta: [i64; 2]) -> Result<DocumentState, String> {
    if c::thumbnail(d, key).is_none() {
        return Err("Movement source was removed".into());
    }
    if delta.iter().any(|v| v.unsigned_abs() > 1_000_000)
        || d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Movement coordinate/snapshot limit".into());
    }
    let t = c::target(d, key).ok_or("Movement source was removed")?;
    p::movable(d, t)?;
    let dp = delta.map(|v| v as f64);
    if let p::Target::Raster(id) = t {
        return crate::precision::Nudge::begin(
            d,
            crate::layers::index(d, id).ok_or("Missing raster")?,
            crate::precision::Target::Layer,
        )?
        .preview_total(d, delta);
    }
    if let p::Target::Vector(id) = t {
        return crate::ruler_scope::translate_layer(d, crate::line_tools::Target::Vector(id), dp);
    }
    let mut s = d.snapshot();
    if delta == [0; 2] {
        return Ok(s);
    }
    match t {
        p::Target::Object(id) => {
            let n = d
                .scene
                .catalog
                .objects
                .index(id, d.scene.objects.len())
                .ok_or("Missing page object")?;
            crate::alignment::shift(d, &mut s, crate::alignment::Item::Object(n), dp)?;
        }
        p::Target::Gradient(id) => {
            crate::alignment::shift(d, &mut s, crate::alignment::Item::Gradient(id), dp)?
        }
        p::Target::Cell { track, cell } => {
            let cell = &mut s.animation.as_mut().ok_or("Missing timeline")?.tracks[track as usize]
                .cells[cell as usize];
            for (a, b) in cell.offset.iter_mut().zip(delta) {
                *a = a.checked_add(b).ok_or("Cell position limit")?;
            }
        }
        p::Target::Track(track) => {
            let tr = &mut s.animation.as_mut().ok_or("Missing timeline")?.tracks[track as usize];
            for (n, cell) in tr.cells.iter_mut().enumerate() {
                p::movable(
                    d,
                    p::Target::Cell {
                        track,
                        cell: n as u16,
                    },
                )?;
                for (a, b) in cell.offset.iter_mut().zip(delta) {
                    *a = a.checked_add(b).ok_or("Cell position limit")?;
                }
            }
        }
        p::Target::World(id) => {
            let n = d
                .scene
                .catalog
                .worlds
                .index(id, d.scene.worlds.len())
                .ok_or("Missing 3D source")?;
            let inv = d.scene.transform.inverse();
            let scale = d
                .scene
                .camera
                .as_ref()
                .ok_or("Missing 3D camera")?
                .pixels_per_unit;
            let v = [
                (inv.0[0] * dp[0] + inv.0[2] * dp[1]) / scale,
                -(inv.0[1] * dp[0] + inv.0[3] * dp[1]) / scale,
            ];
            for p in &mut s.scene.worlds[n].vertices {
                p[0] += v[0];
                p[1] += v[1];
            }
        }
        p::Target::Folder(n) => {
            fn visit(
                d: &Document,
                s: &mut DocumentState,
                n: usize,
                dp: [f64; 2],
                depth: usize,
            ) -> Result<(), String> {
                if depth > 16 {
                    return Err("Folder movement depth limit".into());
                }
                p::movable(d, p::Target::Folder(n as u16))?;
                let inv = d.scene.transform.inverse();
                let v = [
                    inv.0[0] * dp[0] + inv.0[2] * dp[1],
                    inv.0[1] * dp[0] + inv.0[3] * dp[1],
                ];
                if !s.scene.paper.folders[n].through {
                    for (a, b) in s.scene.paper.folders[n].paper.origin.iter_mut().zip(v) {
                        *a += b;
                    }
                }
                if let Some(m) = s
                    .scene
                    .layer_effects
                    .folders
                    .get_mut(&(n as u16))
                    .and_then(|e| e.mask.as_mut())
                    .filter(|m| m.linked)
                {
                    for (a, b) in m.offset.iter_mut().zip(v) {
                        *a += b;
                    }
                }
                for node in d.scene.paper.folders[n].children.iter().copied() {
                    match node {
                        crate::paper::Node::Raster(i) => {
                            p::movable(d, p::raster(d, i))?;
                            crate::alignment::shift(d, s, crate::alignment::Item::Raster(i), dp)?;
                            s.scene.paper.shift_layer(i, v);
                        }
                        crate::paper::Node::Folder(i) => visit(d, s, i, dp, depth + 1)?,
                    }
                }
                for g in &mut s.scene.rulers.guides {
                    if g.scope == crate::ruler_scope::Scope::Folder(n as u16)
                        && let Some(old) = d.scene.rulers.guides.iter().find(|v| v.id == g.id)
                    {
                        g.offset = [old.offset[0] + v[0], old.offset[1] + v[1]];
                    }
                }
                Ok(())
            }
            visit(d, &mut s, n as usize, dp, 0)?;
        }
        _ => return Err("This source has no movable canvas content".into()),
    }
    if !s.scene.valid() || !s.scene.catalog.valid_animation(s.animation.as_ref()) {
        return Err("Moved source exceeds limits".into());
    }
    p::validate_edit(d, &s)?;
    Ok(s)
}
