//! Mask operations share the folder mask validator and one document transaction.
use crate::{
    document::Document,
    layer_effects::Mask,
    layer_properties::{self as p, Target},
    selection::Run,
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Create,
    Replace,
    Intersect,
}
pub enum Command {
    Invert,
    Enabled(bool),
    Linked(bool),
    Move([f64; 2]),
    Remove,
    Copy { source: Target, replace: bool },
    Selection { hide_inside: bool, policy: Policy },
    Apply,
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Target,
    pub notes: Vec<String>,
}
pub fn get(d: &Document, t: Target) -> Option<&Mask> {
    d.scene.layer_effects.get(t).and_then(|e| e.mask.as_ref())
}
fn writable(d: &Document, t: Target) -> Result<(), String> {
    if !matches!(t, Target::Raster(_) | Target::Vector(_) | Target::Folder(_))
        || !p::targets(d).contains(&t)
    {
        return Err("Choose a live raster, vector or layer folder".into());
    }
    let flags = p::inherited(d, t);
    if flags.locked || flags.pixels_locked || flags.position_locked {
        return Err("Unlock this layer and its parent before changing its mask".into());
    }
    Ok(())
}
fn bounds(mask: &Mask) -> Option<([f64; 2], [f64; 2])> {
    if mask.coverage.runs.is_empty() {
        return None;
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for r in &mask.coverage.runs {
        let a = [r.x as f64 + mask.offset[0], r.y as f64 + mask.offset[1]];
        let b = [a[0] + r.length as f64, a[1] + 1.];
        for k in 0..2 {
            min[k] = min[k].min(a[k]);
            max[k] = max[k].max(b[k]);
        }
    }
    Some((min, max))
}
fn raster(
    min: [f64; 2],
    max: [f64; 2],
    mut sample: impl FnMut([f64; 2]) -> u8,
) -> Result<crate::selection::Mask, String> {
    let min = min.map(f64::floor);
    let max = max.map(f64::ceil);
    if min
        .iter()
        .chain(&max)
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
    {
        return Err("Mask coordinate limit".into());
    }
    let size = [(max[0] - min[0]) as i64, (max[1] - min[1]) as i64];
    if size.iter().any(|v| !(1..=32768).contains(v)) || size[0] * size[1] > 4 * 1024 * 1024 {
        return Err("Mask sampling exceeds 4 million pixels or dimension limit".into());
    }
    let mut runs = vec![];
    for y in min[1] as i32..max[1] as i32 {
        let mut x = min[0] as i32;
        while x < max[0] as i32 {
            let start = x;
            let alpha = sample([x as f64 + 0.5, y as f64 + 0.5]);
            x += 1;
            while x < max[0] as i32 && sample([x as f64 + 0.5, y as f64 + 0.5]) == alpha {
                x += 1;
            }
            if alpha > 0 {
                if runs.len() >= 32768 {
                    return Err("Mask run limit".into());
                }
                runs.push(Run {
                    x: start,
                    y,
                    length: (x - start) as u32,
                    alpha,
                });
            }
        }
    }
    let result = crate::selection::Mask { runs };
    if !result.valid() {
        return Err("Invalid mask sample".into());
    }
    Ok(result)
}
fn selection(d: &Document, hide_inside: bool) -> Result<Mask, String> {
    let inv = d.scene.transform.inverse();
    let delta = d.scene.selection_translation;
    let offset = [
        inv.0[0] * delta[0] + inv.0[2] * delta[1],
        inv.0[1] * delta[0] + inv.0[3] * delta[1],
    ];
    let coverage = if let Some(mask) = &d.scene.selection_mask {
        mask.clone()
    } else if let Some(r) = d.scene.selection {
        raster(r.min, r.max, |p| if r.contains(p) { 255 } else { 0 })?
    } else {
        return Err("Create a nonempty area selection first".into());
    };
    if coverage.runs.is_empty() {
        return Err("Create a nonempty area selection first".into());
    }
    Ok(Mask {
        coverage,
        offset,
        enabled: true,
        linked: true,
        inverted: hide_inside,
    })
}
fn intersect(old: &Mask, new: &Mask) -> Result<Mask, String> {
    if !old.enabled {
        return Err("Enable the old mask or explicitly replace it".into());
    }
    let mut all = bounds(old).into_iter().chain(bounds(new));
    let Some((mut min, mut max)) = all.next() else {
        return Ok(Mask {
            coverage: Default::default(),
            offset: [0.; 2],
            enabled: true,
            linked: old.linked,
            inverted: old.inverted && new.inverted,
        });
    };
    for (a, b) in all {
        for k in 0..2 {
            min[k] = min[k].min(a[k]);
            max[k] = max[k].max(b[k]);
        }
    }
    let inverted = old.inverted && new.inverted;
    let coverage = raster(min, max, |p| {
        let a = (old.factor(p) * new.factor(p) * 255.).round() as u8;
        if inverted { 255 - a } else { a }
    })?;
    Ok(Mask {
        coverage,
        offset: [0.; 2],
        enabled: true,
        linked: old.linked,
        inverted,
    })
}
pub fn prepare(d: &Document, target: Target, command: Command) -> Result<Prepared, String> {
    writable(d, target)?;
    if matches!(command, Command::Apply) {
        let mask = get(d, target).ok_or("This layer has no mask")?;
        if !mask.enabled {
            return Err("Enable the mask before applying it to content".into());
        }
        let key = crate::layer_search::Key::Layer(target);
        let prepared = crate::raster_conversion::prepare(
            d,
            key,
            crate::raster_conversion::Action::Rasterize(crate::layer_search::Expression::Color),
        )?;
        let selected = crate::layer_catalog::state_target(&prepared.state, prepared.selected)
            .ok_or("Raster result missing")?;
        let mut notes = prepared.notes;
        notes.insert(0,"Apply mask rasterizes the current content, masks and display effects. Vector geometry/folder structure becomes raster content; one Undo restores it.".into());
        return Ok(Prepared {
            state: prepared.state,
            selected,
            notes,
        });
    }
    let old = get(d, target);
    let mut notes = vec![];
    let mask = match command {
        Command::Remove => {
            if old.is_none() {
                return Err("This layer has no mask".into());
            }
            None
        }
        Command::Copy { source, replace } => {
            if !p::targets(d).contains(&source) {
                return Err("Copy source was removed".into());
            }
            if old.is_some() && !replace {
                return Err("Choose Replace explicitly before overwriting a mask".into());
            }
            Some(get(d, source).ok_or("Copy source has no mask")?.clone())
        }
        Command::Selection {
            hide_inside,
            policy,
        } => {
            if old.is_some() && policy == Policy::Create {
                return Err("A mask already exists; explicitly choose Replace or Intersect".into());
            }
            let new = selection(d, hide_inside)?;
            if policy == Policy::Intersect {
                notes.push("Intersection samples the original pixel grid and multiplies strengths; layer linkage is preserved.".into());
                Some(intersect(
                    old.ok_or("Intersect requires an existing enabled mask")?,
                    &new,
                )?)
            } else {
                Some(new)
            }
        }
        Command::Invert | Command::Enabled(_) | Command::Linked(_) | Command::Move(_) => {
            let mut mask = old.ok_or("This layer has no mask")?.clone();
            match command {
                Command::Invert => mask.inverted = !mask.inverted,
                Command::Enabled(v) => mask.enabled = v,
                Command::Linked(v) => mask.linked = v,
                Command::Move(delta) => {
                    if delta.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.) {
                        return Err("Mask movement limit".into());
                    }
                    let inv = d.scene.transform.inverse();
                    mask.offset[0] += inv.0[0] * delta[0] + inv.0[2] * delta[1];
                    mask.offset[1] += inv.0[1] * delta[0] + inv.0[3] * delta[1];
                }
                _ => unreachable!(),
            };
            Some(mask)
        }
        Command::Apply => unreachable!(),
    };
    let state =
        crate::layer_folders::prepare(d, crate::layer_folders::Command::Mask { target, mask })?
            .state;
    Ok(Prepared {
        state,
        selected: target,
        notes,
    })
}
