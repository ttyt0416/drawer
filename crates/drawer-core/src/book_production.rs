//! Local book setup, physical preflight and perspective preview geometry.
use crate::{document::Document, layer_search::Key, scene::PageObject, story_text::Work};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub version: u32,
    pub front: Option<u64>,
    pub back: Option<u64>,
    pub spine_mm: f64,
    pub minimum_dpi: f64,
    pub bleed_mm: f64,
    pub safe_mm: f64,
    pub trim_mm: [f64; 2],
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            version: 1,
            front: None,
            back: None,
            spine_mm: 5.,
            minimum_dpi: 300.,
            bleed_mm: 3.,
            safe_mm: 5.,
            trim_mm: [0.; 2],
        }
    }
}
impl Setup {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.front.is_none_or(|n| n > 0)
            && self.back.is_none_or(|n| n > 0)
            && (self.front.is_none() || self.front != self.back)
            && (0. ..=100.).contains(&self.spine_mm)
            && (1. ..=4800.).contains(&self.minimum_dpi)
            && (0. ..=100.).contains(&self.bleed_mm)
            && (0. ..=100.).contains(&self.safe_mm)
            && self.trim_mm.iter().all(|v| (0. ..=10000.).contains(v))
            && ((self.trim_mm[0] == 0.) == (self.trim_mm[1] == 0.))
    }
    pub fn validate(&self, work: &Work) -> Result<(), String> {
        if !self.valid()
            || !work.valid()
            || [self.front, self.back]
                .into_iter()
                .flatten()
                .any(|id| !work.page_ids.contains(&id))
        {
            return Err("Invalid book setup or missing cover page ID".into());
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid book setup".into());
        }
        serde_json::to_vec_pretty(self).map_err(|e| e.to_string())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 16384 {
            return Err("Book setup limit16KiB".into());
        }
        let s: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err("Unsupported or invalid book setup".into());
        }
        Ok(s)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub page: u64,
    pub blocking: bool,
    pub message: String,
}
pub fn preflight(work: &Work, pages: &[Document], setup: &Setup) -> Result<Vec<Finding>, String> {
    setup.validate(work)?;
    if pages.len() != work.pages.len() {
        return Err("Book page count changed".into());
    }
    let mut findings: Vec<Finding> = vec![];
    for (id, d) in work.page_ids.iter().copied().zip(pages) {
        let mut report = |blocking, message: String| {
            if findings.len() >= 4096 {
                let last = findings.last_mut().unwrap();
                last.blocking |= blocking;
                last.message =
                    "Further findings omitted; resolve the reported issues and run preflight again"
                        .into();
                return;
            }
            findings.push(Finding {
                page: id,
                blocking,
                message,
            })
        };
        if !d.scene.valid() || !d.dpi.is_finite() || d.dpi <= 0. {
            report(true, "Invalid page document".into());
            continue;
        }
        let p = &d.scene.comics.page;
        match p.guides(d) {
            Err(e) => report(true, e),
            Ok(guides) => {
                if let Some(safe) = guides.get(2) {
                    for (n, o) in d.scene.objects.iter().enumerate() {
                        if matches!(o, PageObject::Text { .. })
                            && let Some(b) = crate::layer_catalog::bounds(
                                d,
                                Key::Object(d.scene.catalog.objects.id(n)),
                            )
                            && (0..2).any(|i| {
                                b.min[i] < safe.min[i] - 1e-6 || b.max[i] > safe.max[i] + 1e-6
                            })
                        {
                            report(
                                false,
                                format!(
                                    "Dialogue object{} crosses the safe area",
                                    d.scene.catalog.objects.id(n)
                                ),
                            );
                        }
                    }
                }
            }
        }
        if d.dpi + 1e-6 < setup.minimum_dpi {
            report(
                false,
                format!(
                    "Page resolution {:.1} DPI is below {:.1}",
                    d.dpi, setup.minimum_dpi
                ),
            );
        }
        if p.bleed_mm + 1e-6 < setup.bleed_mm {
            report(
                false,
                format!("Bleed {:.2}mm is below {:.2}mm", p.bleed_mm, setup.bleed_mm),
            );
        }
        if p.safe_mm + 1e-6 < setup.safe_mm {
            report(
                false,
                format!(
                    "Safe margin {:.2}mm is below {:.2}mm",
                    p.safe_mm, setup.safe_mm
                ),
            );
        }
        if setup.trim_mm[0] > 0. && Some(id) != setup.front && Some(id) != setup.back {
            let trim = std::array::from_fn::<_, 2, _>(|i| {
                if p.trim_mm[i] > 0. {
                    p.trim_mm[i]
                } else {
                    [d.width, d.height][i] as f64 * 25.4 / d.dpi
                }
            });
            if (0..2).any(|i| (trim[i] - setup.trim_mm[i]).abs() > 0.1) {
                report(
                    true,
                    format!(
                        "Trim {:.2}×{:.2}mm differs from requested {:.2}×{:.2}mm",
                        trim[0], trim[1], setup.trim_mm[0], setup.trim_mm[1]
                    ),
                );
            }
        }
        let images = d
            .scene
            .objects
            .iter()
            .enumerate()
            .filter_map(|(n, o)| {
                if let PageObject::Material(m) = o {
                    Some((
                        format!("Image object{}", d.scene.catalog.objects.id(n)),
                        m.as_ref(),
                    ))
                } else {
                    None
                }
            })
            .chain(
                d.scene
                    .file_objects
                    .entries
                    .iter()
                    .map(|l| (format!("Linked image {:?}", l.target), &l.cache)),
            );
        for (label, m) in images {
            if let Some(image) = m.source.image() {
                if !matches!(m.mapping.warp, crate::material_mapping::Warp::None) {
                    report(
                        false,
                        format!("{label}: nonlinear mapping needs resolution review"),
                    );
                    continue;
                }
                let matrix = m.transform.then(d.scene.transform);
                let a = matrix.vector([(m.area.max[0] - m.area.min[0]) / image.width as f64, 0.]);
                let b = matrix.vector([0., (m.area.max[1] - m.area.min[1]) / image.height as f64]);
                let aa = a[0] * a[0] + a[1] * a[1];
                let bb = b[0] * b[0] + b[1] * b[1];
                let ab = a[0] * b[0] + a[1] * b[1];
                let maximum_stretch =
                    ((aa + bb + ((aa - bb).powi(2) + 4. * ab * ab).sqrt()) / 2.).sqrt();
                let dpi = d.dpi / maximum_stretch;
                if dpi < setup.minimum_dpi {
                    report(false, format!("{label} effective resolution {dpi:.1} DPI"));
                }
            }
        }
    }
    Ok(findings)
}
/// Visual left/right slots, with a real blank slot for unpaired covers/end pages.
pub fn spread(work: &Work, index: usize) -> Result<[Option<usize>; 2], String> {
    let model = crate::page_work::Model::new(work.clone())?;
    if index >= work.pages.len() {
        return Err("Page outside book".into());
    }
    let right = work.first_right == index.is_multiple_of(2);
    let other = model.facing(index);
    Ok(if right {
        [other, Some(index)]
    } else {
        [Some(index), other]
    })
}
#[derive(Clone, Debug)]
pub struct Face {
    pub points: [[f64; 2]; 4],
    pub depth: f64,
    pub page: Option<usize>,
    pub spine: bool,
}
/// Millimetre proportions, hinged pages, a physical spine and perspective camera.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    work: &Work,
    setup: &Setup,
    index: usize,
    closed: bool,
    opening: f64,
    yaw: f64,
    pitch: f64,
    size_mm: [f64; 2],
) -> Result<Vec<Face>, String> {
    setup.validate(work)?;
    let aspect = size_mm[0] / size_mm[1];
    if !opening.is_finite()
        || !(0. ..=170.).contains(&opening)
        || !(-180. ..=180.).contains(&yaw)
        || !(-89. ..=89.).contains(&pitch)
        || size_mm.iter().any(|v| !(0.01..=10000.).contains(v))
        || !aspect.is_finite()
        || !(0.05..=20.).contains(&aspect)
    {
        return Err("Invalid preview camera".into());
    }
    let mut slots = if closed {
        [
            setup
                .back
                .and_then(|id| work.page_ids.iter().position(|v| *v == id)),
            setup
                .front
                .and_then(|id| work.page_ids.iter().position(|v| *v == id)),
        ]
    } else {
        spread(work, index)?
    };
    if closed && work.right_binding {
        slots.reverse();
    }
    let thickness = setup.spine_mm / (size_mm[1] / 2.);
    let angle = if closed {
        0.
    } else {
        opening.to_radians() / 2.
    };
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let project = |p: [f64; 3]| {
        let x = p[0] * cy + p[2] * sy;
        let z = -p[0] * sy + p[2] * cy;
        let y = p[1] * cp - z * sp;
        let z = p[1] * sp + z * cp;
        (
            [
                x / (4. + 2. * aspect + thickness - z),
                y / (4. + 2. * aspect + thickness - z),
            ],
            z,
        )
    };
    let mut faces = vec![];
    for (side, page) in slots.into_iter().enumerate() {
        let sign = if side == 0 { -1. } else { 1. };
        let hinge = sign * thickness / 2.;
        let x = sign * 2. * aspect * angle.sin();
        let z = 2. * aspect * angle.cos();
        let points = [
            [hinge, -1., 0.],
            [hinge + x, -1., z],
            [hinge + x, 1., z],
            [hinge, 1., 0.],
        ];
        let points = if side == 0 {
            [points[1], points[0], points[3], points[2]]
        } else {
            points
        };
        let projected = points.map(project);
        faces.push(Face {
            points: projected.map(|p| p.0),
            depth: projected.iter().map(|p| p.1).sum::<f64>() / 4.,
            page,
            spine: false,
        });
    }
    let points = [
        [-thickness / 2., -1., 0.],
        [thickness / 2., -1., 0.],
        [thickness / 2., 1., 0.],
        [-thickness / 2., 1., 0.],
    ]
    .map(project);
    faces.push(Face {
        points: points.map(|p| p.0),
        depth: points.iter().map(|p| p.1).sum::<f64>() / 4.,
        page: None,
        spine: true,
    });
    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    Ok(faces)
}

/// A checked output boundary: physical errors block; warnings need explicit review.
pub fn checked_pdf(
    work: &Work,
    pages: &[Document],
    setup: &Setup,
    reviewed: bool,
    budget: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<u8>, String> {
    let findings = preflight(work, pages, setup)?;
    if findings.iter().any(|f| f.blocking) {
        return Err("Output blocked by trim/reference errors; inspect preflight".into());
    }
    if !findings.is_empty() && !reviewed {
        return Err("Review resolution/bleed/safe-area warnings before output".into());
    }
    let held = pages.iter().map(Document::resident_bytes).sum::<usize>();
    let mut result = vec![];
    let mut used = held;
    for d in pages {
        crate::book_io::check_cancel(cancel)?;
        let mut p = crate::book_io::compose(&[d], d.dpi, budget.saturating_sub(used), cancel)?;
        used = used.saturating_add(p[0].image.memory_bytes());
        result.append(&mut p);
    }
    crate::book_pdf::encode(&result, cancel)
}
