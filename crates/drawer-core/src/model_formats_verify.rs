//! Original, generated format fixtures; no third-party model assets.
use crate::{document::DEFAULT_DOCUMENT_BUDGET as B, model_import::load};
use serde_json::{Value as J, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
fn write(p: &Path, b: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(p, b).map_err(|e| e.to_string())
}
#[derive(Clone)]
enum P {
    S(String),
    I(i32),
    L(i64),
    D(f64),
    Ints(Vec<i32>),
    Doubles(Vec<f64>),
}
impl From<&str> for P {
    fn from(v: &str) -> Self {
        Self::S(v.into())
    }
}
#[derive(Clone)]
struct N {
    key: &'static str,
    props: Vec<P>,
    children: Vec<N>,
}
fn n(key: &'static str, props: Vec<P>, children: Vec<N>) -> N {
    N {
        key,
        props,
        children,
    }
}
fn scalar(key: &'static str, value: i32) -> N {
    n(key, vec![P::I(value)], vec![])
}
fn string(key: &'static str, value: &str) -> N {
    n(key, vec![value.into()], vec![])
}
fn array(key: &'static str, v: &[f64]) -> N {
    n(key, vec![P::Doubles(v.to_vec())], vec![])
}
fn ints(key: &'static str, v: &[i32]) -> N {
    n(key, vec![P::Ints(v.to_vec())], vec![])
}
fn object(key: &'static str, id: i64, name: &str, kind: &str, children: Vec<N>) -> N {
    n(key, vec![P::L(id), name.into(), kind.into()], children)
}
fn prop(key: &str, kind: &str, values: &[f64]) -> N {
    let mut p = vec![key.into(), kind.into(), "".into(), "A".into()];
    p.extend(values.iter().map(|v| P::D(*v)));
    n("P", p, vec![])
}
fn tree() -> Vec<N> {
    let identity = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    let mut global = vec![
        prop("UpAxis", "int", &[1.]),
        prop("UpAxisSign", "int", &[1.]),
        prop("FrontAxis", "int", &[2.]),
        prop("FrontAxisSign", "int", &[-1.]),
        prop("CoordAxis", "int", &[0.]),
        prop("CoordAxisSign", "int", &[1.]),
        prop("UnitScaleFactor", "double", &[100.]),
    ];
    // Integer globals use integer property payloads, as exported by FBX writers.
    for p in global.iter_mut().take(6) {
        if let Some(P::D(v)) = p.props.pop() {
            p.props.push(P::I(v as i32));
        }
    }
    vec![
        n(
            "FBXHeaderExtension",
            vec![],
            vec![
                scalar("FBXHeaderVersion", 1003),
                scalar("FBXVersion", 7400),
                n(
                    "SceneInfo",
                    vec!["SceneInfo::GlobalInfo".into(), "UserData".into()],
                    vec![string(
                        "Copyright",
                        "Original local fixture — retain this text\nNo external asset.",
                    )],
                ),
            ],
        ),
        n(
            "GlobalSettings",
            vec![],
            vec![scalar("Version", 1000), n("Properties70", vec![], global)],
        ),
        n(
            "Objects",
            vec![],
            vec![
                object(
                    "Geometry",
                    1000,
                    "Geometry::Triangle",
                    "Mesh",
                    vec![
                        array("Vertices", &[0., 0., 0., 1., 0., 0., 0., 1., 0.]),
                        ints("PolygonVertexIndex", &[0, 1, -3]),
                        n(
                            "LayerElementNormal",
                            vec![P::I(0)],
                            vec![
                                scalar("Version", 101),
                                string("Name", ""),
                                string("MappingInformationType", "ByPolygonVertex"),
                                string("ReferenceInformationType", "Direct"),
                                array("Normals", &[0., 0., 1., 0., 0., 1., 0., 0., 1.]),
                            ],
                        ),
                        n(
                            "LayerElementUV",
                            vec![P::I(0)],
                            vec![
                                scalar("Version", 101),
                                string("Name", "default"),
                                string("MappingInformationType", "ByPolygonVertex"),
                                string("ReferenceInformationType", "Direct"),
                                array("UV", &[0., 0., 1., 0., 0., 1.]),
                            ],
                        ),
                        n(
                            "LayerElementMaterial",
                            vec![P::I(0)],
                            vec![
                                scalar("Version", 101),
                                string("Name", ""),
                                string("MappingInformationType", "AllSame"),
                                string("ReferenceInformationType", "IndexToDirect"),
                                ints("Materials", &[0]),
                            ],
                        ),
                        n(
                            "Layer",
                            vec![P::I(0)],
                            vec![
                                scalar("Version", 100),
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![
                                        string("Type", "LayerElementNormal"),
                                        scalar("TypedIndex", 0),
                                    ],
                                ),
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![string("Type", "LayerElementUV"), scalar("TypedIndex", 0)],
                                ),
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![
                                        string("Type", "LayerElementMaterial"),
                                        scalar("TypedIndex", 0),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
                object(
                    "Model",
                    2000,
                    "Model::Triangle",
                    "Mesh",
                    vec![scalar("Version", 232), n("Properties70", vec![], vec![])],
                ),
                object(
                    "Material",
                    3000,
                    "Material::Blue",
                    "",
                    vec![
                        scalar("Version", 102),
                        string("ShadingModel", "lambert"),
                        scalar("MultiLayer", 0),
                        n(
                            "Properties70",
                            vec![],
                            vec![
                                prop("DiffuseColor", "Color", &[0.2, 0.4, 0.8]),
                                prop("TransparencyFactor", "Number", &[0.5]),
                            ],
                        ),
                    ],
                ),
                object(
                    "Model",
                    5000,
                    "Model::Root",
                    "LimbNode",
                    vec![scalar("Version", 232), n("Properties70", vec![], vec![])],
                ),
                object(
                    "Deformer",
                    4000,
                    "Deformer::Skin",
                    "Skin",
                    vec![scalar("Version", 101)],
                ),
                object(
                    "Deformer",
                    4001,
                    "SubDeformer::Root",
                    "Cluster",
                    vec![
                        scalar("Version", 100),
                        ints("Indexes", &[0, 1, 2]),
                        array("Weights", &[1., 1., 1.]),
                        array("Transform", &identity),
                        array("TransformLink", &identity),
                    ],
                ),
                object(
                    "Geometry",
                    6000,
                    "Geometry::Smile",
                    "Shape",
                    vec![
                        scalar("Version", 100),
                        ints("Indexes", &[2]),
                        array("Vertices", &[0., 0., 1.]),
                    ],
                ),
                object(
                    "Deformer",
                    6001,
                    "Deformer::Shape",
                    "BlendShape",
                    vec![scalar("Version", 100)],
                ),
                object(
                    "Deformer",
                    6002,
                    "SubDeformer::Smile",
                    "BlendShapeChannel",
                    vec![
                        scalar("Version", 100),
                        n("DeformPercent", vec![P::D(50.)], vec![]),
                        array("FullWeights", &[100.]),
                    ],
                ),
            ],
        ),
        n(
            "Connections",
            vec![],
            [
                (1000, 2000),
                (2000, 0),
                (3000, 2000),
                (5000, 0),
                (4000, 1000),
                (4001, 4000),
                (5000, 4001),
                (6001, 1000),
                (6002, 6001),
                (6000, 6002),
            ]
            .into_iter()
            .map(|(a, b)| n("C", vec!["OO".into(), P::L(a), P::L(b)], vec![]))
            .collect(),
        ),
    ]
}
fn ascii(n: &N, out: &mut String) {
    out.push_str(n.key);
    out.push_str(": ");
    for (i, p) in n.props.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match p {
            P::S(s) => {
                out.push('"');
                out.push_str(
                    &s.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', " "),
                );
                out.push('"');
            }
            P::I(v) => out.push_str(&v.to_string()),
            P::L(v) => out.push_str(&v.to_string()),
            P::D(v) => out.push_str(&v.to_string()),
            P::Ints(v) => out.push_str(&format!(
                "*{} {{ a: {} }}",
                v.len(),
                v.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
            )),
            P::Doubles(v) => out.push_str(&format!(
                "*{} {{ a: {} }}",
                v.len(),
                v.iter().map(f64::to_string).collect::<Vec<_>>().join(",")
            )),
        }
    }
    if !n.children.is_empty() {
        out.push_str(" {\n");
        for c in &n.children {
            ascii(c, out);
        }
        out.push('}');
    }
    out.push('\n');
}
fn binary(n: &N, out: &mut Vec<u8>, wide: bool) {
    let begin = out.len();
    let width = if wide { 8 } else { 4 };
    let header = width * 3 + 1;
    out.resize(begin + header, 0);
    out[begin + width * 3] = n.key.len() as u8;
    out.extend(n.key.as_bytes());
    let props = out.len();
    for p in &n.props {
        match p {
            P::S(s) => {
                let s = if let Some((kind, name)) = s.split_once("::") {
                    format!("{name}\0\u{1}{kind}")
                } else {
                    s.clone()
                };
                out.push(b'S');
                out.extend((s.len() as u32).to_le_bytes());
                out.extend(s.as_bytes());
            }
            P::I(v) => {
                out.push(b'I');
                out.extend(v.to_le_bytes());
            }
            P::L(v) => {
                out.push(b'L');
                out.extend(v.to_le_bytes());
            }
            P::D(v) => {
                out.push(b'D');
                out.extend(v.to_le_bytes());
            }
            P::Ints(v) => {
                out.push(b'i');
                out.extend((v.len() as u32).to_le_bytes());
                out.extend(0u32.to_le_bytes());
                out.extend((v.len() as u32 * 4).to_le_bytes());
                for x in v {
                    out.extend(x.to_le_bytes());
                }
            }
            P::Doubles(v) => {
                out.push(b'd');
                out.extend((v.len() as u32).to_le_bytes());
                out.extend(0u32.to_le_bytes());
                out.extend((v.len() as u32 * 8).to_le_bytes());
                for x in v {
                    out.extend(x.to_le_bytes());
                }
            }
        }
    }
    let size = out.len() - props;
    for c in &n.children {
        binary(c, out, wide);
    }
    if !n.children.is_empty() {
        out.resize(out.len() + header, 0);
    }
    let end = out.len() as u64;
    for (i, value) in [end, n.props.len() as u64, size as u64].iter().enumerate() {
        out[begin + i * width..begin + (i + 1) * width]
            .copy_from_slice(&value.to_le_bytes()[..width]);
    }
}
fn fbx_nodes(path: &Path, nodes: &[N], binary_file: bool, version: u32) -> Result<(), String> {
    if binary_file {
        let mut bytes = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
        bytes.extend(version.to_le_bytes());
        for n in nodes {
            binary(n, &mut bytes, version >= 7500);
        }
        bytes.resize(bytes.len() + if version >= 7500 { 25 } else { 13 }, 0);
        write(path, bytes)
    } else {
        let mut text = format!(
            "; FBX {}.{}.0 project file\n",
            version / 1000,
            version / 100 % 10
        );
        for n in nodes {
            ascii(n, &mut text);
        }
        write(path, text)
    }
}
pub fn fbx_fixture(path: &Path, binary_file: bool) -> Result<(), String> {
    fbx_nodes(path, &tree(), binary_file, 7400)
}
pub fn fixture(root: &Path, ext: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join(format!("source.{ext}"));
    match ext {
        "fbx" => fbx_fixture(&path, false)?,
        "stl" => write(
            &path,
            "solid own triangle\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid own triangle\n",
        )?,
        "obj" => {
            write(
                &path,
                "# Original local fixture; usage is described here.\nmtllib source.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl Blue\nf 1/1 2/2 3/3\n",
            )?;
            write(
                &root.join("source.mtl"),
                "# Copyright local fixture\nnewmtl Blue\nKd 0.2 0.4 0.8\nd 0.5\n",
            )?;
        }
        "gltf" | "glb" | "vrm" => {
            let (mut j, mut bin, _) = crate::model_import_verify::fixture(root, true)?;
            j["asset"]["copyright"] =
                json!("Original local fixture — exact usage text\nSecond line.");
            let offset = bin.len();
            for p in [[0f32; 3], [0.; 3], [0., 0., 1.], [0.; 3]] {
                for v in p {
                    bin.extend(v.to_le_bytes());
                }
            }
            let view = j["bufferViews"].as_array().unwrap().len();
            let acc = j["accessors"].as_array().unwrap().len();
            j["bufferViews"]
                .as_array_mut()
                .unwrap()
                .push(json!({"buffer":0,"byteOffset":offset,"byteLength":48}));
            j["accessors"]
                .as_array_mut()
                .unwrap()
                .push(json!({"bufferView":view,"componentType":5126,"count":4,"type":"VEC3"}));
            j["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":acc}]);
            j["meshes"][0]["weights"] = json!([0.5]);
            j["meshes"][0]["extras"] = json!({"targetNames":["Smile"]});
            j["buffers"][0]["byteLength"] = json!(bin.len());
            j["materials"][0]["alphaMode"] = json!("BLEND");
            j["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([1., 1., 1., 0.5]);
            if ext == "vrm" {
                j["extensionsUsed"] = json!(["VRMC_vrm"]);
                j["extensionsRequired"] = json!(["VRMC_vrm"]);
                j["extensions"] = json!({"VRMC_vrm":{"specVersion":"1.0","meta":{"name":"Own fixture","authors":["Drawer local tests"],"licenseUrl":"https://vrm.dev/licenses/1.0/","otherLicenseUrl":"https://example.invalid/do-not-fetch","allowRedistribution":false},"humanoid":{"humanBones":{}},"expressions":{}}});
            }
            if ext == "gltf" {
                write(&root.join("mesh.bin"), &bin)?;
                write(&path, serde_json::to_vec(&j).unwrap())?;
            } else {
                let generated = crate::model_import_verify::glb(root, &j, &bin)?;
                std::fs::rename(generated, &path).map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("Unknown fixture format".into()),
    }
    Ok(path)
}
pub fn formats(root: &Path) -> Result<(), String> {
    for ext in crate::model_source::EXTENSIONS {
        let dir = root.join(ext);
        let p = fixture(&dir, ext)?;
        let original = std::fs::read(&p).map_err(|e| e.to_string())?;
        let a = load(&p, B, &AtomicBool::new(false)).map_err(|e| format!("{ext}: {e}"))?;
        check(a.valid() && a.models.len() == 1, "Format sample invalid")?;
        let attrs = a.models[0]
            .surface
            .imported
            .as_ref()
            .ok_or("Missing import attributes")?;
        check(
            attrs.source.as_ref().is_some_and(|s| {
                s.file == p.canonicalize().unwrap().display().to_string() && !s.notices.is_empty()
            }),
            "Missing raw source / usage notes",
        )?;
        if matches!(*ext, "fbx" | "gltf" | "glb" | "vrm") {
            check(
                attrs.skin.is_some()
                    && attrs
                        .morph
                        .as_ref()
                        .is_some_and(|m| m.keys.len() == 1 && m.keys[0].weight == 0.5),
                "Rig / morph conversion missing",
            )?;
            check(
                a.models[0].mesh.vertices.iter().any(|v| v[2].abs() == 0.5),
                "Default morph not applied",
            )?;
        }
        if *ext != "stl" {
            check(
                a.models[0].surface.materials[a.models[0].surface.triangles[0].material as usize]
                    .rgba[3]
                    == 0.5,
                "Material opacity lost",
            )?;
        }
        for (budget, cancel) in [(1, false), (B, true)] {
            check(
                load(&p, budget, &AtomicBool::new(cancel)).is_err(),
                "Budget / cancellation ignored",
            )?;
        }
        let bad = dir.join(format!("broken.{ext}"));
        write(&bad, b"invalid format bytes")?;
        check(
            load(&bad, B, &AtomicBool::new(false)).is_err(),
            "Broken format silently accepted",
        )?;
        let wrong = dir.join(if *ext == "stl" {
            "wrong.glb"
        } else {
            "wrong.stl"
        });
        write(&wrong, &original)?;
        check(
            load(&wrong, B, &AtomicBool::new(false)).is_err(),
            "Extension / header mismatch accepted",
        )?;
        check(
            std::fs::read(&p).map_err(|e| e.to_string())? == original,
            "Import changed source bytes",
        )?;
    }
    Ok(())
}
pub fn fbx_stl(root: &Path) -> Result<(), String> {
    let p = root.join("binary.fbx");
    fbx_fixture(&p, true)?;
    let a = load(&p, B, &AtomicBool::new(false))?;
    check(
        a.models[0]
            .surface
            .imported
            .as_ref()
            .unwrap()
            .skin
            .is_some()
            && a.models[0].mesh.vertices.iter().any(|v| v[2].abs() == 0.5),
        "Binary FBX skin / blend shape differs",
    )?;
    let mut bytes = vec![0u8; 80];
    bytes.extend(1u32.to_le_bytes());
    for v in [0f32, 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0.] {
        bytes.extend(v.to_le_bytes());
    }
    bytes.extend(0u16.to_le_bytes());
    let stl = root.join("binary.stl");
    write(&stl, &bytes)?;
    let a = load(&stl, B, &AtomicBool::new(false))?;
    check(
        a.models[0].mesh.vertices.len() == 3
            && a.models[0]
                .surface
                .imported
                .as_ref()
                .unwrap()
                .source
                .as_ref()
                .unwrap()
                .notices
                .is_empty(),
        "Binary STL / absent terms differs",
    )?;
    bytes[80..84].copy_from_slice(&2u32.to_le_bytes());
    write(&stl, &bytes)?;
    check(
        load(&stl, B, &AtomicBool::new(false)).is_err(),
        "STL truncated facet count accepted",
    )?;
    bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    write(&stl, &bytes)?;
    check(
        load(&stl, B, &AtomicBool::new(false)).is_err(),
        "STL huge declared count accepted",
    )?;
    Ok(())
}
pub fn rejection(root: &Path) -> Result<(), String> {
    let dir = root.join("reject");
    let p = fixture(&dir, "gltf")?;
    let j: J = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let mut cases = vec![];
    let mut v = j.clone();
    v["asset"]["copyright"] = json!({"unreadable":true});
    cases.push(v);
    let mut v = j.clone();
    v["asset"]["copyright"] = json!("bad\0usage");
    cases.push(v);
    let mut v = j.clone();
    v["asset"]["copyright"] = json!("x".repeat(65537));
    cases.push(v);
    let mut v = j.clone();
    v["extensionsRequired"] = json!(["VENDOR_unknown"]);
    cases.push(v);
    let mut v = j.clone();
    v["meshes"][0]["weights"] = json!([2.]);
    cases.push(v);
    let mut v = j.clone();
    v["meshes"][0]["primitives"][0]["targets"] = json!(vec![json!({}); 33]);
    cases.push(v);
    for v in cases {
        write(&p, serde_json::to_vec(&v).unwrap())?;
        check(
            load(&p, B, &AtomicBool::new(false)).is_err(),
            "Unreadable terms / mandatory extension / morph limit accepted",
        )?;
    }
    write(&p, serde_json::to_vec(&j).unwrap())?;
    std::fs::remove_file(dir.join("colors.png")).map_err(|e| e.to_string())?;
    check(
        load(&p, B, &AtomicBool::new(false)).is_err(),
        "Missing glTF texture silently accepted",
    )?;
    let p = root.join("unsupported.usdz");
    write(&p, b"unknown")?;
    check(
        load(&p, B, &AtomicBool::new(false)).is_err(),
        "Unknown extension accepted",
    )?;
    for ext in crate::model_source::EXTENSIONS {
        let p = root.join(format!("huge.{ext}"));
        let f = std::fs::File::create(&p).map_err(|e| e.to_string())?;
        f.set_len(33 * 1024 * 1024).map_err(|e| e.to_string())?;
        check(
            load(&p, B, &AtomicBool::new(false)).is_err(),
            "Oversized source accepted",
        )?;
        std::fs::remove_file(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn native(root: &Path) -> Result<(), String> {
    let p = fixture(&root.join("native-source"), "vrm")?;
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    let mut d = crate::model_library_verify::blank()?;
    let before = d.snapshot();
    crate::model_library::prepare(
        &d,
        crate::model_library::Edit::Import {
            path: p.clone(),
            placement: crate::model_library::Placement {
                fit: None,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.world3d.editing.morphs[&1].keys[0].weight == 0.5,
        "Imported key not editable",
    )?;
    let snap = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Imported object not one Undo")?;
    d.redo();
    check(d.snapshot() == snap, "Imported Redo lost metadata")?;
    let file = root.join("format.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == snap,
        "Native121 depends on source / loses usage text or keys",
    )?;
    let mut old = std::fs::read(&file).map_err(|e| e.to_string())?;
    old[8..12].copy_from_slice(&120u32.to_le_bytes());
    write(&root.join("old.drawer"), old)?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "Native120 disguised source metadata accepted",
    )?;
    write(&p, &bytes)?;
    // Use the public editing operation to rebuild geometry, not only metadata.
    crate::model_edit::prepare(
        &d,
        crate::model_edit::Edit::Weight {
            model: 1,
            key: 0,
            value: 0.,
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    check(
        d.scene.worlds[0].vertices != snap.scene.worlds[0].vertices,
        "Imported shape key has no geometric effect",
    )?;
    Ok(())
}
pub fn versions_textures(root: &Path) -> Result<(), String> {
    for version in (7100..=7700).step_by(100) {
        for binary in [false, true] {
            let mut nodes = tree();
            nodes[0].children[1] = scalar("FBXVersion", version as i32);
            let p = root.join(format!("v{version}-{binary}.fbx"));
            fbx_nodes(&p, &nodes, binary, version)?;
            let a = load(&p, B, &AtomicBool::new(false))
                .map_err(|e| format!("FBX {version}/{binary}: {e}"))?;
            let m = &a.models[0];
            let tri = &m.surface.triangles[0];
            let [p, q, r] = tri.v.map(|i| m.mesh.vertices[i as usize]);
            let geometric = crate::world_math::cross(
                crate::world_math::sub(q, p),
                crate::world_math::sub(r, p),
            );
            check(
                crate::world_math::dot(geometric, m.surface.normals[0]) > 0.,
                "FBX axis reflection left winding / normals inconsistent",
            )?;
        }
    }
    let p = root.join("texture.fbx");
    let mut nodes = tree();
    let objects = nodes.iter_mut().find(|n| n.key == "Objects").unwrap();
    objects.children.push(object(
        "Texture",
        7000,
        "Texture::Local",
        "",
        vec![
            string("Type", "TextureVideoClip"),
            scalar("Version", 202),
            string("FileName", "diffuse.png"),
            string("RelativeFilename", "diffuse.png"),
            n("ModelUVTranslation", vec![P::D(0.), P::D(0.)], vec![]),
            n("ModelUVScaling", vec![P::D(1.), P::D(1.)], vec![]),
        ],
    ));
    nodes
        .iter_mut()
        .find(|n| n.key == "Connections")
        .unwrap()
        .children
        .push(n(
            "C",
            vec!["OP".into(), P::L(7000), P::L(3000), "DiffuseColor".into()],
            vec![],
        ));
    crate::surface_edit_verify::image(&root.join("diffuse.png"), &[255, 0, 0, 128], 1, 1)?;
    fbx_nodes(&p, &nodes, false, 7400)?;
    let a = load(&p, B, &AtomicBool::new(false))?;
    let image = a.models[0].surface.imported.as_ref().unwrap().materials[&0]
        .image
        .as_ref()
        .ok_or("FBX diffuse map missing")?;
    check(
        image.decode()? == [255, 0, 0, 128],
        "FBX texture RGBA snapshot differs",
    )?;
    std::fs::remove_file(root.join("diffuse.png")).map_err(|e| e.to_string())?;
    let a = load(&p, B, &AtomicBool::new(false))?;
    check(
        a.warnings
            .iter()
            .any(|w| w.contains("Missing local FBX diffuse texture")),
        "Missing FBX texture silently accepted",
    )?;
    let bad = std::fs::read_to_string(&p)
        .map_err(|e| e.to_string())?
        .replace("diffuse.png", "../escape.png");
    write(&p, bad)?;
    check(
        load(&p, B, &AtomicBool::new(false)).is_err(),
        "FBX external traversal accepted",
    )?;
    Ok(())
}
pub fn usage_vrm0(root: &Path) -> Result<(), String> {
    let dir = root.join("vrm0");
    let p = fixture(&dir, "gltf")?;
    let mut j: J = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    j["asset"].as_object_mut().unwrap().remove("copyright");
    j["extras"] = json!({"generatorNote":"plain source note"});
    write(&p, serde_json::to_vec(&j).unwrap())?;
    let a = load(&p, B, &AtomicBool::new(false))?;
    let s = a.models[0]
        .surface
        .imported
        .as_ref()
        .unwrap()
        .source
        .as_ref()
        .unwrap();
    check(
        !s.usage_present && !s.notices.is_empty(),
        "Unclassified metadata mistaken for usage terms",
    )?;
    j["extensions"] = json!({"VRM":{"specVersion":"0.0","meta":{"title":"Own VRM0","author":"Local tests","licenseName":"Other","otherLicenseUrl":"https://example.invalid/local-only","allowedUserName":"OnlyAuthor","violentUssageName":"Disallow"},"humanoid":{"humanBones":[]}}});
    j["extensionsUsed"] = json!(["VRM"]);
    let bin = std::fs::read(dir.join("mesh.bin")).map_err(|e| e.to_string())?;
    let glb = crate::model_import_verify::glb(&dir, &j, &bin)?;
    let vrm = dir.join("model.vrm");
    std::fs::rename(glb, &vrm).map_err(|e| e.to_string())?;
    let a = load(&vrm, B, &AtomicBool::new(false))?;
    let s = a.models[0]
        .surface
        .imported
        .as_ref()
        .unwrap()
        .source
        .as_ref()
        .unwrap();
    check(
        s.usage_present && s.notices.iter().any(|n| n.text.contains("OnlyAuthor")),
        "VRM0 raw usage metadata lost",
    )?;
    j["extensions"]["VRM"]["meta"]["licenseName"] = json!({"invalid":true});
    let glb = crate::model_import_verify::glb(&dir, &j, &bin)?;
    std::fs::rename(glb, &vrm).map_err(|e| e.to_string())?;
    check(
        load(&vrm, B, &AtomicBool::new(false)).is_err(),
        "Unreadable VRM usage field accepted",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("model-formats");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        formats(&dir)?;
        fbx_stl(&dir)?;
        rejection(&dir)?;
        native(&dir)?;
        versions_textures(&dir)?;
        usage_vrm0(&dir)
    })();
    result.and(std::fs::remove_dir_all(&dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn all_six_local_formats_alpha_rigs_morphs_source_unchanged() {
        let t = crate::test_support::TempDir::new();
        super::formats(&t.0).unwrap();
    }
    #[test]
    fn binary_fbx_stl_headers_counts_and_absent_terms() {
        let t = crate::test_support::TempDir::new();
        super::fbx_stl(&t.0).unwrap();
    }
    #[test]
    fn unsafe_missing_unreadable_unknown_required_and_large_files() {
        let t = crate::test_support::TempDir::new();
        super::rejection(&t.0).unwrap();
    }
    #[test]
    fn fbx_all_versions_ascii_binary_local_rgba_and_missing_texture() {
        let t = crate::test_support::TempDir::new();
        super::versions_textures(&t.0).unwrap();
    }
    #[test]
    fn usage_absence_unclassified_notes_vrm0_raw_terms_and_bad_types() {
        let t = crate::test_support::TempDir::new();
        super::usage_vrm0(&t.0).unwrap();
    }
    #[test]
    fn native121_raw_terms_editable_keys_and_single_undo() {
        let t = crate::test_support::TempDir::new();
        super::native(&t.0).unwrap();
    }
}
