//! Strict bounded ASCII/binary STL. Header metadata is never executed or fetched.
use crate::{
    model_mesh::{Asset, Model, Surface, Triangle},
    scene::WorldMesh,
    world_math::{V, cross, dot, sub},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
};
pub(crate) fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Asset, String> {
    let bytes = crate::book_io::read_local(path, (32 * 1024 * 1024).min(budget / 8))?;
    let binary = bytes.len() >= 84
        && 84usize.checked_add(
            (u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize).saturating_mul(50),
        ) == Some(bytes.len());
    let mut faces = vec![];
    let label;
    let mut color = false;
    if binary {
        label = std::str::from_utf8(&bytes[..80])
            .map_err(|_| "Unreadable STL header metadata (expected UTF-8)")?
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        if label.contains('\0') {
            return Err("Unreadable STL header metadata".into());
        }
        if (bytes.len() - 84) / 50 > 131072 {
            return Err("STL triangle limit131072".into());
        }
        for row in bytes[84..].as_chunks::<50>().0 {
            crate::book_io::check_cancel(cancel)?;
            let f = |i| f32::from_le_bytes(row[i..i + 4].try_into().unwrap()) as f64;
            if [f(0), f(4), f(8)].iter().any(|x| !x.is_finite()) {
                return Err("Invalid STL facet normal".into());
            }
            faces.push([
                [f(12), f(16), f(20)],
                [f(24), f(28), f(32)],
                [f(36), f(40), f(44)],
            ]);
            color |= row[48] != 0 || row[49] != 0;
        }
    } else {
        let raw =
            std::str::from_utf8(&bytes).map_err(|_| "Invalid STL binary count / non-UTF8 ASCII")?;
        let mut rows = raw.lines().map(str::trim).filter(|l| !l.is_empty());
        label = rows
            .next()
            .and_then(|s| s.strip_prefix("solid"))
            .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
            .ok_or("ASCII STL must start with solid")?
            .trim()
            .to_owned();
        let mut ended = false;
        while let Some(row) = rows.next() {
            crate::book_io::check_cancel(cancel)?;
            if let Some(end) = row.strip_prefix("endsolid") {
                if (!end.is_empty() && !end.starts_with(char::is_whitespace))
                    || (!end.trim().is_empty() && end.trim() != label)
                    || rows.next().is_some()
                {
                    return Err("STL end label / trailing content mismatch".into());
                }
                ended = true;
                break;
            }
            let parse = |s: &str, prefix: &str| -> Result<V, String> {
                let rest = s.strip_prefix(prefix).ok_or("Invalid STL facet grammar")?;
                let v = rest
                    .split_whitespace()
                    .map(|s| {
                        s.parse::<f64>()
                            .map_err(|_| "Invalid STL number".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                v.try_into()
                    .map_err(|_| "STL vector requires3 numbers".into())
            };
            let n = parse(row, "facet normal ")?;
            if n.iter().any(|x| !x.is_finite()) {
                return Err("Invalid STL facet normal".into());
            }
            if rows.next() != Some("outer loop") {
                return Err("Missing STL outer loop".into());
            }
            let mut triangle = [[0.; 3]; 3];
            for v in &mut triangle {
                *v = parse(rows.next().ok_or("Truncated STL")?, "vertex ")?;
            }
            if rows.next() != Some("endloop") || rows.next() != Some("endfacet") {
                return Err("Missing STL facet end".into());
            }
            faces.push(triangle);
            if faces.len() > 131072 {
                return Err("STL triangle limit131072".into());
            }
        }
        if !ended {
            return Err("Missing STL endsolid".into());
        }
    }
    let mut vertices = vec![];
    let mut indices = BTreeMap::new();
    let mut triangles = vec![];
    let mut edges = BTreeSet::new();
    for f in faces {
        crate::book_io::check_cancel(cancel)?;
        if f.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1e9)
            || dot(
                cross(sub(f[1], f[0]), sub(f[2], f[0])),
                cross(sub(f[1], f[0]), sub(f[2], f[0])),
            ) < 1e-24
        {
            return Err("Invalid / degenerate STL triangle".into());
        }
        let v = f.map(|v| {
            let key = v.map(|x| if x == 0. { 0 } else { x.to_bits() });
            *indices.entry(key).or_insert_with(|| {
                let i = vertices.len() as u32;
                vertices.push(v);
                i
            })
        });
        for i in 0..3 {
            edges.insert([
                v[i].min(v[(i + 1) % 3]) as usize,
                v[i].max(v[(i + 1) % 3]) as usize,
            ]);
        }
        if vertices.len() > 65536 || edges.len() > 65536 {
            return Err("STL vertex / edge limit65536".into());
        }
        triangles.push(Triangle {
            v,
            t: None,
            n: None,
            material: 0,
            smoothing: 0,
        });
    }
    let mut a = Asset {
        models: vec![Model {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            mesh: WorldMesh {
                vertices,
                edges: edges.into_iter().collect(),
                color: [179, 199, 230, 255],
            },
            surface: Surface {
                triangles,
                source: "STL local triangles (unspecified units)".into(),
                ..Default::default()
            },
        }],
        warnings: vec![],
    };
    if color {
        a.warning("STL nonstandard facet color/attribute words are not converted".into());
    }
    a.warning("STL has unspecified units/axes, no standard materials, transparency, UV, textures, skin or morphs; facet normals are regenerated from winding".into());
    let source = crate::model_source::Source {
        format: "STL".into(),
        version: if binary {
            "binary little-endian"
        } else {
            "ASCII"
        }
        .into(),
        file: path.display().to_string(),
        usage_present: false,
        notices: if label.is_empty() {
            vec![]
        } else {
            vec![crate::model_source::Notice {
                source: if binary {
                    "80-byte header"
                } else {
                    "solid label"
                }
                .into(),
                text: label,
            }]
        },
    };
    crate::model_source::attach(&mut a, source)?;
    if !a.valid() {
        return Err("STL contains no usable triangles / exceeds limits".into());
    }
    Ok(a)
}
