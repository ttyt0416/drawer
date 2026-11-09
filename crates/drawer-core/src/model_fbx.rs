//! Local FBX conversion through the pinned ufbx parser. External IO is disabled.
use crate::{
    model_import::{Alpha, Attributes, Image, Material, Skin, Wrap},
    model_mesh::{Asset, Model, Surface, Triangle},
    model_source::Source,
    scene::WorldMesh,
    world_math::{Matrix, V, add, cross, dot, mul, sub, unit},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
fn vector(v: ufbx::Vec3) -> V {
    [v.x, v.y, v.z]
}
fn matrix(m: ufbx::Matrix) -> Matrix {
    Matrix([
        [m.m00, m.m01, m.m02, m.m03],
        [m.m10, m.m11, m.m12, m.m13],
        [m.m20, m.m21, m.m22, m.m23],
    ])
}
fn name(s: &str, fallback: &str) -> Result<String, String> {
    let s = if s.is_empty() { fallback } else { s };
    if s.len() > 1024 || s.chars().any(char::is_control) {
        return Err("Invalid FBX element label".into());
    }
    Ok(s.into())
}
fn normal(m: Matrix, n: V) -> V {
    let inv = m.inverse();
    unit(std::array::from_fn(|i| {
        (0..3).map(|j| inv.0[j][i] * n[j]).sum()
    }))
}
fn dom_value(
    n: &ufbx::DomNode,
    remaining: &mut usize,
    cancel: &AtomicBool,
) -> Result<serde_json::Value, String> {
    crate::book_io::check_cancel(cancel)?;
    let bytes = n
        .name
        .len()
        .saturating_add(
            n.values
                .iter()
                .map(|v| v.value_str.len() + 64)
                .sum::<usize>(),
        )
        .saturating_add(64);
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or("FBX usage metadata exceeds64KiB")?;
    let values = n
        .values
        .iter()
        .map(|v| match v.type_ {
            ufbx::DomValueType::String => Ok(serde_json::Value::String(v.value_str.to_string())),
            ufbx::DomValueType::Number => serde_json::Number::from_f64(v.value_float)
                .map(serde_json::Value::Number)
                .ok_or("Invalid metadata number".into()),
            _ => Err("Unreadable FBX usage metadata (binary/array value)".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let children = n
        .children
        .iter()
        .map(|c| dom_value(c, remaining, cancel))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({"name":n.name.to_string(),"values":values,"children":children}))
}
fn terms(n: &ufbx::DomNode, path: &str, s: &mut Source, cancel: &AtomicBool) -> Result<(), String> {
    crate::book_io::check_cancel(cancel)?;
    let key = n
        .values
        .first()
        .map_or(n.name.as_ref(), |v| {
            if n.name.as_ref() == "P" {
                v.value_str.as_ref()
            } else {
                n.name.as_ref()
            }
        })
        .to_ascii_lowercase();
    let p = format!("{path}/{}", n.name.as_ref());
    if [
        "license",
        "copyright",
        "author",
        "comment",
        "usage",
        "terms",
    ]
    .iter()
    .any(|k| key.contains(k))
    {
        s.usage_present |= ["license", "copyright", "usage", "terms"]
            .iter()
            .any(|k| key.contains(k));
        s.push(p, &dom_value(n, &mut 65536, cancel)?)?;
    } else {
        for c in &n.children {
            terms(c, &p, s, cancel)?;
        }
    }
    Ok(())
}
fn texture(
    t: &ufbx::Texture,
    root: &Path,
    a: &mut Asset,
    s: &mut Surface,
    budget: usize,
) -> Result<Option<Image>, String> {
    if t.type_ != ufbx::TextureType::File {
        return Err(
            "FBX layered/procedural diffuse textures need conversion to a local PNG/JPEG".into(),
        );
    }
    if t.has_uv_transform {
        a.warning(
            "FBX source texture transform is not converted; use per-material UV controls".into(),
        );
    }
    if !t.uv_set.is_empty() && t.uv_set.as_ref() != "default" {
        a.warning(format!(
            "FBX uses first UV set; requested set {} is not selected",
            t.uv_set.as_ref()
        ));
    }
    if !t.content.is_empty() {
        return Image::from_bytes(&t.content).map(Some);
    }
    let raw = if !t.relative_filename.is_empty() {
        t.relative_filename.as_ref()
    } else {
        t.filename.as_ref()
    };
    let name = crate::model_import::uri_name(&raw.replace('\\', "/"))?;
    let p = root.join(&name);
    if !p.exists() {
        s.references.push(crate::model_mesh::Reference {
            name: name.clone(),
            texture: true,
            present: false,
        });
        a.warning(format!("Missing local FBX diffuse texture: {name}"));
        return Ok(None);
    }
    let p = crate::model_import::resolve(root, &name)?;
    s.references.push(crate::model_mesh::Reference {
        name,
        texture: true,
        present: true,
    });
    let bytes = crate::book_io::read_local(&p, (8 * 1024 * 1024).min(budget / 8))?;
    Image::from_bytes(&bytes).map(Some)
}
fn materials(
    node: &ufbx::Node,
    s: &mut Surface,
    attrs: &mut Attributes,
    root: &Path,
    a: &mut Asset,
    budget: usize,
) -> Result<(), String> {
    if node.materials.len() > 256 {
        return Err("FBX material limit256".into());
    }
    if node.materials.is_empty() {
        return Ok(());
    }
    s.materials.clear();
    for (i, m) in node.materials.iter().enumerate() {
        let f = &m.fbx;
        let factor = if f.diffuse_factor.has_value {
            f.diffuse_factor.value_vec4.x
        } else {
            1.
        };
        let color = if f.diffuse_color.has_value {
            vector(ufbx::Vec3 {
                x: f.diffuse_color.value_vec4.x,
                y: f.diffuse_color.value_vec4.y,
                z: f.diffuse_color.value_vec4.z,
            })
        } else {
            [0.7, 0.78, 0.9]
        };
        let transparency = if f.transparency_factor.has_value || f.transparency_color.has_value {
            let factor = if f.transparency_factor.has_value {
                f.transparency_factor.value_vec4.x
            } else {
                1.
            };
            let c = f.transparency_color.value_vec4;
            if f.transparency_color.has_value && (c.x != c.y || c.y != c.z) {
                a.warning("FBX colored transparency converts to luminance alpha".into());
            }
            factor
                * if f.transparency_color.has_value {
                    c.x * 0.2126 + c.y * 0.7152 + c.z * 0.0722
                } else {
                    1.
                }
        } else {
            0.
        };
        let mut rgba = [
            (color[0] * factor) as f32,
            (color[1] * factor) as f32,
            (color[2] * factor) as f32,
            (1. - transparency) as f32,
        ];
        if rgba.iter().any(|v| !v.is_finite()) {
            return Err("Invalid FBX material color/opacity".into());
        }
        if rgba.iter().any(|v| !(0. ..=1.).contains(v)) {
            a.warning("FBX diffuse color/opacity clamped to0..1".into());
            rgba = rgba.map(|v| v.clamp(0., 1.));
        }
        let t = f.diffuse_color.texture.as_ref();
        let image = t
            .map(|t| texture(t, root, a, s, budget))
            .transpose()?
            .flatten();
        let wrap = |w| {
            if w == ufbx::WrapMode::Clamp {
                Wrap::Clamp
            } else {
                Wrap::Repeat
            }
        };
        attrs.materials.insert(
            i as u16,
            Material {
                image,
                wrap: t.map_or([Wrap::Repeat; 2], |t| [wrap(t.wrap_u), wrap(t.wrap_v)]),
                flip_v: true,
                alpha: Alpha::Blend,
                cutoff: 0.5,
            },
        );
        s.materials.push(crate::model_mesh::Material {
            name: name(m.element.name.as_ref(), &format!("Material {i}"))?,
            rgba,
            texture: None,
        });
        if !m.textures.is_empty()
            || !matches!(
                m.shading_model_name.as_ref(),
                "lambert" | "Lambert" | "phong" | "Phong" | ""
            )
        {
            a.warning("FBX materials convert diffuse color/opacity and diffuse PNG/JPEG only; specular, normal/bump, opacity maps, PBR and custom shaders are not converted".into());
        }
    }
    Ok(())
}
fn skin(mesh: &ufbx::Mesh) -> Result<Option<Skin>, String> {
    if mesh.skin_deformers.is_empty() {
        return Ok(None);
    }
    if mesh.skin_deformers.len() != 1 {
        return Err("FBX needs one skin deformer per mesh".into());
    }
    let d = &mesh.skin_deformers[0];
    if !matches!(
        d.skinning_method,
        ufbx::SkinningMethod::Linear | ufbx::SkinningMethod::Rigid
    ) || d.clusters.is_empty()
        || d.clusters.len() > 128
        || d.max_weights_per_vertex > 4
    {
        return Err(
            "FBX skin requires1..128 joints, linear/rigid weights and at most4 influences".into(),
        );
    }
    let mut ids = BTreeMap::new();
    let mut names = vec![];
    let mut world = vec![];
    let mut bind = vec![];
    for (i, c) in d.clusters.iter().enumerate() {
        let b = c.bone_node.as_ref().ok_or("FBX skin cluster has no bone")?;
        if ids.insert(b.element.element_id, i as u16).is_some() {
            return Err("Duplicate FBX skin bone".into());
        }
        names.push(name(b.element.name.as_ref(), &format!("Joint {i}"))?);
        world.push(matrix(b.node_to_world));
        bind.push(matrix(c.geometry_to_bone));
    }
    let mut parents = vec![];
    for c in &d.clusters {
        let mut p = c.bone_node.as_ref().unwrap().parent.as_ref();
        let mut parent = None;
        while let Some(n) = p {
            if let Some(i) = ids.get(&n.element.element_id) {
                parent = Some(*i);
                break;
            }
            p = n.parent.as_ref();
        }
        parents.push(parent);
    }
    let mut joints = vec![];
    let mut weights = vec![];
    let mut positions = vec![];
    for (i, control) in mesh.vertex_indices.iter().enumerate() {
        let v = d
            .vertices
            .get(*control as usize)
            .ok_or("FBX skin vertex missing")?;
        let begin = v.weight_begin as usize;
        let end = begin + v.num_weights as usize;
        let w = d
            .weights
            .get(begin..end)
            .ok_or("Invalid FBX skin weight range")?;
        if w.is_empty() || w.len() > 4 {
            return Err("Unweighted / excessive FBX skin influences".into());
        }
        let sum = w.iter().map(|w| w.weight).sum::<f64>();
        if !sum.is_finite() || sum <= 1e-12 {
            return Err("Invalid FBX skin weight sum".into());
        }
        let mut ji = [0; 4];
        let mut we = [0.; 4];
        for (i, w) in w.iter().enumerate() {
            ji[i] = w.cluster_index as u16;
            we[i] = w.weight / sum;
        }
        joints.push(ji);
        weights.push(we);
        positions.push(vector(mesh.vertex_position[i]));
    }
    Ok(Some(Skin {
        names,
        parents,
        world,
        inverse_bind: bind,
        positions,
        joints,
        weights,
    }))
}
pub(crate) fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Asset, String> {
    crate::book_io::check_cancel(cancel)?;
    let bytes = crate::book_io::read_local(path, (32 * 1024 * 1024).min(budget / 8))?;
    let progress = |_: &ufbx::Progress| {
        if cancel.load(Ordering::Relaxed) {
            ufbx::ProgressResult::Cancel
        } else {
            ufbx::ProgressResult::Continue
        }
    };
    let allocator = || ufbx::AllocatorOpts {
        memory_limit: (budget / 4).min(256 * 1024 * 1024),
        allocation_limit: 1_000_000,
        ..Default::default()
    };
    let scene = ufbx::load_memory(
        &bytes,
        ufbx::LoadOpts {
            file_format: ufbx::FileFormat::Fbx,
            no_format_from_content: true,
            no_format_from_extension: true,
            load_external_files: false,
            ignore_missing_external_files: false,
            evaluate_caches: false,
            retain_dom: true,
            strict: true,
            node_depth_limit: 64,
            generate_missing_normals: false,
            index_error_handling: ufbx::IndexErrorHandling::AbortLoading,
            unicode_error_handling: ufbx::UnicodeErrorHandling::AbortLoading,
            target_axes: ufbx::axes_right_handed_y_up(),
            target_unit_meters: 1.0,
            temp_allocator: allocator(),
            result_allocator: allocator(),
            progress_cb: ufbx::ProgressCb::Ref(&progress),
            progress_interval_hint: 16384,
            ..Default::default()
        },
    )
    .map_err(|e| format!("FBX parse failed: {e:?}"))?;
    crate::book_io::check_cancel(cancel)?;
    if scene.metadata.file_format != ufbx::FileFormat::Fbx
        || !(7100..=7700).contains(&scene.metadata.version)
    {
        return Err("FBX supports ASCII/binary versions7100..7700".into());
    }
    if scene.nodes.len() > 512 || scene.meshes.len() > 128 {
        return Err("FBX node512 / mesh128 limit".into());
    }
    let mut a = Asset::default();
    for w in &scene.metadata.warnings {
        a.warning(format!("FBX parser: {}", w.description.as_ref()));
    }
    let mut source = Source {
        format: "FBX".into(),
        version: format!(
            "{} {}",
            scene.metadata.version,
            if scene.metadata.ascii {
                "ASCII"
            } else {
                "binary"
            }
        ),
        file: path.display().to_string(),
        notices: vec![],
        usage_present: false,
    };
    if let Some(dom) = &scene.dom_root {
        terms(dom, "", &mut source, cancel)?;
    }
    if !scene.anim_stacks.is_empty() {
        a.warning(
            "FBX animations are not converted; the source default node/skin pose is frozen".into(),
        );
    }
    for node in &scene.nodes {
        crate::book_io::check_cancel(cancel)?;
        let Some(mesh) = &node.mesh else { continue };
        if a.models.len() >= 128
            || mesh.num_indices > 65536
            || mesh.num_triangles > 131072
            || mesh.num_indices == 0
        {
            return Err("FBX editable geometry limit".into());
        }
        if mesh.num_point_faces != 0 || mesh.num_line_faces != 0 || !mesh.cache_deformers.is_empty()
        {
            return Err("FBX points/lines/vertex caches require triangle conversion".into());
        }
        let count = mesh
            .blend_deformers
            .iter()
            .map(|d| d.channels.len())
            .sum::<usize>();
        if count > 32
            || mesh
                .num_indices
                .saturating_mul(128 + 24 * count)
                .saturating_mul(8)
                > budget
        {
            return Err("FBX morph count / memory reservation exceeded".into());
        }
        let mut s = Surface {
            source: "FBX frozen local import (+Y up, metres)".into(),
            ..Default::default()
        };
        let mut attrs = Attributes {
            skin: skin(mesh)?,
            ..Default::default()
        };
        materials(
            node,
            &mut s,
            &mut attrs,
            path.parent().ok_or("Missing FBX folder")?,
            &mut a,
            budget,
        )?;
        let mut transforms = vec![];
        let mut base = vec![];
        for (i, control) in mesh.vertex_indices.iter().enumerate() {
            if i % 256 == 0 {
                crate::book_io::check_cancel(cancel)?;
            }
            let m = if mesh.skin_deformers.is_empty() {
                node.geometry_to_world
            } else {
                ufbx::get_skin_vertex_matrix(
                    &mesh.skin_deformers[0],
                    *control as usize,
                    &node.geometry_to_world,
                )
            };
            let m = matrix(m);
            if !m.valid() {
                return Err("FBX singular geometry / skin transform".into());
            }
            base.push(m.map(vector(mesh.vertex_position[i])));
            transforms.push(m);
            if mesh.vertex_uv.exists {
                let v = mesh.vertex_uv[i];
                s.texcoords.push([v.x, v.y, 0.]);
            }
            if mesh.vertex_normal.exists {
                let n = vector(mesh.vertex_normal[i]);
                if dot(n, n) < 1e-24 {
                    return Err("FBX zero normal".into());
                }
                s.normals.push(normal(m, n));
            }
        }
        if attrs.materials.values().any(|m| m.image.is_some()) && s.texcoords.is_empty() {
            return Err("Textured FBX has no UV coordinates".into());
        }
        let mut keys = vec![];
        for deformer in &mesh.blend_deformers {
            for channel in &deformer.channels {
                if keys.len() >= 32
                    || channel.keyframes.len() != 1
                    || (channel.keyframes[0].target_weight - 1.).abs() > 1e-6
                {
                    return Err(
                        "FBX blend shapes require at most32 single-target keys (no in-betweens)"
                            .into(),
                    );
                }
                let shape = &channel.keyframes[0].shape;
                if shape.offset_vertices.len() > 65536 {
                    return Err("FBX shape offset limit65536".into());
                }
                let mut control_delta = BTreeMap::new();
                for (i, v) in shape.offset_vertices.iter().enumerate() {
                    if i % 256 == 0 {
                        crate::book_io::check_cancel(cancel)?;
                    }
                    if *v as usize >= mesh.num_vertices {
                        return Err("FBX shape index out of range".into());
                    }
                    let offset = shape
                        .position_offsets
                        .get(i)
                        .ok_or("Missing FBX shape position offset")?;
                    if control_delta.insert(*v, vector(*offset)).is_some() {
                        return Err("Duplicate FBX shape offset vertex".into());
                    }
                }
                if !shape.normal_offsets.is_empty() {
                    a.warning("FBX morph normal deltas are not retained; nonzero morphs use geometric face normals".into());
                }
                keys.push(crate::model_edit::ShapeKey {
                    name: name(
                        channel.element.name.as_ref(),
                        &format!("Target {}", keys.len()),
                    )?,
                    weight: channel.weight,
                    delta: mesh
                        .vertex_indices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            transforms[i].vector(control_delta.get(c).copied().unwrap_or([0.; 3]))
                        })
                        .collect(),
                });
            }
        }
        let vertices = base
            .iter()
            .enumerate()
            .map(|(i, p)| {
                keys.iter()
                    .fold(*p, |p, k| add(p, mul(k.delta[i], k.weight)))
            })
            .collect::<Vec<_>>();
        let mut edges = BTreeSet::new();
        let mut indices = vec![];
        for (fi, f) in mesh.faces.iter().enumerate() {
            crate::book_io::check_cancel(cancel)?;
            if f.num_indices < 3 || f.num_indices > 256 {
                return Err("FBX faces require3..256 corners".into());
            }
            let count = ufbx::triangulate_face_vec(&mut indices, mesh, *f) as usize;
            if count + 2 != f.num_indices as usize {
                return Err("FBX polygon triangulation failed".into());
            }
            let material = if mesh.face_material.is_empty() {
                0
            } else {
                mesh.face_material[fi]
            };
            if material as usize >= s.materials.len() {
                return Err("FBX material slot out of range".into());
            }
            for f in indices[..count * 3].as_chunks::<3>().0 {
                let mut v = [f[0], f[1], f[2]];
                if v.iter().all(|i| transforms[*i as usize].determinant() < 0.) {
                    v.swap(1, 2);
                }
                let [p, q, r] = v.map(|i| vertices[i as usize]);
                let n = cross(sub(q, p), sub(r, p));
                if dot(n, n) < 1e-24 {
                    return Err("FBX degenerate triangle".into());
                }
                for i in 0..3 {
                    edges.insert([
                        v[i].min(v[(i + 1) % 3]) as usize,
                        v[i].max(v[(i + 1) % 3]) as usize,
                    ]);
                }
                s.triangles.push(Triangle {
                    v,
                    t: (!s.texcoords.is_empty()).then_some(v),
                    n: (!s.normals.is_empty()).then_some(v),
                    material: material as u16,
                    smoothing: 0,
                });
            }
        }
        if !keys.is_empty() {
            let m = crate::model_edit::Morph {
                base,
                keys,
                normals: s.triangles.iter().map(|t| t.n).collect(),
            };
            if !m.valid() {
                return Err("Invalid FBX morph names, deltas or weights0..1".into());
            }
            if m.keys.iter().any(|k| k.weight != 0.) {
                for t in &mut s.triangles {
                    t.n = None;
                }
            }
            attrs.morph = Some(m);
        }
        s.imported = Some(attrs);
        let color = s.materials[0].rgba.map(|v| (v * 255.).round() as u8);
        a.models.push(Model {
            name: name(node.element.name.as_ref(), "FBX mesh")?,
            mesh: WorldMesh {
                vertices,
                edges: edges.into_iter().collect(),
                color,
            },
            surface: s,
        });
        if !a.valid() {
            return Err("FBX converted geometry / attributes exceed editable limits".into());
        }
        let used = a
            .models
            .iter()
            .map(|m| m.mesh.vertices.len() * 24 + m.mesh.edges.len() * 16 + m.surface.bytes())
            .sum::<usize>();
        if used.saturating_mul(8) > budget {
            return Err(
                "FBX geometry / material / skin / morph memory reservation exceeded".into(),
            );
        }
    }
    // Source notes are display-only; no URL or application-specific script is followed.
    crate::model_source::attach(&mut a, source)?;
    a.warning("FBX axes and units convert to right-handed +Y up / metres. Nodes become independent meshes; source hierarchy is represented by frozen transforms and skin joint parents, not animated scene nodes".into());
    if !a.valid() {
        return Err("FBX contains no usable mesh".into());
    }
    Ok(a)
}
