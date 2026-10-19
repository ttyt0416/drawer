//! Owned page rulers; view transforms never participate in stroke constraints.
use crate::{
    document::Document,
    scene::{Affine, Point},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Straight { a: Point, b: Point },
    Mirror { a: Point, b: Point },
    Radial { center: Point, arms: u8 },
    Perspective { points: Vec<Point> },
    Special(crate::special_rulers::Spec),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guide {
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub display: crate::ruler_scope::Display,
    #[serde(default)]
    pub scope: crate::ruler_scope::Scope,
    #[serde(default)]
    pub frames: Option<[u32; 2]>,
    #[serde(default)]
    pub link: Option<crate::line_tools::Target>,
    #[serde(default)]
    pub offset: Point,
    pub id: u64,
    pub name: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next_id: u64,
    pub active: Option<u64>,
    pub guides: Vec<Guide>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next_id: 1,
            active: None,
            guides: vec![],
        }
    }
}
fn enabled() -> bool {
    true
}
fn point(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
}
impl Kind {
    pub fn valid(&self) -> bool {
        match self {
            Self::Special(s) => s.valid(),
            Self::Straight { a, b } | Self::Mirror { a, b } => {
                point(*a) && point(*b) && (a[0] - b[0]).hypot(a[1] - b[1]) >= 1e-6
            }
            Self::Radial { center, arms } => point(*center) && (2..=16).contains(arms),
            Self::Perspective { points } => {
                !points.is_empty() && points.len() <= 3 && points.iter().all(|p| point(*p))
            }
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Special(s) => s.label(),
            Self::Straight { .. } => "Straight",
            Self::Mirror { .. } => "Axial symmetry",
            Self::Radial { .. } => "Radial symmetry",
            Self::Perspective { points } => match points.len() {
                1 => "1-point perspective",
                2 => "2-point perspective",
                _ => "3-point perspective",
            },
        }
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.next_id > 0
            && self.guides.len() <= 32
            && self
                .active
                .is_none_or(|id| self.guides.iter().any(|g| g.id == id))
            && self.guides.iter().all(|g| {
                g.id > 0
                    && g.id < self.next_id
                    && g.name == g.name.trim()
                    && !g.name.is_empty()
                    && g.name.chars().count() <= 64
                    && !g.name.chars().any(char::is_control)
                    && g.kind.valid()
                    && g.scope.valid()
                    && g.link.is_none_or(crate::ruler_scope::target_valid)
                    && g.frames
                        .is_none_or(|r| r[0] <= r[1] && r[1] < crate::animation::MAX_FRAMES)
                    && g.offset.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
            })
            && self
                .guides
                .iter()
                .enumerate()
                .all(|(i, g)| self.guides[..i].iter().all(|x| x.id != g.id))
    }
    pub fn bytes(&self) -> usize {
        self.guides
            .iter()
            .map(|g| {
                256 + g.name.len()
                    + match &g.kind {
                        Kind::Special(s) => s.bytes(),
                        _ => 0,
                    }
            })
            .sum()
    }
    pub fn add(&mut self, name: &str, kind: Kind) -> Result<u64, String> {
        let mut next = self.clone();
        let id = next.next_id;
        next.next_id = next.next_id.checked_add(1).ok_or("Ruler identity limit")?;
        next.guides.push(Guide {
            enabled: true,
            display: Default::default(),
            scope: Default::default(),
            frames: None,
            link: None,
            offset: [0.0; 2],
            id,
            name: name.trim().into(),
            kind,
        });
        next.active = Some(id);
        if !next.valid() {
            return Err("Invalid ruler/name or ruler limit (32)".into());
        }
        *self = next;
        Ok(id)
    }
}
pub fn apply(doc: &mut Document, data: Data) -> Result<bool, String> {
    if !data.valid() {
        return Err("Invalid ruler data".into());
    }
    let mut scene = doc.scene.clone();
    scene.rulers = data;
    if !scene.valid()
        || serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        return Err("Rulers exceed scene budget".into());
    }
    doc.edit_scene(scene)
}
#[derive(Clone)]
pub struct Runtime {
    lower: Option<crate::snapping::Lower>,
    temporary_disabled: bool,
    constraint: Constraint,
    pub copies: Vec<Affine>,
}
#[derive(Clone)]
enum Constraint {
    None,
    Special(Box<crate::special_rulers::Runtime>),
    Line(Point, Point),
    Perspective {
        points: Vec<Point>,
        anchor: Option<Point>,
        line: Option<Point>,
    },
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            lower: None,
            temporary_disabled: false,
            constraint: Constraint::None,
            copies: vec![],
        }
    }
}
fn direction(a: Point, b: Point) -> Option<Point> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = d[0].hypot(d[1]);
    (length >= 1e-8 && length.is_finite()).then(|| d.map(|v| v / length))
}
fn rotation(center: Point, angle: f64) -> Affine {
    let (s, c) = crate::texture::rotation(angle.to_degrees());
    Affine([
        c,
        s,
        -s,
        c,
        center[0] - c * center[0] + s * center[1],
        center[1] - s * center[0] - c * center[1],
    ])
}
pub fn angle(t: Affine, value: f64) -> f64 {
    let (s, c) = value.to_radians().sin_cos();
    let p = t.map([c, s]);
    let o = t.map([0.0; 2]);
    (p[1] - o[1]).atan2(p[0] - o[0]).to_degrees()
}
pub fn reflected(t: Affine) -> bool {
    t.0[0] * t.0[3] - t.0[1] * t.0[2] < 0.0
}
impl Runtime {
    pub fn new(doc: &Document) -> Result<Self, String> {
        Self::for_target(doc, crate::line_tools::Target::Raster(0))
    }
    pub fn for_target(doc: &Document, target: crate::line_tools::Target) -> Result<Self, String> {
        let data = &doc.scene.rulers;
        if !data.valid() {
            return Err("Invalid document rulers".into());
        }
        let mut r = Self::default();
        if let Some(g) = data
            .active
            .and_then(|id| data.guides.iter().find(|g| g.id == id))
        {
            if !crate::ruler_scope::applies(doc, g, target) {
                return Ok(r);
            }
            let t = crate::ruler_scope::transform(doc, g)
                .ok_or("Linked ruler exceeds page transform limits")?;
            match &g.kind {
                Kind::Special(s) => {
                    r.constraint =
                        Constraint::Special(Box::new(crate::special_rulers::Runtime::new(s, t)?));
                }
                Kind::Straight { a, b } => {
                    let a = t.map(*a);
                    let b = t.map(*b);
                    let d = direction(a, b).ok_or("Degenerate transformed ruler")?;
                    r.constraint = Constraint::Line(a, d);
                }
                Kind::Mirror { a, b } => {
                    let a = t.map(*a);
                    let b = t.map(*b);
                    let [x, y] = direction(a, b).ok_or("Degenerate transformed symmetry axis")?;
                    let m = [
                        2.0 * x * x - 1.0,
                        2.0 * x * y,
                        2.0 * x * y,
                        2.0 * y * y - 1.0,
                    ];
                    r.copies.push(Affine([
                        m[0],
                        m[1],
                        m[2],
                        m[3],
                        a[0] - m[0] * a[0] - m[2] * a[1],
                        a[1] - m[1] * a[0] - m[3] * a[1],
                    ]));
                }
                Kind::Radial { center, arms } => {
                    for n in 1..*arms {
                        r.copies.push(rotation(
                            t.map(*center),
                            std::f64::consts::TAU * n as f64 / *arms as f64,
                        ));
                    }
                }
                Kind::Perspective { points } => {
                    r.constraint = Constraint::Perspective {
                        points: points.iter().map(|p| t.map(*p)).collect(),
                        anchor: None,
                        line: None,
                    }
                }
            }
        }
        Ok(r)
    }
    pub fn configure_snapping(
        &mut self,
        doc: &Document,
        settings: crate::snapping::Settings,
        scale: f64,
    ) -> Result<usize, String> {
        if self.lower.is_some() {
            return Err("Configure snapping once before input".into());
        }
        let lower = crate::snapping::Lower::new(doc, settings, scale)?;
        let bytes = lower.bytes();
        if let Constraint::Special(r) = &mut self.constraint {
            r.set_distance(settings.screen_distance * scale);
        }
        if !settings.rulers {
            self.constraint = Constraint::None;
            self.copies.clear();
        }
        self.lower = Some(lower);
        Ok(bytes)
    }
    pub fn memory_bytes(&self) -> usize {
        self.copies.capacity() * std::mem::size_of::<Affine>()
            + match &self.constraint {
                Constraint::Special(s) => std::mem::size_of_val(s.as_ref()) + s.bytes(),
                _ => 0,
            }
            + self.lower.as_ref().map_or(0, |l| l.bytes())
    }
    pub fn check_snapping(&self) -> Result<(), String> {
        if matches!(&self.constraint,Constraint::Special(s) if s.failed())
            || self.lower.as_ref().is_some_and(|l| l.failed())
        {
            Err("Snapping exceeds ruler/guide/grid work limit; pending input cancelled".into())
        } else {
            Ok(())
        }
    }
    pub fn constrain_dab(&mut self, p: Point, anchor: Point) -> (Point, Point) {
        let a = self.constrain(anchor);
        let p = if !matches!(self.constraint, Constraint::None) {
            self.constrain(p)
        } else if a == anchor {
            p
        } else {
            [p[0] + (a[0] - anchor[0]), p[1] + (a[1] - anchor[1])]
        };
        (p, a)
    }
    pub fn set_snap_disabled(&mut self, disabled: bool) {
        self.temporary_disabled = disabled;
    }
    pub fn constrain(&mut self, p: Point) -> Point {
        self.constrain_with_hit(p).0
    }
    pub fn constrain_with_hit(&mut self, p: Point) -> (Point, bool) {
        if self.temporary_disabled {
            return (p, false);
        }
        if matches!(self.constraint, Constraint::None) {
            return self.lower.as_mut().map_or((p, false), |l| l.snap(p));
        }
        let result = match &mut self.constraint {
            Constraint::None => p,
            Constraint::Special(s) => s.project(p),
            Constraint::Line(a, d) => project(*a, *d, p),
            Constraint::Perspective {
                points,
                anchor,
                line,
            } => {
                let Some(a) = *anchor else {
                    *anchor = Some(p);
                    return (p, true);
                };
                if line.is_none()
                    && let Some(motion) = direction(a, p)
                {
                    let mut best = -1.0;
                    for v in points {
                        if let Some(d) = direction(a, *v) {
                            let score = (d[0] * motion[0] + d[1] * motion[1]).abs();
                            if score > best {
                                best = score;
                                *line = Some(d);
                            }
                        }
                    }
                }
                line.map_or(p, |d| project(a, d, p))
            }
        };
        (result, true)
    }
}
fn project(a: Point, d: Point, p: Point) -> Point {
    let k = (p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1];
    [a[0] + d[0] * k, a[1] + d[1] * k]
}
/// Page-space construction lines; the canvas uses its normal projection/clipping.
pub fn lines(guide: &Guide, transform: Affine, size: Point) -> Vec<[Point; 2]> {
    let span = size[0].hypot(size[1]).max(1.0) * 4.0;
    match &guide.kind {
        Kind::Special(s) => s
            .lines(span)
            .unwrap_or_default()
            .into_iter()
            .map(|line| line.map(|p| transform.map(p)))
            .collect(),
        Kind::Straight { a, b } | Kind::Mirror { a, b } => {
            let a = transform.map(*a);
            let b = transform.map(*b);
            direction(a, b).map_or(vec![], |d| {
                vec![[
                    [a[0] - d[0] * span, a[1] - d[1] * span],
                    [a[0] + d[0] * span, a[1] + d[1] * span],
                ]]
            })
        }
        Kind::Radial { center, arms } => {
            let c = transform.map(*center);
            (0..*arms)
                .map(|n| {
                    let (s, c0) = (std::f64::consts::TAU * n as f64 / *arms as f64).sin_cos();
                    [c, [c[0] + c0 * span, c[1] + s * span]]
                })
                .collect()
        }
        Kind::Perspective { points } => {
            let center = size.map(|v| v / 2.0);
            points
                .iter()
                .flat_map(|p| {
                    let p = transform.map(*p);
                    let base = (center[1] - p[1]).atan2(center[0] - p[0]);
                    (-3..=3).map(move |n| {
                        let (s, c) = (base + n as f64 * std::f64::consts::PI / 12.0).sin_cos();
                        [p, [p[0] + c * span, p[1] + s * span]]
                    })
                })
                .collect()
        }
    }
}

pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        assets::BrushTip,
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        painting::Stroke,
    };
    let make = || {
        Document::create(
            CanvasSpec {
                width: 32,
                height: 32,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())
    };
    let mut style = crate::brush_presets::Preset::capture(
        &crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        },
        BrushTip::PixelPen,
        "Ruler",
    )?
    .style([200, 40, 20, 128]);
    style.shape.size = 1.0;
    style.smoothing = 0.0;
    style.pressure.size = false;
    style.pressure.opacity = false;
    style.profile.minimum_size = 1.0;
    style.profile.minimum_opacity = 1.0;
    for (kind, source, expected) in [
        (
            Kind::Mirror {
                a: [16.0, 0.0],
                b: [16.0, 32.0],
            },
            [5.5, 8.5],
            vec![[5, 8], [26, 8]],
        ),
        (
            Kind::Mirror {
                a: [16.5, 0.0],
                b: [16.5, 32.0],
            },
            [16.5, 8.5],
            vec![[16, 8]],
        ),
        (
            Kind::Radial {
                center: [16.0; 2],
                arms: 4,
            },
            [20.5, 16.5],
            vec![[20, 16], [15, 20], [11, 15], [16, 11]],
        ),
    ] {
        let mut d = make()?;
        d.scene.rulers.add("Symmetry", kind)?;
        let before = d.snapshot();
        let mut stroke = Stroke::begin(&d, 0, style)?;
        stroke.sample(&d, source)?;
        let mut preview = vec![];
        for y in 0..32 {
            for x in 0..32 {
                preview.push(stroke.pixel(&d, 0, &d.layers[0], x, y).unwrap_or([0; 4]));
            }
        }
        stroke.commit(&mut d)?;
        if d.layers[0].pixels != preview || d.history_counts() != (1, 0) {
            return Err("Symmetry preview/commit/history mismatch".into());
        }
        for p in expected {
            if d.layers[0].pixel(32, 32, p[0], p[1]) != [200, 40, 20, 128] {
                return Err(format!(
                    "Symmetry location/axis alpha {p:?}: {:?}",
                    d.layers[0].pixel(32, 32, p[0], p[1])
                ));
            }
        }
        d.undo();
        if d.snapshot() != before {
            return Err("Symmetry Undo".into());
        }
        d.redo();
        let path = root.join("ruler-symmetry.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != d.snapshot()
        {
            return Err("Rulers native reopen".into());
        }
    }
    for direction in crate::brush_dynamics::Direction::ALL {
        for mode in [
            crate::brush_pattern::Mode::Dabs,
            crate::brush_pattern::Mode::Scatter,
            crate::brush_pattern::Mode::Decoration,
        ] {
            let mut d = make()?;
            let mut astyle = crate::brush_presets::Preset::capture(
                &crate::workspaces::Snapshot {
                    properties: Some(Default::default()),
                    ..Default::default()
                },
                BrushTip::Pen,
                "Asymmetric",
            )?
            .style([100, 150, 200, 128]);
            astyle.shape.size = 6.0;
            astyle.shape.aspect = 0.4;
            astyle.shape.angle = 27.0;
            astyle.smoothing = 0.0;
            astyle.hardness = 1.0;
            astyle.pressure.size = false;
            astyle.pressure.opacity = false;
            astyle.dynamics.direction = direction;
            astyle.dynamics.fallback_angle = 13.0;
            astyle.dynamics.scatter_angle = 17.0;
            astyle.pattern.mode = mode;
            astyle.pattern.width = 6.0;
            astyle.pattern.count = 3;
            astyle.pattern.decoration = crate::brush_pattern::Decoration::Leaf;
            let mut one = Stroke::begin(&d, 0, astyle)?;
            one.set_dynamic_input(crate::brush_dynamics::Input {
                bearing: Some(31.0),
                ..Default::default()
            })?;
            for p in [[7.5, 9.5], [10.5, 13.5]] {
                one.sample(&d, p)?;
            }
            one.commit(&mut d)?;
            let pixels = d.layers[0].pixels.clone();
            d.undo();
            d.scene.rulers.add(
                "Reflection",
                Kind::Mirror {
                    a: [16.0, 0.0],
                    b: [16.0, 32.0],
                },
            )?;
            let mut both = Stroke::begin(&d, 0, astyle)?;
            both.set_dynamic_input(crate::brush_dynamics::Input {
                bearing: Some(31.0),
                ..Default::default()
            })?;
            for p in [[7.5, 9.5], [10.5, 13.5]] {
                both.sample(&d, p)?;
            }
            both.commit(&mut d)?;
            for y in 0..32 {
                for x in 0..32 {
                    let expected = if pixels[y * 32 + x] != [0; 4] {
                        pixels[y * 32 + x]
                    } else {
                        pixels[y * 32 + 31 - x]
                    };
                    if d.layers[0].pixels[y * 32 + x] != expected {
                        return Err(format!(
                            "Reflected asymmetric/dynamic tip mismatch {direction:?}/{mode:?} at {x},{y}: actual {:?}, expected {:?}, source {:?}, reflected {:?}",
                            d.layers[0].pixels[y * 32 + x],
                            expected,
                            pixels[y * 32 + x],
                            pixels[y * 32 + 31 - x]
                        ));
                    }
                }
            }
        }
    }
    let mut d = make()?;
    d.scene.rulers.add(
        "Horizontal",
        Kind::Straight {
            a: [0.0, 10.5],
            b: [32.0, 10.5],
        },
    )?;
    let mut stroke = Stroke::begin(&d, 0, style)?;
    stroke.sample(&d, [2.5, 2.5])?;
    stroke.sample(&d, [25.5, 20.5])?;
    stroke.commit(&mut d)?;
    if d.layers[0].pixel(32, 32, 2, 10)[3] != 128
        || d.layers[0]
            .pixels
            .iter()
            .enumerate()
            .any(|(n, p)| p[3] > 0 && n / 32 != 10)
    {
        return Err("Straight ruler did not constrain document coordinates".into());
    }
    d.undo();
    let scene = d.scene.clone();
    let mut stroke = Stroke::begin(&d, 0, style)?;
    stroke.disable_rulers()?;
    stroke.sample(&d, [2.5, 2.5])?;
    stroke.commit(&mut d)?;
    if d.layers[0].pixel(32, 32, 2, 2)[3] != 128 || d.scene != scene {
        return Err("Temporary snap disable altered ruler document".into());
    }
    for points in [
        vec![[0.0; 2]],
        vec![[0.0; 2], [32.0, 0.0]],
        vec![[0.0; 2], [32.0, 0.0], [16.0, 64.0]],
        vec![[0.0; 2]; 3],
    ] {
        let mut d = make()?;
        d.scene
            .rulers
            .add("Perspective", Kind::Perspective { points })?;
        let mut r = Runtime::new(&d)?;
        r.constrain([16.0; 2]);
        let p = r.constrain([9.0, 10.0]);
        let q = r.constrain([31.0, 16.0]);
        if (p[0] - p[1]).abs() > 1e-8 || (q[0] - q[1]).abs() > 1e-8 {
            return Err("Perspective direction did not lock".into());
        }
        let mut r = Runtime::new(&d)?;
        r.constrain([0.0; 2]);
        if r.constrain([1.0; 2]).iter().any(|v| !v.is_finite()) {
            return Err("Coincident vanishing point produced nonfinite output".into());
        }
    }
    let mut d = make()?;
    let before = d.snapshot();
    let mut data = Data::default();
    data.add(
        "Axis",
        Kind::Mirror {
            a: [16.0, 0.0],
            b: [16.0, 32.0],
        },
    )?;
    apply(&mut d, data.clone())?;
    d.undo();
    if d.snapshot() != before {
        return Err("Ruler object Undo".into());
    }
    d.redo();
    let mut invalid = data.clone();
    if invalid
        .add(
            "Bad",
            Kind::Straight {
                a: [0.0; 2],
                b: [0.0; 2],
            },
        )
        .is_ok()
        || invalid != data
    {
        return Err("Invalid ruler partially applied".into());
    }
    d.scene.selection = Some(crate::scene::Rect {
        min: [0.0; 2],
        max: [16.0, 32.0],
    });
    let mut stroke = Stroke::begin(&d, 0, style)?;
    stroke.sample(&d, [5.5, 8.5])?;
    stroke.commit(&mut d)?;
    if d.layers[0].pixel(32, 32, 26, 8)[3] != 0 {
        return Err("Symmetry bypassed selection".into());
    }
    d.undo();
    d.scene.selection = None;
    let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, style, Default::default())?;
    replay.push(crate::mouse_pressure::Sample {
        point: [5.5, 8.5],
        micros: Some(0),
    })?;
    let output = replay.render(&d, true)?;
    if output
        .pixel(&d, 0, &d.layers[0], 26, 8)
        .is_none_or(|p| p[3] != 128)
    {
        return Err("Mouse replay missed symmetry".into());
    }
    Ok(())
}

pub fn lines_for(doc: &Document, g: &Guide, target: crate::line_tools::Target) -> Vec<[Point; 2]> {
    if g.link
        .is_some_and(|t| !crate::ruler_scope::linked_visible(doc, t))
        || g.display == crate::ruler_scope::Display::Hidden
        || g.frames
            .is_some_and(|r| doc.current_frame < r[0] || doc.current_frame > r[1])
        || g.display == crate::ruler_scope::Display::AppliedLayers
            && !crate::ruler_scope::applies(doc, g, target)
    {
        return vec![];
    }
    crate::ruler_scope::transform(doc, g).map_or(vec![], |t| {
        lines(g, t, [doc.width as f64, doc.height as f64])
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn constraints_symmetry_axis_overlap_selection_native_and_replay() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
