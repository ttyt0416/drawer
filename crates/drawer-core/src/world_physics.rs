//! Explicit, deterministic placement commands. Simulation state is never stored.
use crate::{
    document::Document,
    layer_search::Key,
    model_library as lib,
    world_math::add,
    world3d::{self, V},
};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};
#[derive(Clone, Debug)]
pub enum Edit {
    Collider {
        ids: Vec<u64>,
        enabled: bool,
    },
    Floor {
        ids: Vec<u64>,
        height: f64,
    },
    Surface {
        ids: Vec<u64>,
        distance: f64,
    },
    Array {
        ids: Vec<u64>,
        counts: [u16; 3],
        spacing: V,
    },
    Settle {
        ids: Vec<u64>,
        floor: f64,
        gravity: f64,
        seconds: f64,
        timestep: f64,
    },
}
#[derive(Clone, Copy, Debug)]
struct Box3 {
    lo: V,
    hi: V,
}
impl Box3 {
    fn shift(&mut self, delta: f64) {
        self.lo[1] += delta;
        self.hi[1] += delta;
    }
    fn footprint(self, b: Self) -> bool {
        [0, 2]
            .into_iter()
            .all(|i| self.hi[i] > b.lo[i] + 1e-8 && b.hi[i] > self.lo[i] + 1e-8)
    }
    fn overlaps(self, b: Self) -> bool {
        self.footprint(b) && self.hi[1] > b.lo[1] + 1e-8 && b.hi[1] > self.lo[1] + 1e-8
    }
}
fn box_for(s: &crate::scene::Scene, ids: &BTreeSet<u64>) -> Result<Box3, String> {
    let (lo, hi) = lib::bounds(s, ids)?;
    Ok(Box3 { lo, hi })
}
struct Body {
    ids: BTreeSet<u64>,
    bounds: Box3,
    initial: Box3,
    velocity: f64,
}
fn moving(d: &Document, ids: &[u64]) -> Result<Vec<Body>, String> {
    let roots = lib::roots(&d.scene, ids)?;
    let bodies = roots
        .into_iter()
        .map(|id| {
            let ids = lib::tree(&d.scene, id);
            lib::editable(d, &ids)?;
            let bounds = box_for(&d.scene, &ids)?;
            Ok(Body {
                ids,
                bounds,
                initial: bounds,
                velocity: 0.,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(bodies)
}
fn finite(v: f64) -> bool {
    v.is_finite() && v.abs() <= 1e6
}
fn fits(memory: usize, scene: &crate::scene::Scene, limit: usize) -> bool {
    let triangles = scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    triangles <= 262144
        && memory
            .saturating_mul(3)
            .saturating_add(triangles.saturating_mul(512))
            .saturating_add(1024 * 1024)
            <= limit
}
fn budget(state: &crate::sizing::DocumentState, limit: usize) -> bool {
    fits(state.memory_bytes(), &state.scene, limit)
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if !fits(d.memory_bytes(), &d.scene, limit) {
        return Err("Placement snapshot / geometry budget exceeded".into());
    }
    let mut state = d.snapshot();
    let mut affected = vec![];
    let mut warnings = vec![];
    let summary = match edit {
        Edit::Collider { ids, enabled } => {
            for root in lib::roots(&d.scene, &ids)? {
                for id in lib::tree(&d.scene, root) {
                    if enabled {
                        if d.scene.file_objects.get(Key::World(id)).is_some() {
                            return Err(
                                "Detach file-object replacement before marking its collider".into(),
                            );
                        }
                        state.scene.world3d.colliders.insert(id);
                    } else {
                        state.scene.world3d.colliders.remove(&id);
                    }
                    affected.push(id);
                }
            }
            "Set collider roles (world AABB proxies; no motion)".into()
        }
        Edit::Floor { ids, height } => {
            if !finite(height) {
                return Err("Invalid floor height".into());
            }
            for body in moving(d, &ids)? {
                lib::shift(
                    &mut state.scene,
                    &body.ids,
                    [0., height - body.bounds.lo[1], 0.],
                )?;
                affected.extend(body.ids);
            }
            "Snap tree bottom bounds to world floor".into()
        }
        Edit::Surface { ids, distance } => {
            if !distance.is_finite() || !(0.0001..=1e6).contains(&distance) {
                return Err("Invalid downward surface search distance".into());
            }
            let bodies = moving(d, &ids)?;
            let moving = bodies
                .iter()
                .flat_map(|b| b.ids.iter().copied())
                .collect::<BTreeSet<_>>();
            let targets = d
                .scene
                .world3d
                .colliders
                .iter()
                .filter(|id| !moving.contains(id))
                .filter_map(|id| d.scene.catalog.worlds.index(*id, d.scene.worlds.len()))
                .collect::<BTreeSet<_>>();
            if targets.is_empty() {
                return Err("Mark another model as a collider / polygon snap target first".into());
            }
            let trace = crate::world_render::Trace::surfaces(d);
            for body in bodies {
                crate::book_io::check_cancel(cancel)?;
                let anchor = [
                    (body.bounds.lo[0] + body.bounds.hi[0]) / 2.,
                    body.bounds.lo[1],
                    (body.bounds.lo[2] + body.bounds.hi[2]) / 2.,
                ];
                let epsilon = 1e-5;
                let origin = add(anchor, [0., epsilon, 0.]);
                let hit = trace
                    .cast(origin, [0., -1., 0.], distance + epsilon, &targets)
                    .ok_or("No visible polygon surface below the bottom-center anchor")?;
                lib::shift(&mut state.scene, &body.ids, [0., hit[1] - anchor[1], 0.])?;
                affected.extend(body.ids);
            }
            "Snap bottom-center anchors downward to polygon targets; preserve orientation".into()
        }
        Edit::Array {
            ids,
            counts,
            spacing,
        } => {
            if counts.into_iter().any(|n| n == 0 || n > 16)
                || spacing.into_iter().any(|v| !finite(v))
            {
                return Err("Array counts1..16 and finite spacing required".into());
            }
            let bodies = moving(d, &ids)?;
            let all = bodies
                .into_iter()
                .flat_map(|b| b.ids)
                .collect::<BTreeSet<_>>();
            let count = counts.into_iter().map(usize::from).product::<usize>();
            if d.scene.worlds.len() + all.len() * (count - 1) > 256 {
                return Err("Array exceeds model limit256".into());
            }
            let keys = all.iter().copied().map(Key::World).collect::<Vec<_>>();
            for z in 0..counts[2] {
                for y in 0..counts[1] {
                    for x in 0..counts[0] {
                        if [x, y, z] == [0; 3] {
                            continue;
                        }
                        crate::book_io::check_cancel(cancel)?;
                        let mut frozen = crate::layer_merge::preview(&state, d.current_frame)?;
                        frozen.keyframe_content = d.keyframe_content;
                        let (mut next, new) = crate::layer_move::copy_sources(&frozen, &keys)?;
                        let new = new
                            .into_iter()
                            .filter_map(|k| {
                                if let Key::World(id) = k {
                                    Some(id)
                                } else {
                                    None
                                }
                            })
                            .collect::<BTreeSet<_>>();
                        lib::shift(
                            &mut next.scene,
                            &new,
                            std::array::from_fn(|i| [x, y, z][i] as f64 * spacing[i]),
                        )?;
                        if !budget(&next, limit) {
                            return Err("Array result exceeds geometry / memory limits".into());
                        }
                        affected.extend(&new);
                        state = next;
                    }
                }
            }
            format!(
                "Array {counts:?}; {} independent new objects; original at0,0,0",
                affected.len()
            )
        }
        Edit::Settle {
            ids,
            floor,
            gravity,
            seconds,
            timestep,
        } => {
            if !finite(floor)
                || !gravity.is_finite()
                || !(0. ..=100.).contains(&gravity)
                || !seconds.is_finite()
                || !(0. ..=10.).contains(&seconds)
                || !timestep.is_finite()
                || !(1e-6..=1e6).contains(&timestep)
            {
                return Err("Invalid floor / gravity / duration / timestep".into());
            }
            let mut bodies = moving(d, &ids)?;
            if bodies.len() > 64 {
                return Err("Settle at most64 moving trees at once".into());
            }
            bodies.sort_by(|a, b| {
                a.bounds.lo[1]
                    .total_cmp(&b.bounds.lo[1])
                    .then_with(|| a.ids.first().cmp(&b.ids.first()))
            });
            let all = bodies
                .iter()
                .flat_map(|b| b.ids.iter().copied())
                .collect::<BTreeSet<_>>();
            let static_boxes = d
                .scene
                .world3d
                .colliders
                .iter()
                .filter(|id| !all.contains(id))
                .filter(|id| {
                    !crate::layer_catalog::properties(d, Key::World(**id)).hidden
                        && d.scene.file_objects.get(Key::World(**id)).is_none()
                })
                .map(|id| box_for(&d.scene, &BTreeSet::from([*id])))
                .collect::<Result<Vec<_>, _>>()?;
            // Start from non-overlap, lower bodies first; only moving proxies can be lifted.
            let mut lifted = 0;
            for i in 0..bodies.len() {
                for _ in 0..257 {
                    let b = bodies[i].bounds;
                    let mut target = floor.max(b.lo[1]);
                    for other in static_boxes
                        .iter()
                        .copied()
                        .chain(bodies[..i].iter().map(|b| b.bounds))
                    {
                        if b.overlaps(other) {
                            target = target.max(other.hi[1]);
                        }
                    }
                    let delta = target - b.lo[1];
                    if delta <= 1e-8 {
                        break;
                    }
                    bodies[i].bounds.shift(delta);
                    lifted += 1;
                }
            }
            let step = timestep.clamp(1. / 240., 1. / 120.);
            let count = (seconds / step).ceil() as usize;
            if count
                .saturating_mul(bodies.len())
                .saturating_mul(static_boxes.len() + bodies.len())
                > 32_000_000
            {
                return Err(
                    "Settle contact-work budget exceeded; reduce duration / moving trees".into(),
                );
            }
            if timestep > 1. / 120. {
                warnings.push(
                    "Large timestep subdivided to at most1/120 second; no realtime motion".into(),
                );
            }
            if lifted > 0 {
                warnings.push(format!(
                    "Resolved {lifted} initial floor / collider overlaps upward using AABB proxies"
                ));
            }
            let dt = if count == 0 {
                0.
            } else {
                seconds / count as f64
            };
            for _ in 0..count {
                crate::book_io::check_cancel(cancel)?;
                for i in 0..bodies.len() {
                    let b = bodies[i].bounds;
                    let velocity = bodies[i].velocity - gravity * dt;
                    let wanted = b.lo[1] + velocity * dt;
                    let mut landing = floor;
                    for other in static_boxes.iter().copied().chain(
                        bodies
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| *j != i)
                            .map(|(_, b)| b.bounds),
                    ) {
                        if b.footprint(other) && other.hi[1] <= b.lo[1] + 1e-8 {
                            landing = landing.max(other.hi[1]);
                        }
                    }
                    let bottom = wanted.max(landing);
                    bodies[i].bounds.shift(bottom - b.lo[1]);
                    bodies[i].velocity = if wanted <= landing + 1e-8 {
                        0.
                    } else {
                        velocity
                    };
                }
            }
            let falling = bodies.iter().filter(|b| b.velocity.abs() > 1e-8).count();
            if falling > 0 {
                warnings.push(format!("{falling} bodies still falling at the duration limit; Apply retains only these previewed positions"));
            }
            for b in bodies {
                let delta = b.bounds.lo[1] - b.initial.lo[1];
                lib::shift(&mut state.scene, &b.ids, [0., delta, 0.])?;
                affected.extend(b.ids);
            }
            format!(
                "Deterministic vertical gravity settle; {count} bounded substeps; AABB colliders; velocities discarded"
            )
        }
    };
    crate::book_io::check_cancel(cancel)?;
    if !budget(&state, limit) {
        return Err("Placement result exceeds memory / geometry limits".into());
    }
    world3d::prepared(d, state, summary, affected, warnings)
}
