//! Movement tool targeting and persistent local exclusions, independent of selection pixel drags.
#[path = "layer_move_copy.rs"]
mod copy;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_groups::{self as groups, Operation},
    layer_properties::{self as p, Target},
    layer_search::{Key, Kind},
    scene::Point,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Current,
    Click,
    Area,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub source: Source,
    pub auto_select: bool,
    pub keep_original: bool,
    pub exclude_text: bool,
    pub exclude_balloon: bool,
    pub exclude_draft: bool,
    pub exclude_locked: bool,
    pub exclude_texture: bool,
    pub exclude_image: bool,
    pub exclude_fill: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source: Source::Current,
            auto_select: true,
            keep_original: false,
            exclude_text: false,
            exclude_balloon: false,
            exclude_draft: false,
            exclude_locked: true,
            exclude_texture: false,
            exclude_image: false,
            exclude_fill: false,
        }
    }
}
pub fn eligible(d: &Document, key: Key, settings: Settings) -> bool {
    let Ok(rows) = c::rows(d) else {
        return false;
    };
    rows.iter()
        .find(|r| r.key == key)
        .is_some_and(|row| eligible_row(d, key, settings, row.kind))
}
fn eligible_row(d: &Document, key: Key, settings: Settings, kind: Kind) -> bool {
    let Some(t) = c::target(d, key) else {
        return false;
    };
    let flags = p::inherited(d, t);
    if flags.hidden || settings.exclude_locked && (flags.locked || flags.position_locked) {
        return false;
    }
    if settings.exclude_text && kind == Kind::Text
        || settings.exclude_balloon && kind == Kind::Balloon
        || settings.exclude_draft && crate::layer_roles::flags(d, t).draft
        || settings.exclude_image
            && (kind == Kind::Image || d.scene.file_objects.get(key).is_some())
        || settings.exclude_fill && matches!(kind, Kind::Gradient | Kind::Solid)
    {
        return false;
    }
    let texture = match t {
        Target::Raster(id) => crate::layers::index(d, id)
            .and_then(|n| d.scene.paper.layers.get(&n))
            .is_some_and(|p| p.pattern.effective()),
        Target::Folder(n) => d
            .scene
            .paper
            .folders
            .get(n as usize)
            .is_some_and(|f| f.paper.pattern.effective()),
        _ => false,
    };
    !(settings.exclude_texture
        && (texture
            || d.scene
                .appearance
                .get(t)
                .is_some_and(crate::appearance::Style::active)))
}
pub fn resolve(
    d: &Document,
    current: Key,
    point: Point,
    settings: Settings,
) -> Result<Vec<Key>, String> {
    if point.iter().any(|v| !v.is_finite()) || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Movement targeting coordinate/snapshot limit".into());
    }
    let rows = c::rows(d)?
        .into_iter()
        .map(|r| (r.key, r.kind))
        .collect::<std::collections::BTreeMap<_, _>>();
    let eligible = |key: Key| {
        rows.get(&key)
            .is_some_and(|kind| eligible_row(d, key, settings, *kind))
    };
    let keys = match settings.source {
        Source::Current => {
            if !eligible(current) {
                return Err("The current layer is hidden or excluded by movement settings".into());
            }
            vec![current]
        }
        Source::Click => {
            let picked = crate::layer_pick::candidates(
                d,
                point,
                crate::layer_pick::Settings {
                    include_locked: true,
                    ..Default::default()
                },
            );
            let key = picked
                .into_iter()
                .filter_map(|t| c::from_thumbnail(d, t))
                .find(|k| eligible(*k))
                .ok_or("No eligible layer at this point")?;
            vec![key]
        }
        Source::Area => area(d, settings, &rows)?,
    };
    let expanded = groups::expanded(d, &keys, Operation::Move)?;
    if expanded.iter().any(|k| !eligible(*k)) {
        return Err("A linked member is excluded by movement settings; no members changed".into());
    }
    groups::preflight(d, &expanded, Operation::Move)?;
    Ok(expanded)
}
fn area(
    d: &Document,
    settings: Settings,
    rows: &std::collections::BTreeMap<Key, Kind>,
) -> Result<Vec<Key>, String> {
    if d.scene.selection_empty() {
        return Err("Create a nonempty area selection to choose intersecting layers".into());
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut add = |a: Point, b: Point| {
        for p in [a, [b[0], a[1]], b, [a[0], b[1]]].map(|p| d.scene.selection_point(p)) {
            for k in 0..2 {
                min[k] = min[k].min(p[k]).floor();
                max[k] = max[k].max(p[k]).ceil();
            }
        }
    };
    if let Some(mask) = &d.scene.selection_mask {
        for r in &mask.runs {
            add(
                [r.x as f64, r.y as f64],
                [r.x as f64 + r.length as f64, r.y as f64 + 1.],
            );
        }
    } else if let Some(r) = d.scene.selection {
        add(r.min, r.max);
    }
    let keys = crate::layer_pick::visible(
        d,
        crate::layer_pick::Settings {
            include_locked: true,
            ..Default::default()
        },
    )
    .into_iter()
    .filter_map(|t| c::from_thumbnail(d, t))
    .filter(|k| {
        rows.get(k)
            .is_some_and(|kind| eligible_row(d, *k, settings, *kind))
    })
    .collect::<Vec<_>>();
    let size = [max[0] - min[0], max[1] - min[1]];
    let count = size[0] * size[1];
    if min
        .iter()
        .chain(&max)
        .any(|v| !v.is_finite() || v.abs() > 1e6)
        || size.iter().any(|v| !(1. ..=32768.).contains(v))
        || count > 4. * 1024. * 1024.
        || count * keys.len().max(1) as f64 > 64. * 1024. * 1024.
    {
        return Err("Area movement targeting work limit".into());
    }
    let renderer = crate::render::Renderer::new(d);
    if let Some(e) = renderer.error() {
        return Err(e.to_string());
    }
    let mut found = vec![];
    for key in keys {
        let mut hit = false;
        for y in min[1] as i64..max[1] as i64 {
            for x in min[0] as i64..max[0] as i64 {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                if d.scene.selection_coverage(p) > 0
                    && renderer.source_pixel_unbounded(key, p)[3] > 0
                {
                    hit = true;
                    break;
                }
            }
            if hit {
                break;
            }
        }
        if hit {
            found.push(key);
        }
    }
    if found.is_empty() {
        return Err("No eligible layer content intersects this selection".into());
    }
    Ok(found)
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Key,
}
pub fn prepare(
    d: &Document,
    keys: &[Key],
    delta: [i64; 2],
    keep_original: bool,
) -> Result<Prepared, String> {
    if delta.iter().any(|v| v.unsigned_abs() > 1_000_000) {
        return Err("Movement coordinate limit".into());
    }
    let keys = groups::expanded(d, keys, Operation::Move)?;
    groups::preflight(d, &keys, Operation::Move)?;
    if delta == [0; 2] {
        return Ok(Prepared {
            state: d.snapshot(),
            selected: keys[0],
        });
    }
    if !keep_original {
        return Ok(Prepared {
            state: groups::move_linked(d, &keys, delta)?,
            selected: keys[0],
        });
    }
    let copied = copy::prepare(d, &groups::roots(d, &keys))?;
    let working = crate::layer_merge::preview(&copied.state, d.current_frame)?;
    let state = groups::move_linked(&working, &copied.keys, delta)?;
    groups::finish(d, &state)?;
    Ok(Prepared {
        state,
        selected: copied.keys[0],
    })
}
pub fn copy_selected_pixels(d: &Document, n: usize, delta: [i64; 2]) -> Result<Prepared, String> {
    if n >= d.layers.len() || delta.iter().any(|v| v.unsigned_abs() > 1_000_000) {
        return Err("Invalid selected-pixel copy source or movement limit".into());
    }
    if delta == [0; 2] {
        return Ok(Prepared {
            state: d.snapshot(),
            selected: Key::Layer(p::raster(d, n)),
        });
    }
    p::paintable(d, n)?;
    let image = crate::selection_transfer::copy(d, n, DEFAULT_DOCUMENT_BUDGET / 8)?;
    let origin = image.origin.ok_or("Missing selected pixel origin")?;
    let offset = [
        origin[0]
            .checked_add(delta[0])
            .ok_or("Copy position overflow")?,
        origin[1]
            .checked_add(delta[1])
            .ok_or("Copy position overflow")?,
    ];
    let mut state = crate::paste::prepare(d, &image.image, offset, DEFAULT_DOCUMENT_BUDGET)?;
    state.scene.layer_ids.initialize(state.layers.len())?;
    state.scene.layer_ids.append(state.layers.len())?;
    let selected = Key::Layer(Target::Raster(
        state.scene.layer_ids.id(state.layers.len() - 1),
    ));
    state.layers.last_mut().unwrap().name = "Selected pixel copy".into();
    for (a, b) in state.scene.selection_translation.iter_mut().zip(delta) {
        *a += b as f64;
    }
    c::reconcile(d, &mut state)?;
    groups::finish(d, &state)?;
    Ok(Prepared { state, selected })
}
