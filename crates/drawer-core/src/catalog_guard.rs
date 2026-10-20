use crate::{document::Document, layer_catalog as c, layer_search::Key, scene::Scene};
pub fn validate(d: &Document, after: &Scene) -> Result<(), String> {
    for (n, old) in d.scene.objects.iter().enumerate() {
        let id = d.scene.catalog.objects.id(n);
        let p = c::properties(d, Key::Object(id));
        let new = after
            .catalog
            .objects
            .index(id, after.objects.len())
            .and_then(|n| after.objects.get(n));
        let renamed = d
            .scene
            .catalog
            .object_info
            .get(&id)
            .and_then(|i| i.name.as_ref())
            != after
                .catalog
                .object_info
                .get(&id)
                .and_then(|i| i.name.as_ref());
        if p.locked && (new.is_none_or(|v| v != old) || renamed) {
            return Err("Page object layer is locked".into());
        }
        if let Some(new) = new {
            let same_material_source = matches!((old,new),(crate::scene::PageObject::Material(a),crate::scene::PageObject::Material(b)) if a.source==b.source);
            if (p.hidden || p.pixels_locked && !same_material_source) && old != new {
                return Err("Page object layer is hidden or its image is locked".into());
            }
            if p.position_locked
                && matches!((old,new),(crate::scene::PageObject::Material(a),crate::scene::PageObject::Material(b)) if a.transform!=b.transform || a.area!=b.area || a.mapping.warp!=b.mapping.warp)
            {
                return Err("Material geometry is position locked".into());
            }
            let a = old.bounds();
            let b = new.bounds();
            if p.position_locked
                && a.min != b.min
                && (0..2).all(|i| (a.max[i] - a.min[i] - (b.max[i] - b.min[i])).abs() < 1e-9)
            {
                return Err("Page object position is locked".into());
            }
        }
    }
    for old in &d.scene.gradients.layers {
        let p = c::properties(d, Key::Gradient(old.id));
        let new = after.gradients.layers.iter().find(|g| g.id == old.id);
        let changed = new.is_some_and(|n| {
            n.settings != old.settings || n.creation != old.creation || n.areas != old.areas
        });
        if p.locked && (changed || new.is_none_or(|n| n.name != old.name)) {
            return Err("Gradient layer is locked".into());
        }
        if (p.pixels_locked || p.hidden) && changed {
            return Err("Gradient layer is hidden or its image is locked".into());
        }
        if p.position_locked
            && new.is_some_and(|n| {
                n.creation.0[..4] == old.creation.0[..4] && n.creation != old.creation
            })
        {
            return Err("Gradient position is locked".into());
        }
    }
    for (n, old) in d.scene.worlds.iter().enumerate() {
        let id = d.scene.catalog.worlds.id(n);
        let p = c::properties(d, Key::World(id));
        let new = after
            .catalog
            .worlds
            .index(id, after.worlds.len())
            .and_then(|n| after.worlds.get(n));
        let renamed = d
            .scene
            .catalog
            .world_info
            .get(&id)
            .and_then(|i| i.name.as_ref())
            != after
                .catalog
                .world_info
                .get(&id)
                .and_then(|i| i.name.as_ref());
        if p.locked && (new.is_none_or(|n| n != old) || renamed) {
            return Err("3D layer is locked".into());
        }
        if let Some(new) = new {
            if (p.pixels_locked || p.hidden) && new != old {
                return Err("3D layer is hidden or its image is locked".into());
            }
            if p.position_locked
                && new.vertices.len() == old.vertices.len()
                && !old.vertices.is_empty()
            {
                let delta: [f64; 3] =
                    std::array::from_fn(|i| new.vertices[0][i] - old.vertices[0][i]);
                if delta != [0.; 3]
                    && new
                        .vertices
                        .iter()
                        .zip(&old.vertices)
                        .all(|(a, b)| (0..3).all(|i| (a[i] - b[i] - delta[i]).abs() < 1e-9))
                {
                    return Err("3D layer position is locked".into());
                }
            }
        }
    }
    if d.scene.transform != after.transform
        && d.scene
            .catalog
            .object_info
            .values()
            .chain(d.scene.catalog.world_info.values())
            .chain(d.scene.catalog.gradients.values())
            .any(|i| i.properties.locked || i.properties.position_locked)
    {
        return Err("A locked page source prevents page geometry changes".into());
    }
    for old in &d.scene.selections.saved {
        let p = c::properties(d, Key::Selection(old.id));
        let new = after.selections.saved.iter().find(|s| s.id == old.id);
        if p.locked && new.is_none_or(|s| s != old) {
            return Err("Selection layer is locked".into());
        }
        if let Some(new) = new {
            if (p.pixels_locked || p.hidden) && old.shape != new.shape {
                return Err("Selection layer image is hidden or locked".into());
            }
            if p.position_locked && old.shape.translation != new.shape.translation {
                return Err("Selection layer position is locked".into());
            }
        }
    }
    Ok(())
}
