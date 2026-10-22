//! SVG writer with vector outlines and self-contained PNG substitutes.
use crate::{
    document::Document,
    layer_properties::{self as p, Target},
    layer_search::Key,
    scene::{Affine, Point, Rect},
    svg_style::{Clip, Gradient, Kind, Rule, Spread},
    vector_tip::Preset,
};
use base64::Engine;
use std::{
    fmt::Write as _,
    io::{Cursor, Write},
    path::Path,
};
const MAX_XML: usize = 64 * 1024 * 1024;
fn matrix(a: Affine) -> String {
    let [a, b, c, d, e, f] = a.0;
    format!("matrix({a} {b} {c} {d} {e} {f})")
}
fn color(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
fn rule(r: Rule) -> &'static str {
    if r == Rule::EvenOdd {
        "evenodd"
    } else {
        "nonzero"
    }
}
fn path(points: &[Point], closed: bool) -> String {
    let mut s = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(s, "{}{} {} ", if i == 0 { 'M' } else { 'L' }, p[0], p[1]);
    }
    if closed {
        s.push('Z');
    }
    s
}
struct Writer {
    defs: String,
    body: String,
    next: usize,
    warnings: Vec<String>,
    pixels: usize,
}
impl Writer {
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }
    fn limit(&self) -> Result<(), String> {
        if self.body.len() + self.defs.len() > MAX_XML {
            Err("SVG output limit64MiB".into())
        } else {
            Ok(())
        }
    }
    fn gradient(&mut self, g: &Gradient) -> usize {
        let id = self.id();
        let spread = match g.spread {
            Spread::Pad => "pad",
            Spread::Repeat => "repeat",
            Spread::Reflect => "reflect",
        };
        let tag = match g.kind {
            Kind::Linear { start, end } => {
                let _ = write!(
                    self.defs,
                    "<linearGradient id=\"g{id}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                    start[0], start[1], end[0], end[1]
                );
                "linearGradient"
            }
            Kind::Radial {
                center,
                focus,
                radius,
                focal_radius,
            } => {
                let _ = write!(
                    self.defs,
                    "<radialGradient id=\"g{id}\" cx=\"{}\" cy=\"{}\" fx=\"{}\" fy=\"{}\" r=\"{radius}\" fr=\"{focal_radius}\"",
                    center[0], center[1], focus[0], focus[1]
                );
                "radialGradient"
            }
        };
        let _ = write!(
            self.defs,
            " gradientUnits=\"userSpaceOnUse\" gradientTransform=\"{}\" spreadMethod=\"{spread}\">",
            matrix(g.frame.inverse())
        );
        for s in &g.stops {
            let _ = write!(
                self.defs,
                "<stop offset=\"{}\" stop-color=\"{}\" stop-opacity=\"{}\"/>",
                s.offset,
                color(s.color),
                s.color[3] as f64 / 255.
            );
        }
        let _ = write!(self.defs, "</{tag}>");
        id
    }
    fn clip_content(&mut self, c: &Clip, bounds: Rect) -> String {
        match c {
            Clip::Path { points, rule: r } => format!(
                "<path d=\"{}\" fill-rule=\"{}\" clip-rule=\"{}\"/>",
                path(points, true),
                rule(*r),
                rule(*r)
            ),
            Clip::Union(v) => v
                .iter()
                .map(|c| self.clip_content(c, bounds))
                .collect::<String>(),
            Clip::Intersection(_) => {
                let id = self.clip(c, bounds);
                format!(
                    "<path d=\"{}\" clip-path=\"url(#c{id})\"/>",
                    path(
                        &[
                            bounds.min,
                            [bounds.max[0], bounds.min[1]],
                            bounds.max,
                            [bounds.min[0], bounds.max[1]]
                        ],
                        true
                    )
                )
            }
        }
    }
    fn clip(&mut self, c: &Clip, bounds: Rect) -> usize {
        let id = self.id();
        let (content, attribute) = if let Clip::Intersection(v) = c {
            if let Some(first) = v.first() {
                let content = self.clip_content(first, bounds);
                let attribute = if v.len() > 1 {
                    let rest = if v.len() == 2 {
                        self.clip(&v[1], bounds)
                    } else {
                        self.clip(&Clip::Intersection(v[1..].to_vec()), bounds)
                    };
                    format!(" clip-path=\"url(#c{rest})\"")
                } else {
                    String::new()
                };
                (content, attribute)
            } else {
                (
                    format!(
                        "<path d=\"{}\"/>",
                        path(
                            &[
                                bounds.min,
                                [bounds.max[0], bounds.min[1]],
                                bounds.max,
                                [bounds.min[0], bounds.max[1]]
                            ],
                            true
                        )
                    ),
                    String::new(),
                )
            }
        } else {
            (self.clip_content(c, bounds), String::new())
        };
        let _ = write!(
            self.defs,
            "<clipPath id=\"c{id}\" clipPathUnits=\"userSpaceOnUse\"{attribute}>{content}</clipPath>"
        );
        id
    }
    fn vectors(&mut self, d: &Document, id: u64) -> Result<(), String> {
        let layer = d
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .ok_or("SVG vector source missing")?;
        let flags = p::inherited(d, Target::Vector(id));
        if flags.hidden {
            return Ok(());
        }
        let _ = write!(
            self.body,
            "<g transform=\"{}\" opacity=\"{}\">",
            matrix(d.scene.transform),
            flags.opacity as f64 / 100.
        );
        for p in &layer.paths {
            let mut attributes = String::new();
            if let Some(c) = p.svg.as_ref().and_then(|s| s.clip.as_ref()) {
                let clip = self.clip(c, p.bounds());
                let _ = write!(attributes, " clip-path=\"url(#c{clip})\"");
            }
            let fill_rule = p.svg.as_ref().map_or(Rule::EvenOdd, |s| s.rule);
            if let Some(fill) = p.fill {
                let paint = if let Some(g) = p.svg.as_ref().and_then(|s| s.gradient.as_ref()) {
                    let id = self.gradient(g);
                    format!("url(#g{id})")
                } else {
                    color(fill)
                };
                let _ = write!(
                    self.body,
                    "<path d=\"{}\" fill=\"{paint}\" fill-opacity=\"{}\" fill-rule=\"{}\"{attributes}/>",
                    path(&p.points, true),
                    fill[3] as f64 / 255.,
                    rule(fill_rule)
                );
            }
            if p.stroke && p.width > 0. && p.color[3] > 0 {
                if p.tip == Preset::Round && p.pressure.is_default() {
                    let _ = write!(
                        self.body,
                        "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{attributes}/>",
                        path(&p.points, p.closed),
                        color(p.color),
                        p.color[3] as f64 / 255.,
                        p.width
                    );
                } else {
                    let outline = outline(p)?;
                    let _ = write!(
                        self.body,
                        "<path d=\"{outline}\" fill=\"{}\" fill-opacity=\"{}\" fill-rule=\"nonzero\"{attributes}/>",
                        color(p.color),
                        p.color[3] as f64 / 255.
                    );
                    self.warnings.push(format!("Vector path {}: variable width or nonround tip converted to filled outlines",p.id));
                }
            }
            self.limit()?;
        }
        self.body.push_str("</g>");
        Ok(())
    }
    fn raster(
        &mut self,
        d: &Document,
        renderer: &crate::render::Renderer<'_>,
        key: Option<Key>,
        reason: &str,
    ) -> Result<(), String> {
        let count = d.width as usize * d.height as usize;
        self.pixels = self.pixels.saturating_add(count);
        if self.pixels > 16 * 1024 * 1024 {
            return Err("SVG raster substitute work limit16M pixels".into());
        }
        let mut bytes = Vec::with_capacity(count * 4);
        let mut visible = false;
        for y in 0..d.height {
            for x in 0..d.width {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                let p = key.map_or_else(
                    || renderer.pixel(point),
                    |k| renderer.source_pixel(k, point),
                );
                visible |= p[3] > 0;
                bytes.extend_from_slice(&p);
            }
        }
        if !visible {
            return Ok(());
        }
        let image =
            image::RgbaImage::from_raw(d.width, d.height, bytes).ok_or("SVG raster dimensions")?;
        let mut png = Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
        let _ = write!(
            self.body,
            "<image x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{data}\"/>",
            d.width, d.height
        );
        self.warnings.push(reason.into());
        self.limit()
    }
}
fn hull(mut p: Vec<Point>) -> Vec<Point> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |a: Point, b: Point, c: Point| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut out: Vec<Point> = vec![];
    for q in &p {
        while out.len() >= 2 && cross(out[out.len() - 2], out[out.len() - 1], *q) <= 0. {
            out.pop();
        }
        out.push(*q);
    }
    let first = out.len();
    for q in p.iter().rev().skip(1) {
        while out.len() > first && cross(out[out.len() - 2], out[out.len() - 1], *q) <= 0. {
            out.pop();
        }
        out.push(*q);
    }
    out.pop();
    out
}
fn disk(out: &mut String, p: Point, r: f64, flat: bool) {
    if r <= 0. {
        return;
    }
    let y = if flat { r / 3. } else { r };
    let _ = write!(
        out,
        "M{} {} A{r} {y} 0 1 1 {} {} A{r} {y} 0 1 1 {} {} Z ",
        p[0] + r,
        p[1],
        p[0] - r,
        p[1],
        p[0] + r,
        p[1]
    );
}
fn outline(p: &crate::line_tools::Path) -> Result<String, String> {
    let mut out = String::new();
    for (a, b, wa, wb) in p.pressure_segments() {
        let ra = wa / 2.;
        let rb = wb / 2.;
        if ra <= 0. && rb <= 0. {
            continue;
        }
        if let Some(vertices) = p.tip.polygon() {
            let points = vertices
                .into_iter()
                .flat_map(|v| {
                    [
                        [a[0] + v[0] * ra, a[1] + v[1] * ra],
                        [b[0] + v[0] * rb, b[1] + v[1] * rb],
                    ]
                })
                .collect();
            out.push_str(&path(&hull(points), true));
        } else {
            let flat = p.tip == Preset::Flat;
            disk(&mut out, a, ra, flat);
            disk(&mut out, b, rb, flat);
            let factor = if flat { 3. } else { 1. };
            let a = [a[0], a[1] * factor];
            let b = [b[0], b[1] * factor];
            let delta = [b[0] - a[0], b[1] - a[1]];
            let length = delta[0].hypot(delta[1]);
            let dr = rb - ra;
            if length > dr.abs() {
                let u = [delta[0] / length, delta[1] / length];
                let s = dr / length;
                let q = (1. - s * s).max(0.).sqrt();
                let n = [-u[1], u[0]];
                let plus = [-s * u[0] + q * n[0], -s * u[1] + q * n[1]];
                let minus = [-s * u[0] - q * n[0], -s * u[1] - q * n[1]];
                let quad = [
                    [a[0] + ra * plus[0], (a[1] + ra * plus[1]) / factor],
                    [a[0] + ra * minus[0], (a[1] + ra * minus[1]) / factor],
                    [b[0] + rb * minus[0], (b[1] + rb * minus[1]) / factor],
                    [b[0] + rb * plus[0], (b[1] + rb * plus[1]) / factor],
                ];
                out.push_str(&path(&quad, true));
            }
        }
        if out.len() > MAX_XML {
            return Err("SVG stroke outline limit64MiB".into());
        }
    }
    Ok(out)
}
pub struct Export {
    pub text: String,
    pub warnings: Vec<String>,
}
pub fn generate(source: &Document) -> Result<Export, String> {
    let owned = crate::layer_roles::output_view(source)?;
    let d = owned.as_deref().unwrap_or(source);
    let renderer = crate::render::Renderer::new(d);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let mut w = Writer {
        defs: String::new(),
        body: String::new(),
        next: 0,
        warnings: vec![],
        pixels: 0,
    };
    let complex = !d.scene.clipping.is_default()
        || d.color.profile != crate::color_profile::RgbProfile::AssumedSrgb
        || !d.scene.layer_effects.is_default()
        || !d.scene.paper.folders.is_empty()
        || !d.scene.clips.is_empty()
        || d.animation.is_some()
        || p::targets(d)
            .into_iter()
            .any(|t| p::properties(d, t).blend != crate::layer_blend::Mode::Normal);
    if complex {
        w.raster(d,&renderer,None,"Compositing, color profile, masks, page clips or animation require a flattened current-frame SVG raster substitute")?;
    } else {
        for n in 0..d.layers.len() {
            let id = d.scene.layer_ids.id(n);
            if !d.scene.render_order.deferred(id) {
                w.raster(
                    d,
                    &renderer,
                    Some(Key::Layer(Target::Raster(id))),
                    "Raster layer embedded as PNG; source layer remains editable in .drawer",
                )?;
            }
        }
        for e in crate::render_order::normalized(&d.scene) {
            if let Key::Layer(Target::Vector(id)) = e.key {
                if d.scene.file_objects.get(e.key).is_none()
                    && !crate::appearance::active(d, Target::Vector(id))
                {
                    w.vectors(d, id)?;
                } else {
                    w.raster(
                        d,
                        &renderer,
                        Some(e.key),
                        "Linked or appearance-modified vector embedded as PNG",
                    )?;
                }
            } else {
                w.raster(
                    d,
                    &renderer,
                    Some(e.key),
                    "Image, texture, procedural or non-path source embedded as PNG",
                )?;
            }
        }
    }
    let text = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><defs>{}</defs>{}</svg>",
        d.width, d.height, d.width, d.height, w.defs, w.body
    );
    if text.len() > MAX_XML {
        return Err("SVG output limit64MiB".into());
    }
    if text.len() > crate::svg_import::MAX_TEXT {
        w.warnings.push(
            "SVG exceeds this application's2MiB import limit; the native source remains editable"
                .into(),
        );
    }
    Ok(Export {
        text,
        warnings: w.warnings,
    })
}
pub fn write(path: &Path, text: &str) -> Result<(), String> {
    if text.len() > MAX_XML
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("svg"))
    {
        return Err("Choose an SVG destination within the output size limit".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = parent.join(path.file_name().ok_or("Missing SVG filename")?);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Choose a regular SVG destination".into());
    }
    let mut name = path.file_name().unwrap().to_owned();
    name.push(".svg-write.tmp");
    let temporary = path.with_file_name(name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    let result = result.and_then(|_| std::fs::rename(&temporary, &path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}
