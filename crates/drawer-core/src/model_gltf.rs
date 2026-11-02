//! Explicit glTF2/GLB subset: triangle geometry, diffuse maps and frozen LBS rigs.
use crate::{
    model_buffers::{self as b, Resources},
    model_import::{Alpha, Attributes, Material, Skin, Wrap},
    model_mesh::{Asset, Model, Surface, Triangle},
    scene::WorldMesh,
    world_math::{Matrix, V, unit},
};
use serde_json::Value as J;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
};

fn unknown(v: &J, keys: &[&str], where_: &str, a: &mut Asset) -> Result<(), String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("Expected glTF object: {where_}"))?;
    for k in o.keys() {
        if !keys.contains(&k.as_str()) && !matches!(k.as_str(), "name" | "extensions" | "extras") {
            a.warning(format!("Unsupported property {where_}.{k}"));
        }
    }
    Ok(())
}
fn scan(v: &J, path: &str, a: &mut Asset) -> Result<(), String> {
    match v {
        J::Object(o) => {
            for (k, v) in o {
                if k == "extensions" {
                    for name in v.as_object().ok_or("extensions must be an object")?.keys() {
                        a.warning(format!("Unsupported extension {path}.{name}"));
                    }
                }
                if k == "extras" {
                    a.warning(format!("Application extras at {path} are not retained"));
                }
                if k == "uri" {
                    let uri = v.as_str().ok_or("URI must be text")?;
                    if !uri.starts_with("data:") {
                        b::label(
                            &serde_json::json!({"name":crate::model_import::uri_name(uri)?}),
                            "reference",
                        )?;
                    }
                }
                scan(v, &format!("{path}.{k}"), a)?;
            }
        }
        J::Array(v) => {
            for (i, v) in v.iter().enumerate() {
                scan(v, &format!("{path}[{i}]"), a)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn vector<const N: usize>(v: Option<&J>, default: [f64; N]) -> Result<[f64; N], String> {
    let Some(v) = v else { return Ok(default) };
    let v = v
        .as_array()
        .filter(|v| v.len() == N)
        .ok_or("Invalid transform/color vector length")?;
    let mut out = [0.; N];
    for (i, v) in v.iter().enumerate() {
        out[i] = v
            .as_f64()
            .filter(|v| v.is_finite() && v.abs() <= 1e9)
            .ok_or("Invalid transform/color value")?;
    }
    Ok(out)
}
fn matrix(v: &[f64]) -> Result<Matrix, String> {
    if v.len() != 16
        || v[3].abs() > 1e-6
        || v[7].abs() > 1e-6
        || v[11].abs() > 1e-6
        || (v[15] - 1.).abs() > 1e-6
    {
        return Err("Expected an affine column-major MAT4".into());
    }
    let m = Matrix(std::array::from_fn(|r| {
        std::array::from_fn(|c| v[c * 4 + r])
    }));
    if !m.valid() {
        return Err("Singular/excessive model matrix".into());
    }
    Ok(m)
}
fn transform(v: &J) -> Result<Matrix, String> {
    if let Some(m) = v.get("matrix") {
        if ["translation", "rotation", "scale"]
            .iter()
            .any(|k| v.get(k).is_some())
        {
            return Err("A glTF node cannot combine matrix and TRS".into());
        }
        return matrix(&vector(Some(m), [0.; 16])?);
    }
    let t = vector(v.get("translation"), [0.; 3])?;
    let s = vector(v.get("scale"), [1.; 3])?;
    let q = vector(v.get("rotation"), [0., 0., 0., 1.])?;
    if (q.iter().map(|n| n * n).sum::<f64>() - 1.).abs() > 1e-4 {
        return Err("Node quaternion must be normalized".into());
    }
    let [x, y, z, w] = q;
    let rotation = Matrix([
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            0.,
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
            0.,
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
            0.,
        ],
    ]);
    let m = Matrix::scale(s).then(rotation).then(Matrix::translation(t));
    if !m.valid() {
        return Err("Invalid/singular node TRS".into());
    }
    Ok(m)
}
fn worlds(json: &J, a: &mut Asset) -> Result<(Vec<Matrix>, Vec<Option<usize>>), String> {
    let nodes = b::array(json, "nodes")?;
    let mut parents = vec![None; nodes.len()];
    let mut local = vec![];
    for (i, n) in nodes.iter().enumerate() {
        unknown(
            n,
            &[
                "children",
                "mesh",
                "skin",
                "matrix",
                "translation",
                "rotation",
                "scale",
                "camera",
                "weights",
            ],
            &format!("nodes[{i}]"),
            a,
        )?;
        if n.get("camera").is_some() {
            a.warning(format!(
                "Node {i}: source camera is not imported; document camera retained"
            ));
        }
        if n.get("weights").is_some() {
            a.warning(format!("Node {i}: morph weights are not applied"));
        }
        local.push(transform(n)?);
        for c in b::array(n, "children")? {
            let c = b::number(c)?;
            if c >= nodes.len() || parents[c].replace(i).is_some() {
                return Err("Node child is out of range or has multiple parents".into());
            }
        }
    }
    let mut out = vec![Matrix::default(); nodes.len()];
    for (i, m) in out.iter_mut().enumerate() {
        let mut p = Some(i);
        let mut count = 0;
        while let Some(j) = p {
            if count >= nodes.len() {
                return Err("Cycle in glTF node hierarchy".into());
            }
            *m = m.then(local[j]);
            p = parents[j];
            count += 1;
        }
        if !m.valid() {
            return Err("Accumulated node matrix outside limits".into());
        }
    }
    Ok((out, parents))
}
fn wrap(v: usize) -> Result<Wrap, String> {
    match v {
        10497 => Ok(Wrap::Repeat),
        33071 => Ok(Wrap::Clamp),
        33648 => Ok(Wrap::Mirror),
        _ => Err("Invalid texture wrapping mode".into()),
    }
}
fn srgb(x: f64) -> f32 {
    (if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1. / 2.4) - 0.055
    }) as f32
}
fn material(
    r: &mut Resources<'_>,
    i: Option<usize>,
    a: &mut Asset,
) -> Result<(crate::model_mesh::Material, Material), String> {
    let white = crate::model_mesh::Material {
        name: "glTF default".into(),
        rgba: [1.; 4],
        texture: None,
    };
    let mut m = Material {
        image: None,
        wrap: [Wrap::Repeat; 2],
        flip_v: true,
        alpha: Alpha::Opaque,
        cutoff: 0.5,
    };
    let Some(i) = i else { return Ok((white, m)) };
    let v = b::at(r.json, "materials", i)?;
    unknown(
        v,
        &[
            "pbrMetallicRoughness",
            "alphaMode",
            "alphaCutoff",
            "doubleSided",
            "normalTexture",
            "occlusionTexture",
            "emissiveTexture",
            "emissiveFactor",
        ],
        &format!("materials[{i}]"),
        a,
    )?;
    for k in [
        "normalTexture",
        "occlusionTexture",
        "emissiveTexture",
        "emissiveFactor",
    ] {
        if v.get(k).is_some() {
            a.warning(format!(
                "Material {i}: {k} is not applied by the diffuse renderer"
            ));
        }
    }
    let p = v
        .get("pbrMetallicRoughness")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    unknown(
        &p,
        &[
            "baseColorFactor",
            "baseColorTexture",
            "metallicFactor",
            "roughnessFactor",
            "metallicRoughnessTexture",
        ],
        &format!("materials[{i}].pbrMetallicRoughness"),
        a,
    )?;
    a.warning("glTF import uses diffuse/unlit or existing document lighting, nearest texture sampling and two-sided faces; metallic/roughness BRDF, sampler filtering, source lights/cameras and animation are not reproduced".into());
    let factor = vector(p.get("baseColorFactor"), [1.; 4])?;
    if factor.iter().any(|v| !(0. ..=1.).contains(v)) {
        return Err("baseColorFactor must be0..1".into());
    }
    m.alpha = match v
        .get("alphaMode")
        .map_or(Ok("OPAQUE"), |v| v.as_str().ok_or("alphaMode must be text"))?
    {
        "OPAQUE" => Alpha::Opaque,
        "MASK" => Alpha::Mask,
        "BLEND" => Alpha::Blend,
        _ => return Err("Invalid alphaMode".into()),
    };
    m.cutoff = v
        .get("alphaCutoff")
        .map_or(Ok(0.5), |v| v.as_f64().ok_or("Invalid alpha cutoff"))?;
    if !m.cutoff.is_finite() || !(0. ..=1.).contains(&m.cutoff) {
        return Err("Alpha cutoff must be0..1".into());
    }
    if let Some(info) = p.get("baseColorTexture") {
        unknown(info, &["index", "texCoord"], "baseColorTexture", a)?;
        if b::optional(info, "texCoord", 0)? != 0 {
            return Err("Base color requires TEXCOORD_0; other UV sets are not imported".into());
        }
        let t = b::at(r.json, "textures", b::field(info, "index")?)?;
        unknown(t, &["source", "sampler"], "texture", a)?;
        if let Some(s) = t.get("sampler") {
            let s = b::at(r.json, "samplers", b::number(s)?)?;
            unknown(
                s,
                &["wrapS", "wrapT", "minFilter", "magFilter"],
                "sampler",
                a,
            )?;
            m.wrap = [
                wrap(b::optional(s, "wrapS", 10497)?)?,
                wrap(b::optional(s, "wrapT", 10497)?)?,
            ];
        }
        m.image = Some(r.image(b::field(t, "source")?)?);
    }
    let mut rgba = factor.map(srgb);
    rgba[3] = if m.alpha == Alpha::Opaque {
        1.
    } else {
        factor[3] as f32
    };
    Ok((
        crate::model_mesh::Material {
            name: b::label(v, &format!("Material {i}"))?,
            rgba,
            texture: None,
        },
        m,
    ))
}
fn normal(m: Matrix, n: V) -> V {
    let inv = m.inverse();
    unit(std::array::from_fn(|i| {
        (0..3).map(|j| inv.0[j][i] * n[j]).sum()
    }))
}
fn skin(
    r: &Resources<'_>,
    id: usize,
    attributes: &J,
    positions: &[V],
    world: &[Matrix],
    parents: &[Option<usize>],
    a: &mut Asset,
) -> Result<Skin, String> {
    let s = b::at(r.json, "skins", id)?;
    unknown(
        s,
        &["joints", "inverseBindMatrices", "skeleton"],
        &format!("skins[{id}]"),
        a,
    )?;
    let ids = b::array(s, "joints")?
        .iter()
        .map(b::number)
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty()
        || ids.len() > 128
        || ids.iter().any(|i| *i >= world.len())
        || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
    {
        return Err("Skin requires1..128 unique existing joint nodes".into());
    }
    if let Some(s) = s.get("skeleton") {
        let root = b::number(s)?;
        if root >= world.len() {
            return Err("Skin skeleton node is out of range".into());
        }
        for id in &ids {
            let mut p = Some(*id);
            while p.is_some() && p != Some(root) {
                p = parents[p.unwrap()];
            }
            if p.is_none() {
                return Err("Skin skeleton is not an ancestor of every joint".into());
            }
        }
    }
    let bind = if let Some(i) = s.get("inverseBindMatrices") {
        let data = r.accessor(b::number(i)?, "MAT4", &[5126], 128)?;
        if data.len() < ids.len() {
            return Err("Too few inverse bind matrices".into());
        }
        data.iter()
            .take(ids.len())
            .map(|v| matrix(v))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![Matrix::default(); ids.len()]
    };
    let joints = r.accessor(
        b::field(attributes, "JOINTS_0")?,
        "VEC4",
        &[5121, 5123],
        65536,
    )?;
    let weights_id = b::field(attributes, "WEIGHTS_0")?;
    let w = b::at(r.json, "accessors", weights_id)?;
    if b::field(w, "componentType")? != 5126
        && w.get("normalized").and_then(J::as_bool) != Some(true)
    {
        return Err("Integer skin weights must be normalized".into());
    }
    if b::at(r.json, "accessors", b::field(attributes, "JOINTS_0")?)?
        .get("normalized")
        .and_then(J::as_bool)
        == Some(true)
    {
        return Err("Skin joint indices cannot be normalized".into());
    }
    let weights = r.accessor(weights_id, "VEC4", &[5121, 5123, 5126], 65536)?;
    if joints.len() != positions.len() || weights.len() != positions.len() {
        return Err("Skin attributes must match POSITION count".into());
    }
    let mut ji = vec![];
    let mut we = vec![];
    for (j, w) in joints.iter().zip(weights) {
        if j.iter()
            .any(|n| n.fract() != 0. || *n < 0. || *n >= ids.len() as f64)
            || w.iter().any(|n| !(0. ..=1.).contains(n))
        {
            return Err("Invalid skin joint/weight".into());
        }
        let sum = w.iter().sum::<f64>();
        if sum <= 1e-12 {
            return Err("Skin vertex has zero total weight".into());
        }
        ji.push(std::array::from_fn(|i| j[i] as u16));
        we.push(std::array::from_fn(|i| w[i] / sum));
    }
    let mut joint_parents = vec![];
    let mut names = vec![];
    for id in &ids {
        let mut p = parents[*id];
        while p.is_some_and(|p| !ids.contains(&p)) {
            p = parents[p.unwrap()];
        }
        joint_parents.push(
            p.and_then(|p| ids.iter().position(|i| *i == p))
                .map(|p| p as u16),
        );
        names.push(b::label(
            b::at(r.json, "nodes", *id)?,
            &format!("Joint {id}"),
        )?);
    }
    a.warning("Skin JOINTS_0/WEIGHTS_0, joint ancestry, source positions and inverse-bind/rest matrices are retained; the default linear-blend pose is baked into editable geometry. Imported rigs do not use the built-in mannequin pose controls; extra influences, animations and morph targets are not retained".into());
    Ok(Skin {
        names,
        parents: joint_parents,
        world: ids.iter().map(|i| world[*i]).collect(),
        inverse_bind: bind,
        positions: positions.to_vec(),
        joints: ji,
        weights: we,
    })
}
fn primitive(
    r: &mut Resources<'_>,
    node: &J,
    primitive: &J,
    hierarchy: (&[Matrix], &[Option<usize>]),
    at: (usize, usize),
    a: &mut Asset,
    cancel: &AtomicBool,
) -> Result<Model, String> {
    let (world, parents) = hierarchy;
    let (node_id, index) = at;
    unknown(
        primitive,
        &["attributes", "indices", "material", "mode", "targets"],
        "primitive",
        a,
    )?;
    if primitive.get("targets").is_some() {
        a.warning(format!(
            "Node {node_id} primitive {index}: morph targets are not applied/retained"
        ));
    }
    let attributes = primitive
        .get("attributes")
        .and_then(J::as_object)
        .ok_or("Primitive attributes missing")?;
    for key in attributes.keys() {
        if !matches!(
            key.as_str(),
            "POSITION" | "NORMAL" | "TEXCOORD_0" | "JOINTS_0" | "WEIGHTS_0"
        ) {
            a.warning(format!(
                "Node {node_id} primitive {index}: unsupported attribute {key}"
            ));
        }
    }
    let attr = primitive.get("attributes").unwrap();
    let mut vertices = r
        .accessor(b::field(attr, "POSITION")?, "VEC3", &[5126], 65536)?
        .into_iter()
        .map(|v| v.try_into().unwrap())
        .collect::<Vec<V>>();
    let mut normals = if let Some(i) = attr.get("NORMAL") {
        r.accessor(b::number(i)?, "VEC3", &[5126], 65536)?
            .into_iter()
            .map(|v| v.try_into().unwrap())
            .collect::<Vec<V>>()
    } else {
        vec![]
    };
    if !normals.is_empty()
        && (normals.len() != vertices.len()
            || normals
                .iter()
                .any(|n| crate::world_math::dot(*n, *n) < 1e-20))
    {
        return Err("Normals must be nonzero and match vertex count".into());
    }
    let texcoords = if let Some(i) = attr.get("TEXCOORD_0") {
        let id = b::number(i)?;
        let v = b::at(r.json, "accessors", id)?;
        if b::field(v, "componentType")? != 5126
            && v.get("normalized").and_then(J::as_bool) != Some(true)
        {
            return Err("Integer texture coordinates must be normalized".into());
        }
        r.accessor(id, "VEC2", &[5121, 5123, 5126], 65536)?
            .into_iter()
            .map(|v| [v[0], 1. - v[1], 0.])
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    if !texcoords.is_empty() && texcoords.len() != vertices.len() {
        return Err("Texture coordinate count differs from POSITION".into());
    }
    let imported_skin = if let Some(id) = node.get("skin") {
        Some(skin(r, b::number(id)?, attr, &vertices, world, parents, a)?)
    } else {
        if attr.get("JOINTS_0").is_some() || attr.get("WEIGHTS_0").is_some() {
            a.warning(format!(
                "Node {node_id}: joint/weight attributes without a skin are not applied"
            ));
        }
        None
    };
    for (i, v) in vertices.iter_mut().enumerate() {
        if i % 256 == 0 {
            crate::book_io::check_cancel(cancel)?;
        }
        if let Some(s) = &imported_skin {
            let matrices =
                s.joints[i].map(|j| s.inverse_bind[j as usize].then(s.world[j as usize]));
            let blended = Matrix(std::array::from_fn(|r| {
                std::array::from_fn(|c| (0..4).map(|j| matrices[j].0[r][c] * s.weights[i][j]).sum())
            }));
            if !blended.valid() {
                return Err("Skin produced a singular blended vertex transform".into());
            }
            *v = blended.map(*v);
            if !normals.is_empty() {
                normals[i] = normal(blended, normals[i]);
            }
        } else {
            *v = world[node_id].map(*v);
            if !normals.is_empty() {
                normals[i] = normal(world[node_id], normals[i]);
            }
        }
    }
    let indices = if let Some(i) = primitive.get("indices") {
        let id = b::number(i)?;
        if b::at(r.json, "accessors", id)?
            .get("normalized")
            .and_then(J::as_bool)
            == Some(true)
        {
            return Err("Indices cannot be normalized".into());
        }
        r.accessor(id, "SCALAR", &[5121, 5123, 5125], 393216)?
            .into_iter()
            .map(|v| v[0] as usize)
            .collect::<Vec<_>>()
    } else {
        (0..vertices.len()).collect()
    };
    if indices.iter().any(|i| *i >= vertices.len()) {
        return Err("Triangle index exceeds POSITION count".into());
    }
    let mode = b::optional(primitive, "mode", 4)?;
    let mut faces = vec![];
    match mode {
        4 => {
            if indices.len() % 3 != 0 {
                return Err("TRIANGLES index count must be divisible by3".into());
            }
            for f in indices.as_chunks::<3>().0 {
                faces.push([f[0], f[1], f[2]]);
            }
        }
        5 | 6 => {
            if indices.len() < 3 {
                return Err("Triangle strip/fan requires3 indices".into());
            }
            for i in 2..indices.len() {
                faces.push(if mode == 6 {
                    [indices[0], indices[i - 1], indices[i]]
                } else if i % 2 == 0 {
                    [indices[i - 2], indices[i - 1], indices[i]]
                } else {
                    [indices[i - 1], indices[i - 2], indices[i]]
                });
            }
        }
        _ => {
            return Err(format!(
                "Unsupported primitive mode {mode}; convert points/lines to triangle meshes"
            ));
        }
    }
    let (base, material) = material(r, primitive.get("material").map(b::number).transpose()?, a)?;
    if material.image.is_some() && texcoords.is_empty() {
        return Err("Textured primitive has no TEXCOORD_0".into());
    }
    let mut triangles = vec![];
    let mut edges = BTreeSet::new();
    let mut degenerate = 0;
    for mut f in faces {
        if f.iter().collect::<BTreeSet<_>>().len() != 3 {
            degenerate += 1;
            continue;
        }
        let cross = crate::world_math::cross(
            crate::world_math::sub(vertices[f[1]], vertices[f[0]]),
            crate::world_math::sub(vertices[f[2]], vertices[f[0]]),
        );
        if crate::world_math::dot(cross, cross) < 1e-24 {
            degenerate += 1;
            continue;
        }
        if imported_skin.is_none() && world[node_id].determinant() < 0. {
            f.swap(1, 2);
        }
        for i in 0..3 {
            let (a, b) = (f[i], f[(i + 1) % 3]);
            edges.insert([a.min(b), a.max(b)]);
        }
        let v = f.map(|i| i as u32);
        triangles.push(Triangle {
            v,
            t: (!texcoords.is_empty()).then_some(v),
            n: (!normals.is_empty()).then_some(v),
            material: 0,
            smoothing: 0,
        });
    }
    if degenerate > 0 {
        a.warning(format!(
            "Node {node_id} primitive {index}: skipped {degenerate} degenerate triangles"
        ));
    }
    let node_name = b::label(node, &format!("Node {node_id}"))?;
    Ok(Model {
        name: format!("{node_name} / primitive {index}"),
        mesh: WorldMesh {
            vertices,
            edges: edges.into_iter().collect(),
            color: base.rgba.map(|v| (v * 255.).round() as u8),
        },
        surface: Surface {
            triangles,
            texcoords,
            normals,
            materials: vec![base],
            references: vec![],
            source: "glTF2 frozen local import (+Y up, metres)".into(),
            paint: None,
            imported: Some(Attributes {
                materials: BTreeMap::from([(0, material)]),
                skin: imported_skin,
            }),
        },
    })
}
fn container<'a>(
    bytes: &'a [u8],
    glb: bool,
    a: &mut Asset,
) -> Result<(J, Option<&'a [u8]>), String> {
    if !glb {
        return Ok((
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?,
            None,
        ));
    }
    let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    if bytes.len() < 20 || &bytes[..4] != b"glTF" || word(4) != 2 || word(8) != bytes.len() {
        return Err("Invalid GLB2 header/length".into());
    }
    let mut offset = 12;
    let mut json = None;
    let mut bin = None;
    while offset < bytes.len() {
        if offset + 8 > bytes.len() {
            return Err("Truncated GLB chunk header".into());
        }
        let length = word(offset);
        let kind = word(offset + 4);
        offset += 8;
        let end = offset.checked_add(length).ok_or("GLB chunk overflow")?;
        if length % 4 != 0 || end > bytes.len() {
            return Err("Invalid GLB chunk bounds/alignment".into());
        }
        let chunk = &bytes[offset..end];
        if json.is_none() {
            if kind != 0x4e4f534a {
                return Err("GLB JSON must be its first chunk".into());
            }
            json = Some(serde_json::from_slice(chunk).map_err(|e| e.to_string())?);
        } else if kind == 0x004e4942 {
            if bin.replace(chunk).is_some() {
                return Err("Duplicate GLB BIN chunk".into());
            }
        } else if kind == 0x4e4f534a {
            return Err("Duplicate GLB JSON chunk".into());
        } else {
            a.warning(format!("Unsupported GLB chunk type {kind}"));
        }
        offset = end;
    }
    Ok((json.ok_or("Missing GLB JSON")?, bin))
}
pub fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Asset, String> {
    let bytes = crate::book_io::read_local(path, (16 * 1024 * 1024).min(budget / 8))?;
    let mut a = Asset::default();
    let (json, bin) = container(
        &bytes,
        path.extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("glb")),
        &mut a,
    )?;
    unknown(
        &json,
        &[
            "asset",
            "scene",
            "scenes",
            "nodes",
            "meshes",
            "accessors",
            "buffers",
            "bufferViews",
            "materials",
            "textures",
            "images",
            "samplers",
            "skins",
            "animations",
            "cameras",
            "extensionsUsed",
            "extensionsRequired",
        ],
        "root",
        &mut a,
    )?;
    let asset = json.get("asset").ok_or("Missing glTF asset version")?;
    unknown(
        asset,
        &["version", "minVersion", "generator", "copyright"],
        "asset",
        &mut a,
    )?;
    if asset.get("version").and_then(J::as_str) != Some("2.0")
        || asset
            .get("minVersion")
            .is_some_and(|v| v.as_str() != Some("2.0"))
    {
        return Err("Only glTF2.0 assets are supported".into());
    }
    let required = b::array(&json, "extensionsRequired")?;
    if !required.is_empty() {
        return Err(format!(
            "Required extensions are unsupported: {}",
            required
                .iter()
                .map(|v| v.as_str().unwrap_or("invalid extension name"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    scan(&json, "glTF", &mut a)?;
    for e in b::array(&json, "extensionsUsed")? {
        a.warning(format!(
            "Unsupported declared extension: {}",
            e.as_str().ok_or("Invalid extension name")?
        ));
    }
    for (k, n) in [
        ("nodes", 512),
        ("meshes", 128),
        ("accessors", 512),
        ("bufferViews", 512),
        ("buffers", 128),
        ("materials", 256),
        ("images", 128),
        ("textures", 256),
        ("skins", 32),
        ("scenes", 32),
        ("samplers", 256),
    ] {
        if b::array(&json, k)?.len() > n {
            return Err(format!("glTF {k} count exceeds{n}"));
        }
    }
    for (key, allowed) in [
        ("buffers", &["uri", "byteLength"][..]),
        (
            "bufferViews",
            &["buffer", "byteOffset", "byteLength", "byteStride", "target"][..],
        ),
        (
            "accessors",
            &[
                "bufferView",
                "byteOffset",
                "componentType",
                "normalized",
                "count",
                "type",
                "min",
                "max",
                "sparse",
            ][..],
        ),
        ("images", &["uri", "bufferView", "mimeType"][..]),
        ("textures", &["source", "sampler"][..]),
        (
            "samplers",
            &["wrapS", "wrapT", "minFilter", "magFilter"][..],
        ),
        ("skins", &["joints", "inverseBindMatrices", "skeleton"][..]),
        ("scenes", &["nodes"][..]),
        ("meshes", &["primitives", "weights"][..]),
        (
            "materials",
            &[
                "pbrMetallicRoughness",
                "alphaMode",
                "alphaCutoff",
                "doubleSided",
                "normalTexture",
                "occlusionTexture",
                "emissiveTexture",
                "emissiveFactor",
            ][..],
        ),
    ] {
        for (i, v) in b::array(&json, key)?.iter().enumerate() {
            unknown(v, allowed, &format!("{key}[{i}]"), &mut a)?;
        }
    }
    if !b::array(&json, "animations")?.is_empty() {
        a.warning(
            "Source animation channels are not imported; only the default node/skin pose is frozen"
                .into(),
        );
    }
    let (world, parents) = worlds(&json, &mut a)?;
    let scenes = b::array(&json, "scenes")?;
    let roots = if scenes.is_empty() {
        a.warning("No glTF scenes: importing parentless nodes as a local model library".into());
        parents
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_none().then_some(i))
            .collect::<Vec<_>>()
    } else {
        let index = b::optional(&json, "scene", 0)?;
        let s = scenes
            .get(index)
            .ok_or("Default scene index out of range")?;
        unknown(s, &["nodes"], "scene", &mut a)?;
        if scenes.len() > 1 {
            a.warning(format!("Only scene {index} of{} is imported", scenes.len()));
        }
        b::array(s, "nodes")?
            .iter()
            .map(b::number)
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut selected = BTreeSet::new();
    let mut stack = roots.clone();
    if roots.iter().collect::<BTreeSet<_>>().len() != roots.len() {
        return Err("Duplicate scene root".into());
    }
    for i in &roots {
        if *i >= parents.len() || parents[*i].is_some() {
            return Err("Scene root is not a parentless existing node".into());
        }
    }
    while let Some(i) = stack.pop() {
        selected.insert(i);
        let node = b::at(&json, "nodes", i)?;
        for child in b::array(node, "children")? {
            stack.push(b::number(child)?);
        }
    }
    let mut resources = Resources::new(
        &json,
        bin,
        path.parent().ok_or("Model folder missing")?,
        budget,
        cancel,
    )?;
    let mut used = 0usize;
    for i in selected {
        crate::book_io::check_cancel(cancel)?;
        let n = b::at(&json, "nodes", i)?;
        let Some(mesh) = n.get("mesh") else { continue };
        let m = b::at(&json, "meshes", b::number(mesh)?)?;
        unknown(m, &["primitives", "weights"], "mesh", &mut a)?;
        for (j, p) in b::array(m, "primitives")?.iter().enumerate() {
            if a.models.len() >= 128 {
                return Err("Imported model primitive limit128".into());
            }
            let model = primitive(
                &mut resources,
                n,
                p,
                (&world, &parents),
                (i, j),
                &mut a,
                cancel,
            )?;
            used = used.saturating_add(
                model.surface.bytes()
                    + model.mesh.vertices.len() * 24
                    + model.mesh.edges.len() * 16,
            );
            if used.saturating_mul(8) > budget {
                return Err("Imported mesh/texture/skin memory limit".into());
            }
            a.models.push(model);
        }
    }
    a.warning("glTF2 source: right-handed +Y up, distances in metres; import rotation and scale convert axes/units. Fit recentres and rescales the asset. External files are frozen into the document; source files stay unchanged".into());
    if !a.valid() {
        return Err("glTF has no usable triangles or exceeds editable model limits".into());
    }
    Ok(a)
}
