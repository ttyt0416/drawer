fn same_surface(a: &crate::model_mesh::Surface, b: &crate::model_mesh::Surface) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    if let Some(a) = &mut a.imported {
        a.source = None;
    }
    if let Some(b) = &mut b.imported {
        b.source = None;
    }
    a == b
}
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    model_import::*,
    model_library::{self as lib, Edit, Placement},
    world_math::Matrix,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value as J, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn write(path: &Path, data: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| e.to_string())
}
fn bytes(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| e.to_string())
}
pub fn fixture(root: &Path, skinned: bool) -> Result<(J, Vec<u8>, Vec<u8>), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut bin = vec![];
    let mut views = vec![];
    let mut accessors = vec![];
    let mut append = |data: Vec<u8>, component: usize, count: usize, kind: &str| {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let i = views.len();
        views.push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":data.len()}));
        bin.extend(data);
        let a = accessors.len();
        accessors.push(json!({"bufferView":i,"componentType":component,"count":count,"type":kind}));
        a
    };
    let floats = |v: &[f32]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
    let p = append(
        floats(&[-1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.]),
        5126,
        4,
        "VEC3",
    );
    let n = append(
        floats(&[0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.]),
        5126,
        4,
        "VEC3",
    );
    let uv = append(floats(&[0., 1., 1., 1., 1., 0., 0., 0.]), 5126, 4, "VEC2");
    let i = append(
        [0u16, 1, 2, 0, 2, 3]
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect(),
        5123,
        6,
        "SCALAR",
    );
    let mut attr = json!({"POSITION":p,"NORMAL":n,"TEXCOORD_0":uv});
    let mut skins = vec![];
    if skinned {
        let ji = append(
            vec![1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
            5121,
            4,
            "VEC4",
        );
        let wi = append(
            floats(&[
                1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
            ]),
            5126,
            4,
            "VEC4",
        );
        let mut ibm = vec![];
        for m in [Matrix::default(), Matrix::translation([-1., 0., 0.])] {
            for c in 0..4 {
                for r in 0..4 {
                    let n = if r == 3 {
                        if c == 3 { 1. } else { 0. }
                    } else {
                        m.0[r][c]
                    };
                    ibm.extend((n as f32).to_le_bytes());
                }
            }
        }
        let ib = append(ibm, 5126, 2, "MAT4");
        attr["JOINTS_0"] = json!(ji);
        attr["WEIGHTS_0"] = json!(wi);
        skins.push(json!({"joints":[1,2],"skeleton":1,"inverseBindMatrices":ib}));
    }
    let mut node = json!({"name":"Local plane","mesh":0});
    let nodes = if skinned {
        node["skin"] = json!(0);
        node["translation"] = json!([4, 0, 0]);
        vec![
            node,
            json!({"name":"Root joint","children":[2]}),
            json!({"name":"Tip joint","translation":[2,0,0]}),
        ]
    } else {
        vec![node]
    };
    use image::ImageEncoder;
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
            2,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    let roots = if skinned { vec![0, 1] } else { vec![0] };
    let doc = json!({"asset":{"version":"2.0","generator":"Drawer original test fixture"},"scene":0,"scenes":[{"nodes":roots}],"nodes":nodes,"meshes":[{"primitives":[{"attributes":attr,"indices":i,"material":0}]}],"buffers":[{"uri":"mesh.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors,"materials":[{"name":"Local diffuse","pbrMetallicRoughness":{"baseColorTexture":{"index":0},"baseColorFactor":[1,1,1,1]},"doubleSided":true}],"textures":[{"source":0,"sampler":0}],"samplers":[{"wrapS":33071,"wrapT":33071,"magFilter":9728,"minFilter":9728}],"images":[{"uri":"colors.png"}],"skins":skins});
    write(&root.join("mesh.bin"), &bin)?;
    write(&root.join("colors.png"), &png)?;
    write(&root.join("model.gltf"), serde_json::to_vec(&doc).unwrap())?;
    Ok((doc, bin, png))
}
fn save_json(root: &Path, j: &J) -> Result<PathBuf, String> {
    let path = root.join("model.gltf");
    write(&path, serde_json::to_vec(j).unwrap())?;
    Ok(path)
}
pub fn glb(root: &Path, j: &J, bin: &[u8]) -> Result<PathBuf, String> {
    let mut j = j.clone();
    j["buffers"][0].as_object_mut().unwrap().remove("uri");
    let mut data = serde_json::to_vec(&j).unwrap();
    while !data.len().is_multiple_of(4) {
        data.push(b' ');
    }
    let mut binary = bin.to_vec();
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend(((28 + data.len() + binary.len()) as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(0x4e4f534au32.to_le_bytes());
    out.extend(data);
    out.extend((binary.len() as u32).to_le_bytes());
    out.extend(0x004e4942u32.to_le_bytes());
    out.extend(binary);
    let p = root.join("model.glb");
    write(&p, out)?;
    Ok(p)
}
fn load_here(root: &Path) -> Result<crate::model_mesh::Asset, String> {
    load(&root.join("model.gltf"), B, &AtomicBool::new(false))
}
pub fn containers(root: &Path) -> Result<(), String> {
    let (j, bin, png) = fixture(root, false)?;
    let a = load_here(root)?;
    check(
        a.models.len() == 1
            && a.models[0].surface.triangles.len() == 2
            && a.models[0].surface.normals.len() == 4
            && a.models[0].surface.texcoords[0] == [0., 0., 0.],
        "glTF topology/normals/UV conversion differs",
    )?;
    let p = glb(root, &j, &bin)?;
    let b = load(&p, B, &AtomicBool::new(false))?;
    check(
        a.models[0].mesh == b.models[0].mesh
            && same_surface(&a.models[0].surface, &b.models[0].surface),
        "GLB and external glTF differ",
    )?;
    let mut embedded = j.clone();
    embedded["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        STANDARD.encode(&bin)
    ));
    embedded["images"][0]["uri"] =
        json!(format!("data:image/png;base64,{}", STANDARD.encode(&png)));
    save_json(root, &embedded)?;
    check(
        same_surface(&load_here(root)?.models[0].surface, &a.models[0].surface),
        "Embedded data URI differs",
    )?;
    let mut chunk = j.clone();
    chunk["images"][0] =
        json!({"bufferView":j["bufferViews"].as_array().unwrap().len(),"mimeType":"image/png"});
    let mut binary = bin;
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    chunk["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":png.len()}));
    binary.extend(png);
    chunk["buffers"][0]["byteLength"] = json!(binary.len());
    let p = glb(root, &chunk, &binary)?;
    check(
        same_surface(
            &load(&p, B, &AtomicBool::new(false))?.models[0].surface,
            &a.models[0].surface,
        ),
        "GLB buffer-view image differs",
    )?;
    let mut stride = j;
    let mut binary = bytes(&root.join("mesh.bin"))?;
    let offset = binary.len();
    for v in &a.models[0].mesh.vertices {
        for n in v {
            binary.extend((*n as f32).to_le_bytes());
        }
        binary.extend([0u8; 4]);
    }
    let vi = stride["bufferViews"].as_array().unwrap().len();
    stride["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":offset,"byteLength":64,"byteStride":16}));
    stride["accessors"][0]["bufferView"] = json!(vi);
    stride["buffers"][0]["byteLength"] = json!(binary.len());
    write(&root.join("mesh.bin"), binary)?;
    save_json(root, &stride)?;
    check(
        load_here(root)?.models[0].mesh == a.models[0].mesh,
        "Interleaved stride positions differ",
    )?;
    Ok(())
}
pub fn transforms_skin_warnings(root: &Path) -> Result<(), String> {
    let (mut j, _, _) = fixture(root, true)?;
    j["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_1"] = json!(999);
    j["nodes"][0]["customFlag"] = json!(true);
    j["extensionsUsed"] = json!(["VENDOR_optional"]);
    j["extensions"] = json!({"VENDOR_optional":{}});
    save_json(root, &j)?;
    let a = load_here(root)?;
    let m = &a.models[0];
    let skin = m.surface.imported.as_ref().unwrap().skin.as_ref().unwrap();
    check(
        m.mesh.vertices[0] == [0., -1., 0.]
            && skin.parents == vec![None, Some(0)]
            && skin.weights[0] == [1., 0., 0., 0.]
            && skin.positions[0] == [-1., -1., 0.],
        "LBS default pose/retained rig differs",
    )?;
    for word in [
        "WEIGHTS_1",
        "customFlag",
        "VENDOR_optional",
        "default linear-blend",
    ] {
        check(
            a.warnings.iter().any(|s| s.contains(word)),
            "Unsupported property/rig loss not listed",
        )?;
    }
    let mut d = crate::model_library_verify::blank()?;
    let before = d.snapshot();
    let p = lib::prepare(
        &d,
        Edit::Import {
            path: root.join("model.gltf"),
            placement: Placement {
                fit: None,
                scale: [0.01; 3],
                angles: [90., 0., 0.],
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?;
    check(
        !p.warnings.is_empty() && d.snapshot() == before,
        "Import preview changed source",
    )?;
    p.commit(&mut d)?;
    let ext = crate::model_library::bounds(&d.scene, &std::collections::BTreeSet::from([1]))?;
    check(
        (ext.1[0] - ext.0[0] - 0.02).abs() < 1e-9 && (ext.1[2] - ext.0[2] - 0.02).abs() < 1e-9,
        "Axis rotation/unit scale conversion differs",
    )?;
    let done = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Import not one Undo")?;
    d.redo();
    check(d.snapshot() == done, "Import Redo differs")?;
    let saved = root.join("rig.drawer");
    crate::files::save(&mut d, &saved).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&saved, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == done,
        "Native113 lost retained rig",
    )?;
    let (mut j, _, _) = fixture(root, false)?;
    j["nodes"][0]["translation"] = json!([2, 3, 0]);
    j["nodes"][0]["scale"] = json!([-2, 1, 1]);
    j["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"children":[0],"translation":[0,1,0]}));
    j["scenes"][0]["nodes"] = json!([1]);
    save_json(root, &j)?;
    let a = load_here(root)?;
    check(
        a.models[0].mesh.vertices[0] == [4., 3., 0.]
            && a.models[0].surface.triangles[0].v == [0, 2, 1],
        "Parent transform/mirrored winding differs",
    )?;
    Ok(())
}
pub fn rejection(root: &Path) -> Result<(), String> {
    let (j, _, _) = fixture(root, false)?;
    for uri in [
        "https://example.invalid/model.bin",
        "../mesh.bin",
        "%2e%2e/mesh.bin",
        "C:/mesh.bin",
        "%2Foutside.bin",
        "a%5c..%5cmesh.bin",
        "mesh.bin?query=1",
    ] {
        let mut v = j.clone();
        v["buffers"][0]["uri"] = json!(uri);
        save_json(root, &v)?;
        check(load_here(root).is_err(), "Unsafe URI accepted")?;
    }
    let mut cases = vec![];
    let mut v = j.clone();
    v["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    cases.push(v);
    let mut v = j.clone();
    v["bufferViews"][0]["byteLength"] = json!(999999);
    cases.push(v);
    let mut v = j.clone();
    v["bufferViews"][0]["byteStride"] = json!(5);
    cases.push(v);
    let mut v = j.clone();
    v["accessors"][0]["count"] = json!(999999999u64);
    cases.push(v);
    let mut v = j.clone();
    v["accessors"][0]["sparse"] = json!({"count":1});
    cases.push(v);
    let mut v = j.clone();
    v["nodes"][0]["children"] = json!([0]);
    cases.push(v);
    let mut v = j.clone();
    v["nodes"][0]["scale"] = json!([0, 1, 1]);
    cases.push(v);
    let mut v = j.clone();
    v["textures"][0]["source"] = json!(100);
    cases.push(v);
    let mut v = j.clone();
    v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
    cases.push(v);
    let mut v = j.clone();
    v["meshes"][0]["primitives"][0]["mode"] = json!(1);
    cases.push(v);
    for v in cases {
        save_json(root, &v)?;
        check(
            load_here(root).is_err(),
            "Malformed/unsupported glTF accepted",
        )?;
    }
    save_json(root, &j)?;
    let p = glb(root, &j, &bytes(&root.join("mesh.bin"))?)?;
    let original = bytes(&p)?;
    for length in [0, 12, 19, original.len() - 1] {
        write(&p, &original[..length])?;
        check(
            load(&p, B, &AtomicBool::new(false)).is_err(),
            "Truncated GLB accepted",
        )?;
    }
    check(
        load(&root.join("model.gltf"), 1, &AtomicBool::new(false)).is_err()
            && load(&root.join("model.gltf"), B, &AtomicBool::new(true)).is_err(),
        "Budget/cancel ignored",
    )?;
    let (mut j, _, _) = fixture(root, true)?;
    j["skins"][0]["joints"] = json!([99]);
    save_json(root, &j)?;
    check(load_here(root).is_err(), "Unknown skin joint accepted")?;
    let (mut j, _, _) = fixture(root, true)?;
    let i = j["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"]
        .as_u64()
        .unwrap() as usize;
    j["accessors"][i]["count"] = json!(3);
    save_json(root, &j)?;
    check(load_here(root).is_err(), "Mismatched skin weights accepted")?;
    Ok(())
}
pub fn document_textures(root: &Path) -> Result<(), String> {
    fixture(root, false)?;
    let source = bytes(&root.join("model.gltf"))?;
    let mut d = crate::model_library_verify::blank()?;
    lib::prepare(
        &d,
        Edit::Import {
            path: root.join("model.gltf"),
            placement: Placement {
                fit: None,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let mut scene = d.scene.clone();
    scene.world3d.lighting = Some(crate::lighting::Settings {
        style: crate::lighting::Style::Unlit,
        ..Default::default()
    });
    scene.world3d.lens.as_mut().unwrap().projection = crate::world3d::Projection::Orthographic;
    d.edit_scene(scene)?;
    let p = |d: &Document, world| {
        let point = d
            .scene
            .world3d
            .lens
            .unwrap()
            .project(d.scene.camera.as_ref().unwrap(), world)
            .unwrap();
        crate::render::Renderer::new(d).pixel(point)
    };
    check(
        p(&d, [-0.5, 0.5, 0.]) == [255, 0, 0, 255]
            && p(&d, [0.5, 0.5, 0.]) == [0, 255, 0, 255]
            && p(&d, [-0.5, -0.5, 0.]) == [0, 0, 255, 255],
        "Diffuse texture orientation/render pixels differ",
    )?;
    let before = d.snapshot();
    let path = root.join("imported.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    std::fs::remove_file(root.join("colors.png")).map_err(|e| e.to_string())?;
    std::fs::remove_file(root.join("mesh.bin")).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == before && p(&reopened, [-0.5, 0.5, 0.]) == [255, 0, 0, 255],
        "Native texture depends on source files",
    )?;
    let mut old = bytes(&path)?;
    crate::save_generation::verify::legacy_version(&mut old, 112);
    write(&root.join("old.drawer"), old)?;
    check(
        crate::files::open(&root.join("old.drawer"), B).is_err(),
        "native112 accepted imported attributes",
    )?;
    lib::prepare(
        &d,
        Edit::Duplicate {
            ids: vec![1],
            offset: [4., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let original = d.scene.world3d.surfaces[&1].clone();
    let mut scene = d.scene.clone();
    scene
        .world3d
        .surfaces
        .get_mut(&2)
        .unwrap()
        .imported
        .as_mut()
        .unwrap()
        .materials
        .get_mut(&0)
        .unwrap()
        .image = None;
    d.edit_scene(scene)?;
    check(
        d.scene.world3d.surfaces[&1] == original,
        "Duplicate texture aliases source",
    )?;
    check(
        bytes(&root.join("model.gltf"))? == source,
        "Source glTF was modified",
    )?;
    // A MASK hole must not hide geometry behind it or contribute to a contour pass.
    let mut scene = d.scene.clone();
    let m = scene
        .world3d
        .surfaces
        .get_mut(&1)
        .unwrap()
        .imported
        .as_mut()
        .unwrap()
        .materials
        .get_mut(&0)
        .unwrap();
    let mut image = m.image.clone().unwrap();
    let mut pixels = image.decode()?;
    pixels[3] = 0;
    use image::ImageEncoder;
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 2, 2, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    image.png = STANDARD.encode(png);
    m.image = Some(image);
    m.alpha = Alpha::Mask;
    d.edit_scene(scene)?;
    check(
        p(&d, [-0.5, 0.5, 0.])[3] == 0 && p(&d, [0.5, 0.5, 0.])[3] == 255,
        "Texture alpha mask is ignored",
    )?;
    Ok(())
}
pub fn obj_textures(root: &Path) -> Result<(), String> {
    let (_, _, png) = fixture(root, false)?;
    write(&root.join("map.png"), png)?;
    write(
        &root.join("map.mtl"),
        "newmtl Paint\nKd 1 1 1\nmap_Kd map.png\n",
    )?;
    let path = root.join("map.obj");
    write(
        &path,
        "mtllib map.mtl\nv -1 -1 0\nv 1 -1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0.5 1\nusemtl Paint\nf 1/1 2/2 3/3\n",
    )?;
    let before = bytes(&path)?;
    let a = load(&path, B, &AtomicBool::new(false))?;
    check(
        a.models[0]
            .surface
            .imported
            .as_ref()
            .is_some_and(|a| a.materials.values().any(|m| m.image.is_some()))
            && bytes(&path)? == before,
        "OBJ/MTL diffuse map not frozen",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("model-import-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for (name, run) in [
            ("containers", containers as fn(&Path) -> Result<(), String>),
            ("rig", transforms_skin_warnings),
            ("reject", rejection),
            ("document", document_textures),
            ("obj", obj_textures),
        ] {
            let p = dir.join(name);
            std::fs::create_dir(&p).map_err(|e| e.to_string())?;
            run(&p)?;
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn containers_images_and_interleaved_accessors() {
        let t = crate::test_support::TempDir::new();
        super::containers(&t.0).unwrap();
    }
    #[test]
    fn transforms_rigs_and_loss_notices() {
        let t = crate::test_support::TempDir::new();
        super::transforms_skin_warnings(&t.0).unwrap();
    }
    #[test]
    fn malformed_unsafe_cancel_and_budget() {
        let t = crate::test_support::TempDir::new();
        super::rejection(&t.0).unwrap();
    }
    #[test]
    fn native113_textures_alpha_and_source_independence() {
        let t = crate::test_support::TempDir::new();
        super::document_textures(&t.0).unwrap();
    }
    #[test]
    fn obj_mtl_diffuse_image() {
        let t = crate::test_support::TempDir::new();
        super::obj_textures(&t.0).unwrap();
    }
}
