//! Local selection shortcuts and geometry-only contour conversion.
use crate::{
    document::Document,
    scene::{Point, Scene},
    selection_edit::Combine,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Clear,
    DeleteOutside,
    Invert,
    Reselect,
    Outline,
    Fill,
    Copy,
    MoveBoundary,
    MovePixels,
    Alpha,
    Contour,
    VectorEdit,
    Undo,
    Redo,
}
impl Command {
    pub const ALL: [Self; 14] = [
        Self::Clear,
        Self::DeleteOutside,
        Self::Invert,
        Self::Reselect,
        Self::Outline,
        Self::Fill,
        Self::Copy,
        Self::MoveBoundary,
        Self::MovePixels,
        Self::Alpha,
        Self::Contour,
        Self::VectorEdit,
        Self::Undo,
        Self::Redo,
    ];
    pub fn action(self) -> &'static str {
        match self {
            Self::Clear => "selection-clear",
            Self::DeleteOutside => "delete-outside-selection",
            Self::Invert => "selection-invert",
            Self::Reselect => "selection-reselect",
            Self::Outline => "selection-outline",
            Self::Fill => "fill-selection",
            Self::Copy => "copy-selection",
            Self::MoveBoundary => "move-selection-boundary",
            Self::MovePixels => "move-selected-pixels",
            Self::Alpha => "alpha-selection-replace",
            Self::Contour => "contour-selection",
            Self::VectorEdit => "layer-selection-tools",
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Deselect",
            Self::DeleteOutside => "Delete outside",
            Self::Invert => "Invert",
            Self::Reselect => "Reselect",
            Self::Outline => "Outline",
            Self::Fill => "Fill",
            Self::Copy => "Copy",
            Self::MoveBoundary => "Move boundary",
            Self::MovePixels => "Move pixels",
            Self::Alpha => "Layer alpha",
            Self::Contour => "From contour…",
            Self::VectorEdit => "Vector editing…",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub visible: bool,
    pub commands: Vec<Command>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: true,
            commands: vec![
                Command::Clear,
                Command::Invert,
                Command::Outline,
                Command::Contour,
                Command::Undo,
                Command::Redo,
            ],
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.commands.len() <= Command::ALL.len()
            && self
                .commands
                .iter()
                .enumerate()
                .all(|(i, c)| !self.commands[..i].contains(c))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Vector(u64),
    Rulers(Vec<u64>),
}
pub struct Preview {
    pub scene: Scene,
    pub points: Vec<Point>,
    pub closure: Option<[Point; 2]>,
}
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= 1e-6
}
fn chain(doc: &Document, ids: &[u64]) -> Result<(Vec<Point>, bool), String> {
    if !(2..=32).contains(&ids.len()) || ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id))
    {
        return Err("Choose 2–32 distinct finite straight ruler segments".into());
    }
    let mut vertices: Vec<Point> = vec![];
    let mut edges = vec![];
    for id in ids {
        let g = doc
            .scene
            .rulers
            .guides
            .iter()
            .find(|g| g.id == *id)
            .ok_or("Ruler is missing")?;
        let crate::rulers::Kind::Straight { a, b } = g.kind else {
            return Err("Contour uses straight ruler endpoints; symmetry/perspective is not an enclosed contour".into());
        };
        let mut pair = [0; 2];
        for (n, p) in [a, b].into_iter().enumerate() {
            pair[n] = if let Some(i) = vertices.iter().position(|v| near(*v, p)) {
                i
            } else {
                vertices.push(p);
                vertices.len() - 1
            };
        }
        if pair[0] == pair[1]
            || edges
                .iter()
                .any(|e: &[usize; 2]| *e == pair || *e == [pair[1], pair[0]])
        {
            return Err("Degenerate/duplicate contour edge".into());
        }
        edges.push(pair);
    }
    let mut adjacency = vec![vec![]; vertices.len()];
    for (i, [a, b]) in edges.iter().copied().enumerate() {
        adjacency[a].push((b, i));
        adjacency[b].push((a, i));
    }
    if vertices.len() < 3 || adjacency.iter().any(|v| v.len() > 2) {
        return Err("Contour must be an unbranched chain with area".into());
    }
    let ends = adjacency
        .iter()
        .enumerate()
        .filter_map(|(i, a)| (a.len() == 1).then_some(i))
        .collect::<Vec<_>>();
    let closed = ends.is_empty();
    if !closed && ends.len() != 2 {
        return Err("Disconnected contour".into());
    }
    let start = ends.first().copied().unwrap_or(0);
    let mut at = start;
    let mut used = vec![false; edges.len()];
    let mut points = vec![vertices[at]];
    while let Some((next, edge)) = adjacency[at].iter().copied().find(|(_, i)| !used[*i]) {
        used[edge] = true;
        at = next;
        if at != start {
            points.push(vertices[at]);
        }
    }
    if used.contains(&false) || (closed && at != start) {
        return Err("Select one connected contour".into());
    }
    Ok((points, closed))
}
pub fn preview(doc: &Document, source: &Source, combine: Combine) -> Result<Preview, String> {
    let (points, closed) = match source {
        Source::Vector(id) => {
            let path = doc
                .scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .find(|p| p.id == *id)
                .ok_or("Vector path is missing")?;
            (
                path.points.clone(),
                path.closed || near(path.points[0], *path.points.last().unwrap()),
            )
        }
        Source::Rulers(ids) => chain(doc, ids)?,
    };
    let points = points
        .into_iter()
        .map(|p| doc.scene.transform.map(p))
        .collect::<Vec<_>>();
    let scene = crate::selection_geometry::prepare(
        doc,
        crate::selection_geometry::Kind::Polygon,
        &points,
        combine,
    )?;
    let closure = (!closed).then(|| [*points.last().unwrap(), points[0]]);
    Ok(Preview {
        scene,
        points,
        closure,
    })
}
pub fn prepare(
    doc: &Document,
    source: &Source,
    combine: Combine,
    confirm_close: bool,
) -> Result<Scene, String> {
    let p = preview(doc, source, combine)?;
    if p.closure.is_some() && !confirm_close {
        return Err("Preview the closing segment and confirm auto-close first".into());
    }
    Ok(p.scene)
}
