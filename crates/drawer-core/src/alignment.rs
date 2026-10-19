//! Atomic page-space alignment with explicit reference bounds and stable object identities.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, PageObject, Point, Rect},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    Raster(usize),
    Object(usize),
    Vector(u64),
    Gradient(u64),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reference {
    #[default]
    Selection,
    Canvas,
    Key(Item),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}
impl Align {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Center,
        Self::Right,
        Self::Top,
        Self::Middle,
        Self::Bottom,
    ];
    fn axis(self) -> usize {
        if matches!(self, Self::Left | Self::Center | Self::Right) {
            0
        } else {
            1
        }
    }
    fn position(self) -> f64 {
        match self {
            Self::Left | Self::Top => 0.0,
            Self::Center | Self::Middle => 0.5,
            Self::Right | Self::Bottom => 1.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Align(Align),
    GapsHorizontal,
    GapsVertical,
    CentersHorizontal,
    CentersVertical,
}
pub fn items(doc: &Document) -> Vec<Item> {
    (0..doc.layers.len())
        .map(Item::Raster)
        .chain((0..doc.scene.objects.len()).map(Item::Object))
        .chain(
            doc.scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .map(|p| Item::Vector(p.id)),
        )
        .chain(
            doc.scene
                .gradients
                .layers
                .iter()
                .map(|l| Item::Gradient(l.id)),
        )
        .collect()
}
fn rectangle(points: impl IntoIterator<Item = Point>) -> Result<Rect, String> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in points {
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let r = Rect { min, max };
    if !r.valid() {
        return Err("Object/reference has no finite bounds".into());
    }
    Ok(r)
}
fn mapped(r: Rect, a: Affine) -> Result<Rect, String> {
    rectangle([r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]].map(|p| a.map(p)))
}
pub fn bounds(doc: &Document, item: Item) -> Result<Rect, String> {
    match item {
        Item::Raster(i) => {
            let p = doc
                .layers
                .get(i)
                .ok_or("Missing raster object")?
                .extent(doc.width, doc.height);
            Ok(Rect {
                min: p.offset.map(|v| v as f64),
                max: [
                    (p.offset[0] + p.width as i64) as f64,
                    (p.offset[1] + p.height as i64) as f64,
                ],
            })
        }
        Item::Object(i) => mapped(
            doc.scene
                .objects
                .get(i)
                .ok_or("Missing page object")?
                .bounds(),
            doc.scene.transform,
        ),
        Item::Vector(id) => {
            let p = doc
                .scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .find(|p| p.id == id)
                .ok_or("Missing vector object")?;
            let mut r = p.bounds();
            if !p.stroke {
                for k in 0..2 {
                    r.min[k] += p.width / 2.0;
                    r.max[k] -= p.width / 2.0;
                }
            }
            mapped(r, doc.scene.transform)
        }
        Item::Gradient(id) => {
            let l = doc
                .scene
                .gradients
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Missing gradient object")?;
            let mapping = l.creation.inverse().then(doc.scene.transform);
            rectangle(
                l.areas
                    .iter()
                    .flat_map(|a| [&a.paint, &a.black, &a.transparent])
                    .flat_map(|m| &m.runs)
                    .flat_map(|r| {
                        let x = r.x as f64;
                        let y = r.y as f64;
                        let end = x + r.length as f64;
                        [[x, y], [end, y], [end, y + 1.0], [x, y + 1.0]].map(|p| mapping.map(p))
                    }),
            )
        }
    }
}
fn selection(doc: &Document, objects: &[(Item, Rect)]) -> Result<Rect, String> {
    if let Some(mask) = &doc.scene.selection_mask {
        return rectangle(mask.runs.iter().flat_map(|r| {
            let x = r.x as f64;
            let y = r.y as f64;
            let end = x + r.length as f64;
            [[x, y], [end, y], [end, y + 1.0], [x, y + 1.0]].map(|p| doc.scene.selection_point(p))
        }));
    }
    if let Some(r) = doc.scene.selection {
        return rectangle(
            [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
                .map(|p| doc.scene.selection_point(p)),
        );
    }
    rectangle(objects.iter().flat_map(|(_, r)| [r.min, r.max]))
}
fn writable(doc: &Document, item: Item) -> Result<(), String> {
    let okay = match item {
        Item::Raster(n) => doc
            .layers
            .get(n)
            .is_some_and(|l| !l.locked && doc.scene.paper.raster_visible(n, doc.layers.len())),
        Item::Vector(id) => doc
            .scene
            .lines
            .layers
            .iter()
            .any(|l| !l.locked && !l.hidden && l.paths.iter().any(|p| p.id == id)),
        Item::Gradient(id) => doc
            .scene
            .gradients
            .layers
            .iter()
            .any(|l| l.id == id && !l.hidden),
        Item::Object(n) => n < doc.scene.objects.len(),
    };
    if okay {
        Ok(())
    } else {
        Err(format!(
            "{item:?} is missing, hidden or locked; no objects changed"
        ))
    }
}
pub fn prepare(
    doc: &Document,
    selected: &[Item],
    reference: Reference,
    operation: Operation,
) -> Result<DocumentState, String> {
    if !doc.scene.valid()
        || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || doc
            .memory_bytes()
            .saturating_mul(2)
            .saturating_add(doc.resident_bytes())
            > DEFAULT_DOCUMENT_BUDGET
        || selected.len() > 8192
    {
        return Err("Invalid alignment page or undo/work limit".into());
    }
    let unique: std::collections::BTreeSet<_> = selected.iter().copied().collect();
    if unique.len() != selected.len() {
        return Err("Duplicate alignment object".into());
    }
    if selected.is_empty() {
        return Ok(doc.snapshot());
    }
    let mut objects: Vec<_> = selected
        .iter()
        .map(|i| bounds(doc, *i).map(|r| (*i, r)))
        .collect::<Result<_, _>>()?;
    if let Reference::Key(key) = reference {
        objects.retain(|(i, _)| *i != key);
    }
    if objects.is_empty() || (!matches!(operation, Operation::Align(_)) && objects.len() < 3) {
        return Ok(doc.snapshot());
    }
    let base = match reference {
        Reference::Selection => selection(doc, &objects)?,
        Reference::Canvas => Rect {
            min: [0.0; 2],
            max: [doc.width as f64, doc.height as f64],
        },
        Reference::Key(i) => bounds(doc, i)?,
    };
    for (item, _) in &objects {
        writable(doc, *item)?;
    }
    let mut movements = vec![];
    match operation {
        Operation::Align(a) => {
            let axis = a.axis();
            let t = a.position();
            let anchor = base.min[axis] * (1.0 - t) + base.max[axis] * t;
            for (item, r) in objects {
                let mut delta = [0.0; 2];
                delta[axis] = anchor - (r.min[axis] * (1.0 - t) + r.max[axis] * t);
                movements.push((item, delta));
            }
        }
        _ => {
            let axis = usize::from(matches!(
                operation,
                Operation::GapsVertical | Operation::CentersVertical
            ));
            objects.sort_by(|a, b| {
                ((a.1.min[axis] + a.1.max[axis]) / 2.0)
                    .total_cmp(&((b.1.min[axis] + b.1.max[axis]) / 2.0))
            });
            let centers = matches!(
                operation,
                Operation::CentersHorizontal | Operation::CentersVertical
            );
            let span = base.max[axis] - base.min[axis];
            let sizes: Vec<_> = objects
                .iter()
                .map(|(_, r)| r.max[axis] - r.min[axis])
                .collect();
            let spacing = if centers {
                (span - (sizes[0] + sizes[sizes.len() - 1]) / 2.0) / (objects.len() - 1) as f64
            } else {
                (span - sizes.iter().sum::<f64>()) / (objects.len() - 1) as f64
            };
            if spacing < -1e-9
                || centers
                    && sizes
                        .windows(2)
                        .any(|s| spacing < (s[0] + s[1]) / 2.0 - 1e-9)
            {
                return Err(
                    "Insufficient space for non-overlapping equal distribution; no objects changed"
                        .into(),
                );
            }
            let mut next = base.min[axis];
            for (n, (item, r)) in objects.into_iter().enumerate() {
                let location = if centers {
                    base.min[axis] + sizes[0] / 2.0 + n as f64 * spacing - sizes[n] / 2.0
                } else {
                    next
                };
                let mut delta = [0.0; 2];
                delta[axis] = location - r.min[axis];
                movements.push((item, delta));
                next += sizes[n] + spacing.max(0.0);
            }
        }
    }
    let mut after = doc.snapshot();
    for (item, d) in movements {
        shift(doc, &mut after, item, d)?;
    }
    if !after.scene.valid()
        || after.layers.iter().any(|l| !l.valid(doc.width, doc.height))
        || serde_json::to_vec(&after.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Alignment exceeds object/document limits; no objects changed".into());
    }
    Ok(after)
}
fn shift(doc: &Document, state: &mut DocumentState, item: Item, d: Point) -> Result<(), String> {
    if d.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return Err("Alignment movement limit".into());
    }
    if d == [0.0; 2] {
        return Ok(());
    }
    let inverse = doc.scene.transform.inverse();
    let delta = [
        inverse.0[0] * d[0] + inverse.0[2] * d[1],
        inverse.0[1] * d[0] + inverse.0[3] * d[1],
    ];
    let move_point = |p: &mut Point| {
        p[0] += delta[0];
        p[1] += delta[1];
    };
    let creation = Affine([1.0, 0.0, 0.0, 1.0, -delta[0], -delta[1]]);
    match item {
        Item::Raster(n) => {
            let shift = d.map(|v| v.round() as i64);
            if shift == [0; 2] {
                return Ok(());
            }
            let mut p = state.layers[n].extent(doc.width, doc.height);
            for (k, s) in shift.into_iter().enumerate() {
                p.offset[k] = p.offset[k]
                    .checked_add(s)
                    .ok_or("Raster alignment coordinate limit")?;
            }
            if !p.valid() {
                return Err("Raster alignment placement limit".into());
            }
            state.layers[n].placement = Some(p);
        }
        Item::Object(n) => match &mut state.scene.objects[n] {
            PageObject::Text { baseline, .. } => move_point(baseline),
            PageObject::Line { start, end, .. } => {
                move_point(start);
                move_point(end);
            }
            PageObject::Tone { bounds, .. } | PageObject::Shadow { bounds, .. } => {
                move_point(&mut bounds.min);
                move_point(&mut bounds.max);
            }
        },
        Item::Vector(id) => {
            let p = state
                .scene
                .lines
                .layers
                .iter_mut()
                .flat_map(|l| &mut l.paths)
                .find(|p| p.id == id)
                .ok_or("Missing vector alignment target")?;
            for point in &mut p.points {
                move_point(point);
            }
            if let Some(def) = &mut p.definition {
                def.creation = creation.then(def.creation);
            }
        }
        Item::Gradient(id) => {
            let l = state
                .scene
                .gradients
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing gradient alignment target")?;
            l.creation = creation.then(l.creation);
        }
    }
    Ok(())
}
