//! Embedded local vector tip presets; no file or network dependency in saved artwork.
use crate::{
    document::Document,
    scene::{Affine, Point},
    sizing::DocumentState,
    vector_edit::Selection,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Round,
    Flat,
    Square,
    Diamond,
}
impl Preset {
    pub const ALL: [Self; 4] = [Self::Round, Self::Flat, Self::Square, Self::Diamond];
    pub fn is_round(&self) -> bool {
        *self == Self::Round
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Round => "Round",
            Self::Flat => "Flat oval",
            Self::Square => "Square",
            Self::Diamond => "Diamond",
        }
    }
    pub fn named(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|p| p.label() == name)
            .ok_or_else(|| format!("Missing local vector tip preset: {name}"))
    }
    pub(crate) fn frame(self) -> Affine {
        if self == Self::Flat {
            Affine([1., 0., 0., 1. / 3., 0., 0.])
        } else {
            Affine::default()
        }
    }
    pub(crate) fn polygon(self) -> Option<[Point; 4]> {
        match self {
            Self::Square => Some([[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]]),
            Self::Diamond => Some([[0., -1.], [1., 0.], [0., 1.], [-1., 0.]]),
            _ => None,
        }
    }
    pub(crate) fn hit(self, p: Point, a: Point, b: Point, wa: f64, wb: f64) -> bool {
        if self == Self::Round {
            return crate::vector_pressure::taper_hit(p, a, b, wa, wb);
        }
        if self == Self::Flat {
            let map = |v: Point| [v[0], v[1] * 3.];
            return crate::vector_pressure::taper_hit(map(p), map(a), map(b), wa, wb);
        }
        let axes = if self == Self::Square {
            [[1., 0.], [-1., 0.], [0., 1.], [0., -1.]]
        } else {
            [[1., 1.], [-1., 1.], [1., -1.], [-1., -1.]]
        };
        let mut lo: f64 = 0.;
        let mut hi: f64 = 1.;
        let dr = (wb - wa) / 2.;
        for axis in axes {
            let c = axis[0] * (p[0] - a[0]) + axis[1] * (p[1] - a[1]) - wa / 2.;
            let slope = axis[0] * (b[0] - a[0]) + axis[1] * (b[1] - a[1]) + dr;
            if slope > 0. {
                lo = lo.max(c / slope);
            } else if slope < 0. {
                hi = hi.min(c / slope);
            } else if c > 0. {
                return false;
            }
        }
        lo <= hi
            && lo <= 1.
            && hi >= 0.
            && (wa / 2. + dr * lo.max(0.) > 0. || wa / 2. + dr * hi.min(1.) > 0.)
    }
}
pub fn prepare(
    d: &Document,
    layer: u64,
    s: &Selection,
    preset: &str,
) -> Result<DocumentState, String> {
    let tip = Preset::named(preset)?;
    crate::vector_refine::prepare(d, layer, s, crate::vector_refine::Operation::Tip(tip))
}
pub fn requires_v67(scene: &crate::scene::Scene) -> bool {
    scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .any(|p| !p.tip.is_round())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    Selected,
    All,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Points {
    #[default]
    Ends,
    All,
    None,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overlay {
    pub enabled: bool,
    pub scope: Scope,
    pub points: Points,
}
pub struct Outline {
    pub layer: u64,
    pub path: u64,
    pub closed: bool,
    pub points: Vec<Point>,
    pub controls: Vec<Point>,
}
pub fn outlines(d: &Document, selected: Option<u64>, settings: Overlay) -> Vec<Outline> {
    if !settings.enabled {
        return vec![];
    }
    let mut out = vec![];
    for layer in &d.scene.lines.layers {
        let target = crate::layer_properties::Target::Vector(layer.id);
        if settings.scope == Scope::Selected && selected != Some(layer.id)
            || crate::layer_properties::inherited(d, target).hidden
        {
            continue;
        }
        for path in &layer.paths {
            let controls = match settings.points {
                Points::None => vec![],
                Points::Ends => {
                    if path.closed {
                        vec![]
                    } else {
                        vec![path.points[0], *path.points.last().unwrap()]
                    }
                }
                Points::All => (0..path.control_count())
                    .filter_map(|n| path.control_point(n))
                    .collect(),
            };
            out.push(Outline {
                layer: layer.id,
                path: path.id,
                closed: path.closed,
                points: path
                    .points
                    .iter()
                    .map(|p| d.scene.transform.map(*p))
                    .collect(),
                controls: controls
                    .into_iter()
                    .map(|p| d.scene.transform.map(p))
                    .collect(),
            });
        }
    }
    out
}
