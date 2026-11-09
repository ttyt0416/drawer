//! Surface topology kept with the editable wire geometry, independent of files.
use crate::{scene::WorldMesh, world_math::V};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Triangle {
    pub v: [u32; 3],
    pub t: Option<[u32; 3]>,
    pub n: Option<[u32; 3]>,
    pub material: u16,
    pub smoothing: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub name: String,
    pub rgba: [f32; 4],
    pub texture: Option<String>,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            rgba: [0.7, 0.78, 0.9, 1.],
            texture: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub name: String,
    pub present: bool,
    pub texture: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub triangles: Vec<Triangle>,
    pub texcoords: Vec<V>,
    pub normals: Vec<V>,
    pub materials: Vec<Material>,
    pub references: Vec<Reference>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<crate::uv_paint::Texture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported: Option<crate::model_import::Attributes>,
    #[serde(default, skip_serializing_if = "crate::surface_edit::Data::is_default")]
    pub tools: crate::surface_edit::Data,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            triangles: vec![],
            texcoords: vec![],
            normals: vec![],
            materials: vec![Material::default()],
            references: vec![],
            source: "Original primitive".into(),
            paint: None,
            imported: None,
            tools: Default::default(),
        }
    }
}
pub fn relative(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.chars().any(|c| c.is_control() || ":\\".contains(c))
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn label(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control)
}
impl Surface {
    pub fn valid(&self, m: &WorldMesh) -> bool {
        self.tools.valid(self)
            && self
                .imported
                .as_ref()
                .is_none_or(|a| a.valid(self, m.vertices.len()))
            && self.paint.as_ref().is_none_or(|t| t.decode().is_ok())
            && self.triangles.len() <= 131072
            && self.texcoords.len() <= 65536
            && self.normals.len() <= 65536
            && self.materials.len() <= 256
            && !self.materials.is_empty()
            && self.references.len() <= 128
            && label(&self.source)
            && self
                .texcoords
                .iter()
                .chain(&self.normals)
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
            && self
                .normals
                .iter()
                .all(|n| crate::world_math::dot(*n, *n) > 1e-24)
            && self.materials.iter().all(|m| {
                label(&m.name)
                    && m.rgba
                        .into_iter()
                        .all(|n| n.is_finite() && (0. ..=1.).contains(&n))
                    && m.texture.as_ref().is_none_or(|p| relative(p))
            })
            && self.references.iter().all(|r| relative(&r.name))
            && self.triangles.iter().all(|t| {
                t.v.iter().all(|n| (*n as usize) < m.vertices.len())
                    && t.v.iter().collect::<BTreeSet<_>>().len() == 3
                    && (t.material as usize) < self.materials.len()
                    && t.t
                        .is_none_or(|v| v.into_iter().all(|n| (n as usize) < self.texcoords.len()))
                    && t.n
                        .is_none_or(|v| v.into_iter().all(|n| (n as usize) < self.normals.len()))
            })
    }
    pub fn bytes(&self) -> usize {
        self.tools.bytes()
            + self.imported.as_ref().map_or(0, |a| a.bytes())
            + self.paint.as_ref().map_or(0, |t| t.bytes())
            + self.triangles.len() * std::mem::size_of::<Triangle>()
            + (self.texcoords.len() + self.normals.len()) * 24
            + self
                .materials
                .iter()
                .map(|m| m.name.len() + m.texture.as_ref().map_or(0, String::len) + 48)
                .sum::<usize>()
            + self
                .references
                .iter()
                .map(|r| r.name.len() + 32)
                .sum::<usize>()
            + self.source.len()
    }
}
#[derive(Clone)]
pub struct Model {
    pub name: String,
    pub mesh: WorldMesh,
    pub surface: Surface,
}
#[derive(Clone, Default)]
pub struct Asset {
    pub models: Vec<Model>,
    pub warnings: Vec<String>,
}
impl Asset {
    pub fn valid(&self) -> bool {
        !self.models.is_empty()
            && self.models.len() <= 128
            && self.warnings.len() <= 128
            && self.warnings.iter().all(|s| label(s))
            && self.models.iter().all(|m| {
                label(&m.name)
                    && !m.mesh.vertices.is_empty()
                    && m.mesh.vertices.len() <= 65536
                    && m.mesh.edges.len() <= 65536
                    && m.mesh
                        .vertices
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && v.abs() <= 1e9)
                    && m.mesh
                        .edges
                        .iter()
                        .flatten()
                        .all(|i| *i < m.mesh.vertices.len())
                    && (!m.mesh.edges.is_empty() || !m.surface.triangles.is_empty())
                    && m.surface.valid(&m.mesh)
            })
    }
    pub fn warning(&mut self, s: String) {
        if !self.warnings.contains(&s) {
            if self.warnings.len() < 127 {
                self.warnings.push(s);
            } else if self.warnings.len() == 127 {
                self.warnings
                    .push("Additional warnings omitted after127 entries".into());
            }
        }
    }
}
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
/// Ear clipping rejects crossed / nonplanar polygons; corner indices retain UVs
/// and normals rather than assigning a face's attributes to another corner.
pub fn triangulate(
    vertices: &[V],
    face: &[usize],
    cancel: &AtomicBool,
) -> Result<Vec<[usize; 3]>, String> {
    if !(3..=256).contains(&face.len())
        || face.iter().collect::<BTreeSet<_>>().len() != face.len()
        || face.iter().any(|i| *i >= vertices.len())
    {
        return Err("Face needs3..256 distinct existing vertices".into());
    }
    let o = vertices[face[0]];
    let points = face
        .iter()
        .map(|i| crate::world_math::sub(vertices[*i], o))
        .collect::<Vec<_>>();
    let mut normal = [0.; 3];
    for i in 1..points.len() - 1 {
        normal = crate::world_math::add(normal, crate::world_math::cross(points[i], points[i + 1]));
    }
    let extent = points.iter().flatten().map(|v| v.abs()).fold(0., f64::max);
    let eps = (extent * extent * 1e-12).max(1e-24);
    if crate::world_math::dot(normal, normal).sqrt() <= eps {
        return Err("Degenerate or crossed polygon".into());
    }
    normal = crate::world_math::unit(normal);
    if points
        .iter()
        .any(|p| crate::world_math::dot(*p, normal).abs() > extent * 1e-6)
    {
        return Err("Triangulate nonplanar polygons before import".into());
    }
    let drop = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    let axes = (0..3).filter(|i| *i != drop).collect::<Vec<_>>();
    let p = points
        .iter()
        .map(|p| [p[axes[0]], p[axes[1]]])
        .collect::<Vec<_>>();
    for i in 0..p.len() {
        crate::book_io::check_cancel(cancel)?;
        for j in i + 1..p.len() {
            let a = (i + 1) % p.len();
            let b = (j + 1) % p.len();
            if a == j || b == i {
                continue;
            }
            let x = cross(p[i], p[a], p[j]);
            let y = cross(p[i], p[a], p[b]);
            let z = cross(p[j], p[b], p[i]);
            let w = cross(p[j], p[b], p[a]);
            if x * y < -eps * eps && z * w < -eps * eps {
                return Err("Crossed polygon edges".into());
            }
        }
    }
    let area = (0..p.len())
        .map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[i][1] * p[(i + 1) % p.len()][0])
        .sum::<f64>();
    if area.abs() <= eps {
        return Err("Zero-area polygon".into());
    }
    let sign = area.signum();
    let mut ring = (0..p.len()).collect::<Vec<_>>();
    let mut out = vec![];
    while ring.len() > 3 {
        crate::book_io::check_cancel(cancel)?;
        let ear = (0..ring.len())
            .find(|i| {
                let a = ring[(*i + ring.len() - 1) % ring.len()];
                let b = ring[*i];
                let c = ring[(*i + 1) % ring.len()];
                cross(p[a], p[b], p[c]) * sign > eps
                    && !ring.iter().any(|q| {
                        *q != a
                            && *q != b
                            && *q != c
                            && cross(p[a], p[b], p[*q]) * sign >= -eps
                            && cross(p[b], p[c], p[*q]) * sign >= -eps
                            && cross(p[c], p[a], p[*q]) * sign >= -eps
                    })
            })
            .ok_or("Polygon cannot be triangulated safely")?;
        out.push([
            ring[(ear + ring.len() - 1) % ring.len()],
            ring[ear],
            ring[(ear + 1) % ring.len()],
        ]);
        ring.remove(ear);
    }
    if cross(p[ring[0]], p[ring[1]], p[ring[2]]) * sign <= eps {
        return Err("Degenerate final triangle".into());
    }
    out.push([ring[0], ring[1], ring[2]]);
    Ok(out)
}
