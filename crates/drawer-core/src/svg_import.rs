//! Bounded, offline SVG import into owned vector outlines and embedded image materials.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, Point},
    sizing::DocumentState,
    svg_style::{Clip, Gradient, Kind, Rule, Spread, Stop, Style},
};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use usvg::{
    Node, Paint,
    tiny_skia_path::{Path as SkPath, PathSegment},
};
pub const MAX_TEXT: usize = 2 * 1024 * 1024;
fn affine(t: usvg::Transform) -> Affine {
    Affine([
        t.sx as f64,
        t.ky as f64,
        t.kx as f64,
        t.sy as f64,
        t.tx as f64,
        t.ty as f64,
    ])
}
fn skia(t: Affine) -> usvg::Transform {
    let [a, b, c, d, e, f] = t.0;
    usvg::Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32)
}
fn point(p: usvg::tiny_skia_path::Point) -> Point {
    [p.x as f64, p.y as f64]
}
fn middle(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]
}
fn curve(out: &mut Vec<Point>, p: [Point; 4], depth: usize) -> Result<(), String> {
    let error = crate::line_tools::distance(p[1], p[0], p[3])
        .0
        .max(crate::line_tools::distance(p[2], p[0], p[3]).0);
    if error <= 0.02 || depth >= 16 {
        out.push(p[3]);
    } else {
        let a = middle(p[0], p[1]);
        let b = middle(p[1], p[2]);
        let c = middle(p[2], p[3]);
        let d = middle(a, b);
        let e = middle(b, c);
        let f = middle(d, e);
        curve(out, [p[0], a, d, f], depth + 1)?;
        curve(out, [f, e, c, p[3]], depth + 1)?;
    }
    if out.len() > 4096 {
        return Err("SVG curve exceeds4096 outline points".into());
    }
    Ok(())
}
pub(crate) fn contours(path: &SkPath, matrix: Affine) -> Result<Vec<(Vec<Point>, bool)>, String> {
    let mut out = vec![];
    let mut points = vec![];
    let mut last = [0.; 2];
    let mut total = 0;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                if !points.is_empty() {
                    total += points.len();
                    out.push((std::mem::take(&mut points), false));
                }
                last = matrix.map(point(p));
                points.push(last);
            }
            PathSegment::LineTo(p) => {
                last = matrix.map(point(p));
                points.push(last);
            }
            PathSegment::QuadTo(b, c) => {
                let b = matrix.map(point(b));
                let c = matrix.map(point(c));
                let a = [
                    last[0] + (b[0] - last[0]) * 2. / 3.,
                    last[1] + (b[1] - last[1]) * 2. / 3.,
                ];
                let b = [
                    c[0] + (b[0] - c[0]) * 2. / 3.,
                    c[1] + (b[1] - c[1]) * 2. / 3.,
                ];
                curve(&mut points, [last, a, b, c], 0)?;
                last = c;
            }
            PathSegment::CubicTo(a, b, c) => {
                let c = matrix.map(point(c));
                curve(
                    &mut points,
                    [last, matrix.map(point(a)), matrix.map(point(b)), c],
                    0,
                )?;
                last = c;
            }
            PathSegment::Close => {
                if points.len() > 1 && points.first() == points.last() {
                    points.pop();
                }
                if !points.is_empty() {
                    total += points.len();
                    out.push((std::mem::take(&mut points), true));
                }
            }
        }
        if total + points.len() > 4096 {
            return Err("SVG path exceeds4096 sampled points".into());
        }
    }
    if !points.is_empty() {
        out.push((points, false));
    }
    Ok(out)
}
fn compound(contours: Vec<(Vec<Point>, bool)>) -> Vec<Point> {
    let mut out = vec![];
    for (mut p, _) in contours.into_iter().filter(|(p, _)| p.len() >= 3) {
        if out.is_empty() {
            out.append(&mut p);
            out.push(out[0]);
        } else {
            let bridge = out[0];
            out.push(p[0]);
            out.extend_from_slice(&p[1..]);
            out.push(p[0]);
            out.push(bridge);
        }
    }
    out
}
fn rule(r: usvg::FillRule) -> Rule {
    if r == usvg::FillRule::EvenOdd {
        Rule::EvenOdd
    } else {
        Rule::NonZero
    }
}
fn clip_group(group: &usvg::Group, base: Affine, depth: usize) -> Result<Clip, String> {
    if depth > 16 {
        return Err("SVG clipping nesting limit16".into());
    }
    let mut children = vec![];
    for node in group.children() {
        match node {
            Node::Path(p) if p.is_visible() => {
                let points = compound(contours(p.data(), affine(p.abs_transform()).then(base))?);
                if points.len() >= 3 {
                    children.push(Clip::Path {
                        points,
                        rule: p.fill().map_or(Rule::NonZero, |f| rule(f.rule())),
                    });
                }
            }
            Node::Group(g) => {
                let mut c = clip_group(g, base, depth + 1)?;
                if let Some(extra) = g.clip_path() {
                    c = Clip::Intersection(vec![
                        c,
                        clip_path(extra, affine(g.abs_transform()).then(base), depth + 1)?,
                    ]);
                }
                children.push(c);
            }
            _ => {}
        }
    }
    Ok(Clip::Union(children))
}
fn clip_path(clip: &usvg::ClipPath, parent: Affine, depth: usize) -> Result<Clip, String> {
    let mut c = clip_group(
        clip.root(),
        affine(clip.transform()).then(parent),
        depth + 1,
    )?;
    if let Some(extra) = clip.clip_path() {
        c = Clip::Intersection(vec![c, clip_path(extra, parent, depth + 1)?]);
    }
    Ok(c)
}
fn paint(
    paint: &Paint,
    opacity: f32,
    path_frame: Affine,
) -> Result<([u8; 4], Option<Gradient>), String> {
    let alpha = (opacity * 255.).round() as u8;
    if let Paint::Color(c) = paint {
        return Ok(([c.red, c.green, c.blue, alpha], None));
    }
    let (base, kind) = match paint {
        Paint::LinearGradient(g) => (
            g as &usvg::BaseGradient,
            Kind::Linear {
                start: [g.x1() as f64, g.y1() as f64],
                end: [g.x2() as f64, g.y2() as f64],
            },
        ),
        Paint::RadialGradient(g) => (
            g as &usvg::BaseGradient,
            Kind::Radial {
                center: [g.cx() as f64, g.cy() as f64],
                focus: [g.fx() as f64, g.fy() as f64],
                radius: g.r().get() as f64,
                focal_radius: g.fr().get() as f64,
            },
        ),
        _ => return Err("SVG pattern requires raster conversion".into()),
    };
    if base.stops().len() > 256 {
        return Err("SVG gradient stop limit256".into());
    }
    let gradient = Gradient {
        frame: affine(base.transform()).then(path_frame).inverse(),
        kind,
        spread: match base.spread_method() {
            usvg::SpreadMethod::Pad => Spread::Pad,
            usvg::SpreadMethod::Repeat => Spread::Repeat,
            usvg::SpreadMethod::Reflect => Spread::Reflect,
        },
        stops: base
            .stops()
            .iter()
            .map(|s| {
                let c = s.color();
                Stop {
                    offset: s.offset().get() as f64,
                    color: [
                        c.red,
                        c.green,
                        c.blue,
                        (s.opacity().get() * 255.).round() as u8,
                    ],
                }
            })
            .collect(),
    };
    Ok(([0, 0, 0, alpha], Some(gradient)))
}
fn image_data(mime: &str, data: Arc<Vec<u8>>) -> Option<usvg::ImageKind> {
    if data.len() > 8 * 1024 * 1024 || !matches!(mime, "image/png" | "image/jpeg" | "image/jpg") {
        return None;
    }
    let format = image::guess_format(&data).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return None;
    }
    let reader = image::ImageReader::with_format(Cursor::new(data.as_slice()), format);
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 4 * 1024 * 1024 {
        return None;
    }
    if format == image::ImageFormat::Png {
        return Some(usvg::ImageKind::PNG(data));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(data.as_slice()), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?;
    let mut out = Cursor::new(vec![]);
    rgba.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(usvg::ImageKind::PNG(Arc::new(out.into_inner())))
}
fn local_file(href: &str, root: Option<&Path>) -> Option<Vec<u8>> {
    if href.contains(':') || href.contains('?') || href.contains('#') {
        return None;
    }
    let mut decoded = vec![];
    let b = href.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            decoded.push(b[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.contains(':') || decoded.chars().any(|c| c.is_control()) {
        return None;
    }
    let path = Path::new(&decoded);
    if path.is_absolute()
        || path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let root = root?.canonicalize().ok()?;
    let path = root.join(path).canonicalize().ok()?;
    if !path.starts_with(root) || !path.is_file() {
        return None;
    }
    let mut bytes = vec![];
    std::fs::File::open(path)
        .ok()?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 8 * 1024 * 1024).then_some(bytes)
}
pub(crate) fn parse(text: &str, base: Option<&Path>) -> Result<(usvg::Tree, Vec<String>), String> {
    if text.len() > MAX_TEXT {
        return Err("SVG source limit2MiB".into());
    }
    let xml =
        usvg::roxmltree::Document::parse(text).map_err(|e| format!("Invalid SVG XML: {e}"))?;
    if xml.root_element().tag_name().name() != "svg"
        || xml.descendants().count() > 8192
        || xml.descendants().any(|n| n.ancestors().count() > 64)
    {
        return Err("SVG root, node or nesting limit".into());
    }
    let mut warnings = vec![];
    let mut removed = vec![];
    for (index, n) in xml.descendants().filter(|n| n.is_element()).enumerate() {
        let tag = n.tag_name().name();
        let label = format!(
            "{tag} #{}",
            n.attribute("id")
                .map(|s| s.chars().take(64).collect::<String>())
                .unwrap_or_else(|| index.to_string())
        );
        if matches!(tag, "script" | "text" | "foreignObject" | "use" | "pattern") {
            warnings.push(format!("{label}: unsupported element skipped"));
            removed.push(n.range());
        }
        if tag == "filter" {
            warnings.push(format!(
                "{label}: filter is converted to an embedded raster"
            ));
        }
        if tag == "path"
            && let Some(d) = n.attribute("d")
            && svgtypes::PathParser::from(d).any(|s| s.is_err())
        {
            return Err(format!("{label}: invalid path commands"));
        }
        for a in n.attributes() {
            if a.name() == "href"
                && !matches!(tag, "image" | "feImage")
                && !a.value().starts_with('#')
            {
                warnings.push(format!("{label}: nonlocal reference ignored"));
            }
            if a.value().contains("url(") && a.value().contains("://") {
                warnings.push(format!("{label}: external paint resource ignored"));
            }
            if a.name().starts_with("on") {
                warnings.push(format!("{label}: event handler ignored"));
            }
        }
    }
    removed.sort_by_key(|r| r.start);
    let mut sanitized = String::new();
    let mut previous = 0;
    for range in removed {
        if range.start >= previous {
            sanitized.push_str(&text[previous..range.start]);
            previous = range.end;
        }
    }
    sanitized.push_str(&text[previous..]);
    let shared = Arc::new(Mutex::new(warnings));
    let wd = shared.clone();
    let ws = shared.clone();
    let root: Option<PathBuf> = base.map(Path::to_path_buf);
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime, data, _| {
                let result = image_data(mime, data);
                if result.is_none()
                    && let Ok(mut w) = wd.lock()
                {
                    w.push(format!(
                        "Embedded {mime}: unsupported or oversized image skipped"
                    ));
                }
                result
            }),
            resolve_string: Box::new(move |href, _| {
                let result = local_file(href, root.as_deref()).and_then(|bytes| {
                    let mime = match image::guess_format(&bytes).ok()? {
                        image::ImageFormat::Png => "image/png",
                        image::ImageFormat::Jpeg => "image/jpeg",
                        _ => return None,
                    };
                    image_data(mime, Arc::new(bytes))
                });
                if result.is_none()
                    && let Ok(mut w) = ws.lock()
                {
                    w.push(format!("Image reference {}: outside allowed local PNG/JPEG resources, missing or oversized",href.chars().take(200).collect::<String>()));
                }
                result
            }),
        },
        ..Default::default()
    };
    let tree =
        usvg::Tree::from_str(&sanitized, &options).map_err(|e| format!("SVG parse failed: {e}"))?;
    if tree.size().width() > 32768. || tree.size().height() > 32768. {
        return Err("SVG viewport limit32768 pixels".into());
    }
    let warnings = shared.lock().map_err(|_| "SVG diagnostic state")?.clone();
    Ok((tree, warnings))
}
pub struct Prepared {
    pub state: DocumentState,
    pub warnings: Vec<String>,
    pub paths: usize,
    pub images: usize,
    pub size: [f64; 2],
    pub layer: Option<u64>,
}
struct Builder<'a> {
    complexity: usize,
    source: &'a Document,
    state: DocumentState,
    order: Vec<crate::render_order::Entry>,
    run: Option<u64>,
    first: Option<u64>,
    warnings: Vec<String>,
    paths: usize,
    images: usize,
    size: [f64; 2],
}
impl Builder<'_> {
    fn add(&mut self, points: Vec<Point>, color: [u8; 4], style: Style) -> Result<(), String> {
        if points.len() < 3 {
            return Ok(());
        }
        if points.len() > 4096
            || !points.iter().copied().all(crate::line_tools::point_valid)
            || !style.valid()
        {
            return Err("SVG geometry, paint or clipping limits".into());
        }
        let layer = if let Some(id) = self.run {
            id
        } else {
            let id = self.state.scene.lines.add_layer("SVG outlines")?;
            self.run = Some(id);
            self.first = self.first.or(Some(id));
            self.order.push(crate::render_order::Entry {
                key: crate::layer_search::Key::Layer(crate::layer_properties::Target::Vector(id)),
                group: crate::render_order::Group::Vector,
            });
            id
        };
        let id = self
            .state
            .scene
            .lines
            .add_path(layer, points, true, 0., color)?;
        let p = self
            .state
            .scene
            .lines
            .layers
            .iter_mut()
            .find(|l| l.id == layer)
            .unwrap()
            .paths
            .iter_mut()
            .find(|p| p.id == id)
            .unwrap();
        p.stroke = false;
        p.fill = Some(color);
        p.svg = Some(Box::new(style));
        self.paths += 1;
        Ok(())
    }
    fn raster(
        &mut self,
        node: &Node,
        parent: Affine,
        clip: &Clip,
        reason: &str,
    ) -> Result<(), String> {
        let Some(b) = node.abs_layer_bounding_box() else {
            return Ok(());
        };
        let x = (b.x() as f64 - 2.).floor().max(0.);
        let y = (b.y() as f64 - 2.).floor().max(0.);
        let right = (b.right() as f64 + 2.).ceil().min(self.size[0]);
        let bottom = (b.bottom() as f64 + 2.).ceil().min(self.size[1]);
        if right <= x || bottom <= y {
            return Ok(());
        }
        let w = (right - x) as u32;
        let h = (bottom - y) as u32;
        if !matches!(node, Node::Image(_))
            && (w as usize)
                .saturating_mul(h as usize)
                .saturating_mul(self.complexity)
                > 1024 * 1024
        {
            return Err("SVG complex raster substitute work limit; simplify filters, masks or isolated groups".into());
        }
        if u64::from(w) * u64::from(h) > 4 * 1024 * 1024 {
            return Err("SVG raster substitute limit4M pixels".into());
        }
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("SVG raster allocation")?;
        let matrix = Affine([1., 0., 0., 1., b.x() as f64, b.y() as f64])
            .then(parent)
            .then(Affine([1., 0., 0., 1., -x, -y]));
        resvg::render_node(node, skia(matrix), &mut pixmap.as_mut())
            .ok_or("SVG node raster conversion")?;
        let inverse = self.source.scene.transform.inverse();
        let mut pixels = Vec::with_capacity(w as usize * h as usize);
        for j in 0..h {
            for i in 0..w {
                let p = pixmap.pixel(i, j).unwrap().demultiply();
                pixels.push(
                    if clip.hit(inverse.map([x + i as f64 + 0.5, y + j as f64 + 0.5])) {
                        [p.red(), p.green(), p.blue(), p.alpha()]
                    } else {
                        [0; 4]
                    },
                );
            }
        }
        if self.state.layers.len() >= 256 {
            return Err("SVG raster layer limit256".into());
        }
        let placement = crate::document::PlacedRaster {
            width: w,
            height: h,
            offset: [x as i64, y as i64],
        };
        let mut layer = crate::document::RasterLayer {
            precise: None,
            locked: false,
            name: "SVG embedded raster".into(),
            pixels,
            placement: Some(placement),
        };
        if self.source.color != crate::color_storage::Config::legacy() {
            let transform = crate::color_profile::Transform::new(
                &crate::color_profile::RgbProfile::AssumedSrgb,
                &self.source.color.profile,
            )?;
            let samples = layer
                .pixels
                .iter()
                .map(|p| transform.map(crate::color_precision::Sample::from8(*p)))
                .collect::<Result<Vec<_>, _>>()?;
            crate::color_storage::set_samples(&mut layer, &self.source.color, placement, samples)?;
        }
        self.state
            .scene
            .layer_ids
            .initialize(self.state.layers.len())?;
        self.state.layers.push(layer);
        self.state.scene.layer_ids.append(self.state.layers.len())?;
        if self
            .state
            .memory_bytes()
            .saturating_add(self.source.memory_bytes())
            > DEFAULT_DOCUMENT_BUDGET / 4
        {
            return Err("SVG embedded image snapshot budget".into());
        }
        let id = self.state.scene.layer_ids.id(self.state.layers.len() - 1);
        if let Node::Group(g) = node {
            use crate::layer_blend::Mode as M;
            let blend = match g.blend_mode() {
                usvg::BlendMode::Normal => M::Normal,
                usvg::BlendMode::Multiply => M::Multiply,
                usvg::BlendMode::Screen => M::Screen,
                usvg::BlendMode::Overlay => M::Overlay,
                usvg::BlendMode::Darken => M::Darken,
                usvg::BlendMode::Lighten => M::Lighten,
                usvg::BlendMode::ColorDodge => M::ColorDodge,
                usvg::BlendMode::ColorBurn => M::ColorBurn,
                usvg::BlendMode::HardLight => M::HardLight,
                usvg::BlendMode::SoftLight => M::SoftLight,
                usvg::BlendMode::Difference => M::Difference,
                usvg::BlendMode::Exclusion => M::Exclusion,
                usvg::BlendMode::Hue => M::Hue,
                usvg::BlendMode::Saturation => M::Saturation,
                usvg::BlendMode::Color => M::Color,
                usvg::BlendMode::Luminosity => M::Brightness,
            };
            if blend != M::Normal {
                self.state
                    .scene
                    .layer_properties
                    .rasters
                    .entry(id)
                    .or_default()
                    .blend = blend;
            }
        }
        self.order.push(crate::render_order::Entry {
            key: crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(id)),
            group: crate::render_order::Group::Vector,
        });
        self.run = None;
        self.images += 1;
        self.warnings.push(format!(
            "{}: {reason}; embedded raster, geometry is not retained for this element",
            node.id()
        ));
        Ok(())
    }
    fn group(&mut self, g: &usvg::Group, clip: Clip, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("SVG group depth limit".into());
        }
        let inverse = self.source.scene.transform.inverse();
        let clip = if let Some(c) = g.clip_path() {
            Clip::Intersection(vec![
                clip,
                clip_path(c, affine(g.abs_transform()).then(inverse), 0)?,
            ])
        } else {
            clip
        };
        for node in g.children() {
            match node {
                Node::Group(child) => {
                    if child.opacity().get() != 1.
                        || child.blend_mode() != usvg::BlendMode::Normal
                        || child.mask().is_some()
                        || !child.filters().is_empty()
                        || child.isolate()
                    {
                        self.raster(
                            node,
                            affine(g.abs_transform()),
                            &clip,
                            "isolated group opacity, blend, mask or filter",
                        )?;
                    } else {
                        self.group(child, clip.clone(), depth + 1)?;
                    }
                }
                Node::Image(_) => self.raster(
                    node,
                    affine(g.abs_transform()),
                    &clip,
                    "local image snapshot",
                )?,
                Node::Path(p) if p.is_visible() => {
                    if p.fill()
                        .is_some_and(|f| matches!(f.paint(), Paint::Pattern(_)))
                        || p.stroke()
                            .is_some_and(|s| matches!(s.paint(), Paint::Pattern(_)))
                    {
                        self.raster(node, affine(g.abs_transform()), &clip, "pattern paint")?;
                        continue;
                    }
                    let frame = affine(p.abs_transform()).then(inverse);
                    let matrix = affine(p.abs_transform());
                    let mut records = vec![];
                    if let Some(f) = p.fill() {
                        let points = compound(contours(p.data(), matrix)?)
                            .into_iter()
                            .map(|p| inverse.map(p))
                            .collect();
                        let (color, gradient) = paint(f.paint(), f.opacity().get(), frame)?;
                        records.push((
                            points,
                            color,
                            Style {
                                rule: rule(f.rule()),
                                gradient,
                                clip: Some(clip.clone()),
                            },
                        ));
                    }
                    if let Some(s) = p.stroke() {
                        let scale = (crate::line_tools::maximum_scale(matrix).max(1.) * 8.) as f32;
                        let mut stroke = s.to_tiny_skia();
                        let dashed = if let Some(dash) = stroke.dash.take() {
                            p.data()
                                .dash(&dash, scale)
                                .ok_or("SVG stroke dash conversion")?
                        } else {
                            p.data().clone()
                        };
                        let outline = dashed
                            .stroke(&stroke, scale)
                            .ok_or("SVG stroke outline conversion")?;
                        let points = compound(contours(&outline, matrix)?)
                            .into_iter()
                            .map(|p| inverse.map(p))
                            .collect();
                        let (color, gradient) = paint(s.paint(), s.opacity().get(), frame)?;
                        records.push((
                            points,
                            color,
                            Style {
                                rule: Rule::NonZero,
                                gradient,
                                clip: Some(clip.clone()),
                            },
                        ));
                        self.warnings.push(format!("{}: stroke converted to an editable filled outline; caps, joins and dashes are baked",p.id()));
                    }
                    if p.paint_order() == usvg::PaintOrder::StrokeAndFill && records.len() == 2 {
                        records.reverse();
                    }
                    for (points, color, style) in records {
                        self.add(points, color, style)?;
                    }
                }
                Node::Text(_) => self.warnings.push(format!("{}: text skipped", node.id())),
                _ => {}
            }
        }
        Ok(())
    }
}
pub fn prepare(d: &Document, text: &str, base: Option<&Path>) -> Result<Prepared, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("SVG snapshot budget".into());
    }
    let (tree, warnings) = parse(text, base)?;
    let size = [tree.size().width() as f64, tree.size().height() as f64];
    let inverse = d.scene.transform.inverse();
    let root_clip = Clip::Path {
        points: vec![[0., 0.], [size[0], 0.], size, [0., size[1]]]
            .into_iter()
            .map(|p| inverse.map(p))
            .collect(),
        rule: Rule::NonZero,
    };
    let mut b = Builder {
        complexity: text.matches('<').count().max(1),
        source: d,
        state: d.snapshot(),
        order: crate::render_order::normalized(&d.scene),
        run: None,
        first: None,
        warnings,
        paths: 0,
        images: 0,
        size,
    };
    b.group(tree.root(), root_clip, 0)?;
    b.order.sort_by_key(|e| e.group);
    if b.paths + b.images > 0 {
        b.state.scene.render_order.entries = b.order;
    }
    if !b.state.scene.valid()
        || serde_json::to_vec(&b.state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        || b.state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("SVG import exceeds scene, geometry or Undo limits".into());
    }
    crate::layer_properties::validate_edit(d, &b.state)?;
    Ok(Prepared {
        state: b.state,
        warnings: b.warnings,
        paths: b.paths,
        images: b.images,
        size,
        layer: b.first,
    })
}
