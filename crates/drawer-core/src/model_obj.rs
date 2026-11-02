//! Bounded local polygon OBJ reader. No command execution or recursive includes.
use crate::{
    model_mesh::{self as mesh, Asset, Material, Model, Reference, Surface, Triangle},
    scene::WorldMesh,
    world_math::V,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
const MAX_FILE: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
struct Corner {
    v: usize,
    t: Option<usize>,
    n: Option<usize>,
}
#[derive(Default)]
struct Group {
    name: String,
    tris: Vec<([Corner; 3], u16, u16)>,
    edges: BTreeSet<[usize; 2]>,
}
fn float(s: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && n.abs() <= 1e9)
        .ok_or_else(|| "Invalid / excessive OBJ number".into())
}
fn index(s: &str, count: usize) -> Result<usize, String> {
    let n = s.parse::<i64>().map_err(|_| "Invalid OBJ index")?;
    let n = if n < 0 { count as i64 + n } else { n - 1 };
    if n < 0 || n >= count as i64 {
        return Err("OBJ index outside declared vertices".into());
    }
    Ok(n as usize)
}
fn corner(s: &str, v: usize, t: usize, n: usize) -> Result<Corner, String> {
    let c = s.split('/').collect::<Vec<_>>();
    if c.is_empty() || c.len() > 3 {
        return Err("Invalid face corner".into());
    }
    Ok(Corner {
        v: index(c[0], v)?,
        t: c.get(1)
            .filter(|s| !s.is_empty())
            .map(|s| index(s, t))
            .transpose()?,
        n: c.get(2)
            .filter(|s| !s.is_empty())
            .map(|s| index(s, n))
            .transpose()?,
    })
}
fn edge(group: &mut Group, a: usize, b: usize) {
    if a != b {
        group.edges.insert([a.min(b), a.max(b)]);
    }
}
fn name(s: &str) -> Result<String, String> {
    if s.trim().is_empty() || s.len() > 1024 || s.chars().any(char::is_control) {
        return Err("Invalid model / material name".into());
    }
    Ok(s.trim().into())
}
fn logical(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() > MAX_FILE {
        return Err("OBJ exceeds8MiB limit".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "OBJ must be UTF-8 / ASCII text")?;
    let mut out = vec![];
    let mut pending = String::new();
    for row in text.trim_start_matches('\u{feff}').lines() {
        let row = row.split('#').next().unwrap_or("").trim();
        if pending.len() + row.len() > 16384 {
            return Err("OBJ logical line exceeds16KiB".into());
        }
        let more = row.ends_with('\\');
        pending.push_str(if more {
            row[..row.len() - 1].trim_end()
        } else {
            row
        });
        if more {
            pending.push(' ');
        } else {
            out.push(std::mem::take(&mut pending));
        }
        if out.len() > 500000 {
            return Err("OBJ line count limit".into());
        }
    }
    if !pending.is_empty() {
        return Err("Unfinished OBJ continuation".into());
    }
    Ok(out)
}
pub struct Parsed {
    pub asset: Asset,
    pub libraries: Vec<String>,
}
pub fn decode(bytes: &[u8], source: &str, cancel: &AtomicBool) -> Result<Parsed, String> {
    crate::book_io::check_cancel(cancel)?;
    let mut vertices: Vec<V> = vec![];
    let mut tex: Vec<V> = vec![];
    let mut normals: Vec<V> = vec![];
    let mut groups = vec![Group {
        name: "Model".into(),
        ..Default::default()
    }];
    let mut materials = vec![Material::default()];
    let mut material = 0u16;
    let mut smooth = 0u16;
    let mut libraries = vec![];
    let mut asset = Asset::default();
    let mut total_triangles = 0usize;
    for (line, row) in logical(bytes)?.iter().enumerate() {
        crate::book_io::check_cancel(cancel)?;
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.is_empty() {
            continue;
        }
        let args = &fields[1..];
        let result = (|| {
            match fields[0] {
                "v" => {
                    if !(3..=4).contains(&args.len())
                        || args.get(3).is_some_and(|v| float(v).ok() != Some(1.))
                    {
                        return Err("Use geometric v x y z [w=1] records".into());
                    }
                    vertices.push([float(args[0])?, float(args[1])?, float(args[2])?]);
                }
                "vt" => {
                    if args.is_empty() || args.len() > 3 {
                        return Err("Invalid texture coordinate".into());
                    }
                    let mut p = [0.; 3];
                    for (i, a) in args.iter().enumerate() {
                        p[i] = float(a)?;
                    }
                    tex.push(p);
                }
                "vn" => {
                    if args.len() != 3 {
                        return Err("Invalid vertex normal".into());
                    }
                    let p = [float(args[0])?, float(args[1])?, float(args[2])?];
                    if crate::world_math::dot(p, p) <= 1e-24 {
                        return Err("Zero vertex normal".into());
                    }
                    normals.push(p);
                }
                "o" => {
                    let n = name(&args.join(" "))?;
                    if groups.last().unwrap().tris.is_empty()
                        && groups.last().unwrap().edges.is_empty()
                    {
                        groups.last_mut().unwrap().name = n;
                    } else {
                        if groups.len() >= 128 {
                            return Err("OBJ object limit128".into());
                        }
                        groups.push(Group {
                            name: n,
                            ..Default::default()
                        });
                    }
                }
                "f" => {
                    if !(3..=256).contains(&args.len()) {
                        return Err("Face corner limit3..256".into());
                    }
                    let corners = args
                        .iter()
                        .map(|s| corner(s, vertices.len(), tex.len(), normals.len()))
                        .collect::<Result<Vec<_>, _>>()?;
                    let v = corners.iter().map(|c| c.v).collect::<Vec<_>>();
                    let tris = mesh::triangulate(&vertices, &v, cancel)?;
                    total_triangles += tris.len();
                    if total_triangles > 131072 {
                        return Err("OBJ triangle limit131072".into());
                    }
                    let g = groups.last_mut().unwrap();
                    for i in 0..v.len() {
                        edge(g, v[i], v[(i + 1) % v.len()]);
                    }
                    for t in tris {
                        g.tris.push((t.map(|i| corners[i]), material, smooth));
                    }
                }
                "l" => {
                    if !(2..=256).contains(&args.len()) {
                        return Err("Polyline corner limit2..256".into());
                    }
                    let c = args
                        .iter()
                        .map(|s| corner(s, vertices.len(), tex.len(), 0))
                        .collect::<Result<Vec<_>, _>>()?;
                    for pair in c.windows(2) {
                        edge(groups.last_mut().unwrap(), pair[0].v, pair[1].v);
                    }
                    if c.iter().any(|c| c.t.is_some()) {
                        asset.warning(
                            "Line texture coordinates are not used by wireframe output".into(),
                        );
                    }
                }
                "s" => {
                    if args.len() != 1 {
                        return Err("Invalid smoothing group".into());
                    }
                    smooth = if matches!(args[0], "off" | "0") {
                        0
                    } else if args[0] == "on" {
                        1
                    } else {
                        args[0]
                            .parse::<u16>()
                            .map_err(|_| "Smoothing group must be0..65535")?
                    };
                }
                "mtllib" => {
                    for p in args {
                        let p = normal_path(p)?;
                        if !p.to_ascii_lowercase().ends_with(".mtl") {
                            return Err(
                                "Material libraries must be relative local .mtl files".into()
                            );
                        }
                        if !libraries.contains(&p) {
                            libraries.push(p);
                        }
                    }
                    if libraries.len() > 128 {
                        return Err("Material library count limit".into());
                    }
                }
                "usemtl" => {
                    let n = name(&args.join(" "))?;
                    material = if let Some(i) = materials
                        .iter()
                        .enumerate()
                        .skip(1)
                        .find_map(|(i, m)| (m.name == n).then_some(i))
                    {
                        i as u16
                    } else {
                        if materials.len() >= 256 {
                            return Err("Material count limit256".into());
                        }
                        materials.push(Material {
                            name: n,
                            ..Default::default()
                        });
                        (materials.len() - 1) as u16
                    };
                }
                "g" => asset.warning("OBJ grouping names are merged within each object".into()),
                "call" | "csh" | "shadow_obj" | "trace_obj" => {
                    return Err(
                        "Executable commands and recursive object references are not accepted"
                            .into(),
                    );
                }
                key => asset.warning(format!(
                    "Unsupported OBJ statement: {}",
                    key.chars().take(64).collect::<String>()
                )),
            }
            if vertices.len() > 65536 || tex.len() > 65536 || normals.len() > 65536 {
                return Err("OBJ vertex / attribute count limit65536".into());
            }
            if groups.last().unwrap().edges.len() > 65536 {
                return Err("OBJ wire edge limit65536 per object".into());
            }
            Ok::<_, String>(())
        })();
        result.map_err(|e| format!("OBJ line{}: {e}", line + 1))?;
    }
    for g in groups {
        if g.edges.is_empty() {
            continue;
        }
        let mut used = BTreeSet::new();
        for e in &g.edges {
            used.extend(*e);
        }
        for (t, _, _) in &g.tris {
            used.extend(t.map(|c| c.v));
        }
        let map = used
            .iter()
            .enumerate()
            .map(|(n, i)| (*i, n as u32))
            .collect::<BTreeMap<_, _>>();
        let used_t = g
            .tris
            .iter()
            .flat_map(|(t, _, _)| t.iter().filter_map(|c| c.t))
            .collect::<BTreeSet<_>>();
        let tm = used_t
            .iter()
            .enumerate()
            .map(|(n, i)| (*i, n as u32))
            .collect::<BTreeMap<_, _>>();
        let used_n = g
            .tris
            .iter()
            .flat_map(|(t, _, _)| t.iter().filter_map(|c| c.n))
            .collect::<BTreeSet<_>>();
        let nm = used_n
            .iter()
            .enumerate()
            .map(|(n, i)| (*i, n as u32))
            .collect::<BTreeMap<_, _>>();
        let used_m = g
            .tris
            .iter()
            .map(|(_, m, _)| *m)
            .chain([0])
            .collect::<BTreeSet<_>>();
        let mm = used_m
            .iter()
            .enumerate()
            .map(|(n, i)| (*i, n as u16))
            .collect::<BTreeMap<_, _>>();
        let triangles = g
            .tris
            .iter()
            .map(|(t, m, s)| {
                let uv = t.map(|c| c.t.map(|i| tm[&i]));
                let normal = t.map(|c| c.n.map(|i| nm[&i]));
                if uv.iter().any(Option::is_some) && uv.iter().any(Option::is_none)
                    || normal.iter().any(Option::is_some) && normal.iter().any(Option::is_none)
                {
                    return Err("A triangle must have all or no UVs / normals".into());
                }
                Ok(Triangle {
                    v: t.map(|c| map[&c.v]),
                    t: uv[0].map(|_| uv.map(Option::unwrap)),
                    n: normal[0].map(|_| normal.map(Option::unwrap)),
                    material: mm[m],
                    smoothing: *s,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        asset.models.push(Model {
            name: g.name,
            mesh: WorldMesh {
                vertices: used.iter().map(|i| vertices[*i]).collect(),
                edges: g
                    .edges
                    .into_iter()
                    .map(|e| e.map(|i| map[&i] as usize))
                    .collect(),
                color: [180, 210, 245, 255],
            },
            surface: Surface {
                triangles,
                texcoords: used_t.iter().map(|i| tex[*i]).collect(),
                normals: used_n.iter().map(|i| normals[*i]).collect(),
                materials: used_m
                    .iter()
                    .map(|i| materials[*i as usize].clone())
                    .collect(),
                references: vec![],
                source: name(source)?,
                paint: None,
                imported: None,
            },
        });
    }
    if !asset.valid() {
        return Err("OBJ has no usable polygon / line geometry or exceeds model limits".into());
    }
    Ok(Parsed { asset, libraries })
}
fn normal_path(s: &str) -> Result<String, String> {
    let s = s.replace('\\', "/");
    if s.starts_with('/') || s.contains(':') || s.chars().any(char::is_control) || s.len() > 1024 {
        return Err("Use a relative local asset path".into());
    }
    let mut out = vec![];
    for part in s.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.pop().is_none() {
                    return Err("Asset path escapes model folder".into());
                }
            }
            p => out.push(p),
        }
    }
    let out = out.join("/");
    if !mesh::relative(&out) {
        return Err("Invalid relative asset path".into());
    }
    Ok(out)
}
fn resolve(root: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    if !mesh::relative(name) {
        return Err("External assets must stay inside the local model folder".into());
    }
    let p = root.join(name);
    let parent = match p.parent().unwrap().canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !parent.starts_with(root) || !crate::font_library::local(&parent) {
        return Err("Asset path escapes model folder".into());
    }
    match p.canonicalize() {
        Ok(p) => {
            if !p.starts_with(root) || !crate::font_library::local(&p) || !p.is_file() {
                Err("Asset reference is not a local regular model-folder file".into())
            } else {
                Ok(Some(p))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
pub fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Asset, String> {
    crate::book_io::check_cancel(cancel)?;
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("obj"))
    {
        return Err("Choose a local polygon .obj model".into());
    }
    if path.is_absolute() && !crate::font_library::local(path) {
        return Err("Choose a local drive model".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let root = path.parent().ok_or("Model folder missing")?;
    let data = crate::book_io::read_local(&path, MAX_FILE.min(budget / 32))?;
    let Parsed {
        mut asset,
        libraries,
    } = decode(
        &data,
        path.file_name()
            .unwrap()
            .to_str()
            .ok_or("Use a UTF-8 model filename")?,
        cancel,
    )?;
    let mut definitions = BTreeMap::new();
    let mut refs = vec![];
    for lib in libraries {
        crate::book_io::check_cancel(cancel)?;
        let p = resolve(root, &lib)?;
        refs.push(Reference {
            name: lib.clone(),
            present: p.is_some(),
            texture: false,
        });
        let Some(p) = p else {
            asset.warning(format!("Missing material library: {lib}"));
            continue;
        };
        let bytes = crate::book_io::read_local(&p, (1024 * 1024).min(budget / 32))?;
        let mut current: Option<Material> = None;
        for row in logical(&bytes)? {
            crate::book_io::check_cancel(cancel)?;
            let mut words = row.split_whitespace();
            let Some(key) = words.next() else {
                continue;
            };
            let args = words.collect::<Vec<_>>();
            if key == "newmtl" {
                if let Some(m) = current.take()
                    && definitions.insert(m.name.clone(), m).is_some()
                {
                    return Err("Duplicate material definition".into());
                }
                current = Some(Material {
                    name: name(&args.join(" "))?,
                    ..Default::default()
                });
                continue;
            }
            let Some(m) = current.as_mut() else {
                return Err("Material property precedes newmtl".into());
            };
            match key {
                "Kd" => {
                    if args.len() != 3 {
                        return Err("Use RGB diffuse material values".into());
                    }
                    for (i, n) in args.iter().enumerate() {
                        let v = float(n)?;
                        if !(0. ..=1.).contains(&v) {
                            return Err("Diffuse material color must be0..1".into());
                        }
                        m.rgba[i] = v as f32;
                    }
                }
                "d" | "Tr" => {
                    if args.len() != 1 {
                        return Err("Use a single material dissolve value".into());
                    }
                    let v = float(args[0])?;
                    if !(0. ..=1.).contains(&v) {
                        return Err("Material dissolve must be0..1".into());
                    }
                    m.rgba[3] = if key == "Tr" { 1. - v as f32 } else { v as f32 };
                }
                "map_Kd" => {
                    let texture = args.join(" ").replace('\\', "/");
                    if texture.starts_with('-') {
                        asset.warning(
                            "Texture map options are not applied; the diffuse factor is retained"
                                .into(),
                        );
                        continue;
                    }
                    if texture.is_empty() || texture.starts_with('/') || texture.contains(':') {
                        return Err("Texture must be a relative local model-folder file".into());
                    }
                    let material_parent = p
                        .parent()
                        .unwrap()
                        .strip_prefix(root)
                        .map_err(|_| "Material escaped model folder")?
                        .to_string_lossy()
                        .replace('\\', "/");
                    let relative = normal_path(&if material_parent.is_empty() {
                        texture.clone()
                    } else {
                        format!("{material_parent}/{texture}")
                    })?;
                    let present = resolve(root, &relative)?.is_some();
                    if refs.len() >= 128 {
                        return Err("External asset reference limit128".into());
                    }
                    refs.push(Reference {
                        name: relative.clone(),
                        present,
                        texture: true,
                    });
                    if !present {
                        asset.warning(format!("Missing texture: {relative}"));
                    }
                    asset.warning(
                        "Texture references are retained; current output uses wireframe color"
                            .into(),
                    );
                    m.texture = Some(relative);
                }
                "call" | "csh" => {
                    return Err("Executable material statements are not accepted".into());
                }
                other => asset.warning(format!(
                    "Material property not applied: {}",
                    other.chars().take(64).collect::<String>()
                )),
            }
        }
        if let Some(m) = current
            && definitions.insert(m.name.clone(), m).is_some()
        {
            return Err("Duplicate material definition".into());
        }
        if definitions.len() > 256 {
            return Err("Material definition limit256".into());
        }
    }
    let references_bytes = refs
        .iter()
        .map(|r| r.name.len() + std::mem::size_of::<Reference>())
        .sum::<usize>();
    let models_bytes = asset
        .models
        .iter()
        .map(|m| m.mesh.vertices.len() * 24 + m.mesh.edges.len() * 16 + m.surface.bytes())
        .sum::<usize>();
    if models_bytes
        .saturating_add(references_bytes.saturating_mul(asset.models.len()))
        .saturating_mul(4)
        > budget
    {
        return Err("Model attributes / references exceed remaining working memory".into());
    }
    for model in &mut asset.models {
        for m in model.surface.materials.iter_mut().skip(1) {
            if let Some(found) = definitions.get(&m.name) {
                *m = found.clone();
            }
        }
        model.surface.references = refs.clone();
    }
    let missing = asset
        .models
        .iter()
        .flat_map(|m| m.surface.materials.iter().skip(1))
        .filter(|m| !definitions.contains_key(&m.name))
        .map(|m| m.name.clone())
        .collect::<BTreeSet<_>>();
    for m in missing {
        asset.warning(format!("Undefined material: {m}"));
    }
    for model in &mut asset.models {
        let material = model
            .surface
            .triangles
            .first()
            .map_or(0, |t| t.material as usize);
        model.mesh.color = model.surface.materials[material]
            .rgba
            .map(|n| (n * 255.).round() as u8);
    }
    crate::book_io::check_cancel(cancel)?;
    if !asset.valid() {
        return Err("Imported model metadata exceeds limits".into());
    }
    Ok(asset)
}
